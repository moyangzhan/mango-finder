use crate::entities::FileMetaEmbedding;
use crate::repositories::RepositoryError;
use crate::repositories::embedding_collection_repo::{self, CollectionTables};
use crate::structs::sparse_vector::SparseVector;
use crate::utils::vector_util;
use rusqlite::{Result, Row, named_params};

/// File metadata embeddings of the ACTIVE vector collection (see file_content_embedding_repo).
///
/// 激活向量库的文件元数据向量(见 file_content_embedding_repo)。

fn active_tables() -> (CollectionTables, usize) {
    let identity = embedding_collection_repo::active_identity();
    let dim = identity.dim;
    (embedding_collection_repo::collection_tables(&identity), dim)
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * std::mem::size_of::<f32>());
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub fn insert(
    file_metadata_embedding: &FileMetaEmbedding,
) -> Result<Option<FileMetaEmbedding>, RepositoryError> {
    let (tables, dim) = active_tables();
    let mut conn = crate::db_init_manager::open_db()?;

    let tx: rusqlite::Transaction<'_> = conn.transaction()?;

    let file_metadata_embedding = {
        let mut stmt = tx.prepare(&format!(
            "insert into {vec}(embedding) values (:embedding)",
            vec = tables.meta_vec
        ))?;
        // L2-normalize at the collection dim (cosine metric; also keeps Matryoshka
        // truncation correct if a backend dim ever exceeds the collection dim)
        let embedding = vector_util::finalize_metadata_embedding(
            file_metadata_embedding.embedding.clone(),
            dim,
        );
        if embedding.len() != dim {
            return Err(RepositoryError::InvalidInput(format!(
                "metadata embedding dim {} does not match active collection dim {dim}",
                embedding.len()
            )));
        }
        let embedding_bytes = f32_bytes(&embedding);
        let _ = stmt.insert(named_params! {
            ":embedding": &embedding_bytes,
        })?;

        let mut stmt = tx.prepare(&format!(
            "insert into {data}(file_id, sparse_weights) values (:file_id, :sparse_weights)",
            data = tables.meta_data
        ))?;
        let _ = stmt.insert(named_params! {
            ":file_id": &file_metadata_embedding.file_id,
            ":sparse_weights": &file_metadata_embedding.sparse_vec.to_blob(),
        })?;

        // where rowid = ?1 will cause error: no such column: rowid
        let mut query_stmt = tx.prepare(&format!(
            "select v.id, d.file_id, -0.1 as distance, d.sparse_weights from {vec} v join {data} d on v.id = d.id where d.file_id = ?1 order by v.id desc limit 1",
            vec = tables.meta_vec,
            data = tables.meta_data
        ))?;
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

/// Metadata hybrid search over the active collection.
/// Empty query sparse vector = dense-only mode: the unconditional 0.01 sparse
/// filter (which would drop every result) and the sparse score component are skipped.
///
/// 激活库的元数据混合检索。查询 sparse 为空即纯 dense 模式:跳过原本
/// 无条件的 0.01 sparse 过滤(否则会滤掉全部结果)与 sparse 评分分量。
pub fn hybrid_search(
    dense_embedding: &[f32],
    query_sparse_indices: &[u32],
    query_sparse_values: &[f32],
    min_score: usize, // 1 - 100
) -> Result<Vec<FileMetaEmbedding>, RepositoryError> {
    let (tables, dim) = active_tables();
    let dense_only = query_sparse_indices.is_empty();
    let q_dense = vector_util::finalize_metadata_embedding(dense_embedding.to_vec(), dim);
    let embedding_bytes = f32_bytes(&q_dense);

    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "select v.id, d.file_id, v.distance, d.sparse_weights from ( select id, distance from {vec} where embedding match :embedding order by distance limit 50 ) v join {data} d on v.id = d.id order by v.distance asc",
        vec = tables.meta_vec,
        data = tables.meta_data
    ))?;
    let rows = stmt.query_map(named_params! {":embedding": embedding_bytes}, |row| {
        let sparse_blob: Vec<u8> = row.get("sparse_weights").unwrap_or(vec![]); // Default to empty vector if sparse_weights is not available
        let sparse_vec = SparseVector::from_blob(&sparse_blob);
        let distance: f32 = row.get("distance").unwrap_or(1.0); // Default to 1.0 if distance is not available
        let sparse_score = sparse_vec.dot_product(query_sparse_indices, query_sparse_values);
        let score = if dense_only {
            vector_util::calculate_dense_only_score(distance)
        } else {
            vector_util::calculate_metadata_score(distance, sparse_score)
        };
        Ok(FileMetaEmbedding {
            id: row.get("id")?,
            file_id: row.get("file_id")?,
            embedding: Vec::new(),
            sparse_vec,

            distance,
            sparse_score,
            score,
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
                if !dense_only && fme.sparse_score < 0.01 {
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

/// Delete a file's metadata rows in the ACTIVE collection (idempotent re-embed).
pub fn delete_by_file_id(file_id: i64) -> Result<usize, RepositoryError> {
    if file_id < 1 {
        return Ok(0);
    }
    let (tables, _) = active_tables();
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    tx.execute(
        &format!(
            "delete from {vec} where id in (select id from {data} where file_id = :file_id)",
            vec = tables.meta_vec,
            data = tables.meta_data
        ),
        &[(":file_id", &file_id)],
    )?;
    let affected = tx.execute(
        &format!("delete from {data} where file_id = :file_id", data = tables.meta_data),
        &[(":file_id", &file_id)],
    )?;
    tx.commit()?;
    Ok(affected)
}

fn build_file_metadata_embedding(row: &Row<'_>) -> Result<FileMetaEmbedding, RepositoryError> {
    let distance: f32 = row.get("distance").unwrap_or(1.0); // Default to 1.0 if distance is not available
    return Ok(FileMetaEmbedding {
        id: row.get("id")?,
        file_id: row.get("file_id")?,
        embedding: Vec::new(),
        sparse_vec: {
            let sparse_blob: Vec<u8> = row.get("sparse_weights")?;
            SparseVector::from_blob(&sparse_blob)
        },
        distance: distance,
        sparse_score: 0.0,
        score: 0,
    });
}
