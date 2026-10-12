use crate::entities::FileMetaEmbedding;
use crate::repositories::RepositoryError;
use crate::structs::sparse_vector::SparseVector;
use crate::utils::vector_util;
use rusqlite::{Result, Row, named_params};

pub fn insert(
    file_metadata_embedding: &FileMetaEmbedding,
) -> Result<Option<FileMetaEmbedding>, RepositoryError> {
    let mut conn = crate::db_init_manager::open_db()?;

    let tx: rusqlite::Transaction<'_> = conn.transaction()?;

    let file_metadata_embedding = {
        let mut stmt =
            tx.prepare("insert into file_metadata_vec(embedding) values (:embedding)")?;
        let embedding = vector_util::finalize_metadata_embedding(
            file_metadata_embedding.embedding.to_vec(),
            256,
        );
        let embedding_bytes = unsafe {
            std::slice::from_raw_parts(
                embedding.as_ptr() as *const u8,
                embedding.len() * std::mem::size_of::<f32>(),
            )
        };
        let _ = stmt.insert(named_params! {
            ":embedding": &embedding_bytes,
        })?;

        let mut stmt = tx.prepare("insert into file_metadata_data(file_id,sparse_weights) values (:file_id,:sparse_weights)")?;
        let _ = stmt.insert(named_params! {
            ":file_id": &file_metadata_embedding.file_id,
            ":sparse_weights": &file_metadata_embedding.sparse_vec.to_blob(),
        })?;

        // where rowid = ?1 will cause error: no such column: rowid
        let mut query_stmt = tx.prepare(
        "select v.id, d.file_id, -0.1 as distance, d.sparse_weights from file_metadata_vec v join file_metadata_data d on v.id = d.id where d.file_id = ?1 order by v.id desc limit 1",
    )?;
        let file_metadata_embedding = query_stmt
            .query_row([&file_metadata_embedding.file_id], |row| {
                Ok(Some(build_file_metadata_embedding(row)?))
            })
            .unwrap_or_else(|e| {
                log::debug!("file_metadata_embedding_repo.insert() Error: {}", e);
                None
            });

        file_metadata_embedding
    };
    tx.commit()?;
    Ok(file_metadata_embedding)
}

pub fn update(file_metadata_embedding: &FileMetaEmbedding) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("update file_metadata_vec set embedding=:embedding where id = :id")?;
    let embedding_bytes = unsafe {
        std::slice::from_raw_parts(
            file_metadata_embedding.embedding.as_ptr() as *const u8,
            file_metadata_embedding.embedding.len() * std::mem::size_of::<f32>(),
        )
    };
    let affected = stmt.execute(named_params! {
        ":id": &file_metadata_embedding.id,
        ":embedding": embedding_bytes,
    })?;
    log::debug!("update file_metadata_vec affected: {:?}", affected);
    Ok(affected)
}

pub fn hybrid_search(
    dense_embedding: &[f32],
    query_sparse_indices: &[u32],
    query_sparse_values: &[f32],
    min_score: usize, // 1 - 100
) -> Result<Vec<FileMetaEmbedding>, RepositoryError> {
    let q_dense_256 = vector_util::finalize_metadata_embedding(dense_embedding.to_vec(), 256);
    let embedding_bytes = unsafe {
        std::slice::from_raw_parts(
            q_dense_256.as_ptr() as *const u8,
            q_dense_256.len() * std::mem::size_of::<f32>(),
        )
    };

    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare( "select v.id, d.file_id, v.distance, d.sparse_weights from ( select id, distance from file_metadata_vec where embedding match :embedding order by distance limit 50 ) v join file_metadata_data d on v.id = d.id order by v.distance asc", )?;
    let rows = stmt.query_map(named_params! {":embedding": embedding_bytes}, |row| {
        let embedding_bytes: Vec<u8> = row.get("embedding").unwrap_or(vec![0; 256 * 4]); // Default to zero vector if embedding is not available
        let embedding: [f32; 256] = unsafe {
            let ptr = embedding_bytes.as_ptr() as *const f32;
            std::ptr::read(ptr as *const [f32; 256])
        };
        let sparse_blob: Vec<u8> = row.get("sparse_weights").unwrap_or(vec![]); // Default to empty vector if sparse_weights is not available
        let sparse_vec = SparseVector::from_blob(&sparse_blob);
        let distance: f32 = row.get("distance").unwrap_or(1.0); // Default to 1.0 if distance is not available
        let sparse_score = sparse_vec.dot_product(query_sparse_indices, query_sparse_values);
        Ok(FileMetaEmbedding {
            id: row.get("id")?,
            file_id: row.get("file_id")?,
            embedding,
            sparse_vec,

            distance,
            sparse_score,
            score: vector_util::calculate_metadata_score(distance, sparse_score),
        })
    })?;
    let filtered_result = rows
        .into_iter()
        .filter_map(|res| match res {
            Ok(fme) => {
                log::debug!(
                    "meta search file_id: {}, distance: {}, sparse_score: {}, score: {}",
                    fme.file_id,
                    fme.distance,
                    fme.sparse_score,
                    fme.score
                );
                if fme.sparse_score < 0.01 {
                    return None;
                }
                if min_score > 0 && fme.score < min_score {
                    return None;
                }
                Some(fme)
            }
            Err(e) => {
                log::error!("Error retrieving file embedding: {}", e);
                None
            }
        })
        .collect::<Vec<FileMetaEmbedding>>();
    return Ok(filtered_result);
}

pub fn delete_by_file_id(file_id: i64) -> Result<usize, RepositoryError> {
    if file_id < 1 {
        return Ok(0);
    }
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_metadata_vec where id in (select id from file_metadata_data where file_id = :file_id)")?;
    let _ = stmt.execute(named_params! {":file_id": file_id})?;
    let mut stmt = conn.prepare("delete from file_metadata_data where file_id = :file_id")?;
    let affected = stmt.execute(named_params! {":file_id": file_id})?;
    Ok(affected)
}

pub fn delete_by_file_prefix_path(pre_path: &str) -> Result<usize, RepositoryError> {
    if pre_path.is_empty() {
        return Ok(0);
    }
    let pattern = if pre_path.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", pre_path)
    } else {
        format!("{}{}%", pre_path, std::path::MAIN_SEPARATOR)
    };
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    let affected = {
        let mut stmt_vec = tx.prepare( "delete from file_metadata_vec where id in ( 
        select d.id from file_metadata_data d join file_info f on d.file_id = f.id where f.path like :prefix_path 
        )", )?;
        stmt_vec.execute(named_params! {":prefix_path": pattern})?;

        let mut stmt_data = tx.prepare(
            "delete from file_metadata_data where file_id in (
                select id from file_info where path like :prefix_path
            )",
        )?;
        stmt_data.execute(named_params! {":prefix_path": pattern})?
    };
    tx.commit()?;
    Ok(affected)
}

pub fn clear() -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_metadata_vec")?;
    let affected = stmt.execute([])?;
    let mut stmt = conn.prepare("delete from file_metadata_data")?;
    let affected_data = stmt.execute([])?;
    Ok(affected + affected_data)
}

fn build_file_metadata_embedding(row: &Row<'_>) -> Result<FileMetaEmbedding, RepositoryError> {
    let embedding_bytes: Vec<u8> = row.get("embedding").unwrap_or(vec![0; 256 * 4]); // Default to zero vector if embedding is not available
    let embedding: [f32; 256] = unsafe {
        let ptr = embedding_bytes.as_ptr() as *const f32;
        std::ptr::read(ptr as *const [f32; 256])
    };
    let distance: f32 = row.get("distance").unwrap_or(1.0); // Default to 1.0 if distance is not available
    return Ok(FileMetaEmbedding {
        id: row.get("id")?,
        file_id: row.get("file_id")?,
        embedding,
        sparse_vec: {
            let sparse_blob: Vec<u8> = row.get("sparse_weights")?;
            SparseVector::from_blob(&sparse_blob)
        },
        distance: distance,
        sparse_score: 0.0,
        score: 0,
    });
}
