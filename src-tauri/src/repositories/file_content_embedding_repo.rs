use crate::entities::FileContentEmbedding;
use crate::global::SHORT_QUERY_LEN;
use crate::repositories::RepositoryError;
use crate::repositories::embedding_collection_repo::{self, CollectionTables};
use crate::structs::image_similarity_candidate::ImageSimilarityCandidate;
use crate::structs::sparse_vector::SparseVector;
use crate::utils::vector_util::{calculate_content_score, calculate_dense_only_score};
use rusqlite::{Result, Row, named_params};

/// Content chunk embeddings of the ACTIVE vector collection.
/// Table names are derived from the active embedding identity (config table is
/// the single source of truth, read directly on every call - the local static
/// tables are just one possible derived value).
///
/// 激活向量库的内容分块向量。表名由激活嵌入身份派生(config 表为单一事实源,
/// 每次调用直读;本地静态表只是派生函数的一个返回值)。

fn active_tables() -> CollectionTables {
    embedding_collection_repo::collection_tables(&embedding_collection_repo::active_identity())
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * std::mem::size_of::<f32>());
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

pub fn insert(
    file_content_embedding: &FileContentEmbedding,
) -> Result<Option<FileContentEmbedding>, RepositoryError> {
    let tables = active_tables();
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;

    let file_content_embedding = {
        let mut stmt = tx.prepare(&format!(
            "insert into {vec}(embedding) values (:embedding)",
            vec = tables.content_vec
        ))?;
        let embedding_bytes = f32_bytes(&file_content_embedding.embedding);
        let last_insert_rowid = stmt.insert(named_params! {
            ":embedding": &embedding_bytes,
        })?;
        log::debug!(
            "file_content_embedding_repo.insert() last_insert_rowid: {}",
            last_insert_rowid
        );

        // file_content_data
        let mut stmt = tx.prepare(&format!(
            "insert into {data}(id, file_id, sparse_weights, chunk_index, chunk_text) values (:id, :file_id, :sparse_weights, :chunk_index, :chunk_text)",
            data = tables.content_data
        ))?;
        let _ = stmt.insert(named_params! {
            ":id": last_insert_rowid,
            ":file_id": &file_content_embedding.file_id,
            ":chunk_index": &file_content_embedding.chunk_index,
            ":chunk_text": &file_content_embedding.chunk_text,
            ":sparse_weights": &file_content_embedding.sparse_vec.to_blob(),
        })?;

        //where rowid = ?1 will cause error: no such column: rowid ???
        let mut query_stmt = tx.prepare(&format!(
            "select v.id, d.file_id, d.chunk_index, d.chunk_text, -0.1 as distance, d.sparse_weights from {vec} v join {data} d on v.id = d.id where d.file_id = ?1 order by v.id desc limit 1",
            vec = tables.content_vec,
            data = tables.content_data
        ))?;
        let file_content_embedding = query_stmt
            .query_row([&file_content_embedding.file_id], |row| {
                Ok(Some(build_file_content_embedding(row)?))
            })
            .unwrap_or_else(|e| {
                log::debug!("file_content_embedding_repo.insert() Error: {}", e);
                None
            });

        file_content_embedding
    };
    tx.commit()?;
    Ok(file_content_embedding)
}

pub fn update(file_content_embedding: &FileContentEmbedding) -> Result<usize, RepositoryError> {
    let tables = active_tables();
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "update {vec} set embedding = :embedding where id = :id",
        vec = tables.content_vec
    ))?;
    let embedding_bytes = f32_bytes(&file_content_embedding.embedding);
    let affected = stmt.execute(named_params! {
        ":id": &file_content_embedding.id,
        ":embedding": embedding_bytes,
    })?;
    log::debug!("update content vec affected: {:?}", affected);
    Ok(affected)
}

/// dense + sparse hybrid search over the active collection.
/// An empty query sparse vector switches to dense-only mode (remote backends
/// have no sparse output): the lexical noise filter and sparse score component
/// are skipped entirely.
///
/// 激活库上的 dense + sparse 混合检索。查询 sparse 为空时进入纯 dense 模式
/// (远程后端无 sparse 输出):完全跳过词法噪声过滤与 sparse 评分分量。
pub fn hybrid_search(
    dense_embedding: &[f32],
    query_sparse_indices: &[u32],
    query_sparse_values: &[f32],
    min_score: usize, // 1 - 100
) -> Result<Vec<FileContentEmbedding>, RepositoryError> {
    let tables = active_tables();
    let dense_only = query_sparse_indices.is_empty();
    let is_short_query = !dense_only && query_sparse_indices.len() <= SHORT_QUERY_LEN;
    let sparse_threshold = if dense_only {
        0.0
    } else if is_short_query {
        0.01
    } else {
        0.0
    };

    let embedding_bytes = f32_bytes(dense_embedding);
    log::debug!(
        "content hybrid_search, dense_only: {}, query_sparse_indices: {:?}, min_score: {}",
        dense_only,
        query_sparse_indices,
        min_score
    );
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "select v.id, d.file_id, d.chunk_index, d.chunk_text, v.distance, d.sparse_weights from ( select id, distance from {vec} where embedding match :embedding order by distance limit 50 ) v join {data} d on v.id = d.id order by v.distance asc",
        vec = tables.content_vec,
        data = tables.content_data
    ))?;
    let rows = stmt.query_map(named_params! {":embedding": embedding_bytes}, |row| {
        let sparse_blob: Vec<u8> = row.get("sparse_weights").unwrap_or(vec![]); // Default to empty vector if sparse_weights is not available
        let sparse_vec = SparseVector::from_blob(&sparse_blob);
        let distance: f32 = row.get("distance").unwrap_or(1.0); // Default to 1.0 if distance is not available
        let sparse_score = sparse_vec.dot_product(query_sparse_indices, query_sparse_values);
        let score = if dense_only {
            calculate_dense_only_score(distance)
        } else {
            calculate_content_score(distance, sparse_score, is_short_query)
        };
        Ok(FileContentEmbedding {
            id: row.get("id")?,
            file_id: row.get("file_id")?,
            chunk_index: row.get("chunk_index")?,
            chunk_text: row.get("chunk_text")?,
            embedding: Vec::new(),
            sparse_vec,

            distance: distance,
            sparse_score,
            score,
        })
    })?;
    log::debug!("Raw search results count: {}", rows.size_hint().0);
    let filtered_result = rows
        .filter_map(|res| match res {
            Ok(fce) => {
                log::debug!(
                    "Raw Search Result - file_id: {}, chunk_id: {}, distance: {}, sparse_score: {}, score: {}",
                    fce.file_id, fce.chunk_index, fce.distance, fce.sparse_score, fce.score
                );
                // 1. If Sparse score is too low (keywords don't match), treat as noise and filter out
                // Key to fixing Chinese-to-English short-query noise recall
                if fce.sparse_score < sparse_threshold {
                    return None;
                }

                // 2. If Dense distance is too large, filter out (semantically unrelated), regardless of Sparse score
                if min_score > 0 && fce.score < min_score {
                    return None;
                }

                Some(fce)
            }
            Err(e) => {
                log::error!("Error retrieving file embedding: {}", e);
                None
            }
        })
        .collect::<Vec<_>>();
    return Ok(filtered_result);
}

pub fn list_chunks_by_ids(ids: &Vec<u32>) -> Result<Vec<String>, RepositoryError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let tables = active_tables();
    let conn = crate::db_init_manager::open_db()?;
    let ids_str = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = conn.prepare(&format!(
        "select chunk_text from {} where id in ({}) order by chunk_index asc",
        tables.content_data, ids_str
    ))?;
    let rows = stmt.query_map((), |row| {
        let chunk_text: String = row.get("chunk_text")?;
        Ok(chunk_text)
    })?;
    let mut segments: Vec<String> = Vec::new();
    for row_result in rows {
        match row_result {
            Ok(chunk_text) => segments.push(chunk_text),
            Err(e) => log::error!("Error retrieving chunk text: {}", e),
        }
    }
    Ok(segments)
}

pub fn count() -> Result<i64, RepositoryError> {
    let tables = active_tables();
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!("SELECT COUNT(*) FROM {}", tables.content_vec))?;
    let count: i64 = stmt.query_row([], |row| row.get(0))?;
    return Ok(count);
}

/// Get all embeddings for a specific file (for document similarity)
pub fn list_by_file_id(file_id: i64) -> Result<Vec<FileContentEmbedding>, RepositoryError> {
    let tables = active_tables();
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT v.id, d.file_id, d.chunk_index, d.chunk_text, v.embedding, d.sparse_weights
         FROM {vec} v
         JOIN {data} d ON v.id = d.id
         WHERE d.file_id = ?1
         ORDER BY d.chunk_index ASC",
        vec = tables.content_vec,
        data = tables.content_data
    ))?;

    let rows = stmt.query_map([file_id], |row| {
        let embedding_bytes: Vec<u8> = row.get::<_, Vec<u8>>(4)?;
        let sparse_blob: Vec<u8> = row.get::<_, Vec<u8>>(5)?;
        Ok(FileContentEmbedding {
            id: row.get(0)?,
            file_id: row.get(1)?,
            chunk_index: row.get(2)?,
            chunk_text: row.get(3)?,
            embedding: bytes_to_f32(&embedding_bytes),
            sparse_vec: SparseVector::from_blob(&sparse_blob),
            distance: 0.0,
            sparse_score: 0.0,
            score: 0,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }

    Ok(results)
}

/// Delete a file's content rows in the ACTIVE collection only (used by the
/// fast re-embed channel, which must not touch the source collection).
pub fn delete_active_rows_by_file(file_id: i64) -> Result<usize, RepositoryError> {
    if file_id < 1 {
        return Ok(0);
    }
    let tables = active_tables();
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    tx.execute(
        &format!(
            "delete from {vec} where id in (select id from {data} where file_id = :file_id)",
            vec = tables.content_vec,
            data = tables.content_data
        ),
        &[(":file_id", &file_id)],
    )?;
    let affected = tx.execute(
        &format!("delete from {data} where file_id = :file_id", data = tables.content_data),
        &[(":file_id", &file_id)],
    )?;
    tx.commit()?;
    Ok(affected)
}

/// List raw chunk rows (id, file_id, chunk_index, chunk_text) of the active
/// collection - used by the fast re-embed rebuild channel.
///
/// 列出激活库的原始分块行,供「仅重嵌入」快速通道使用。
pub struct RawChunkRow {
    pub file_id: i64,
    pub chunk_index: i64,
    pub chunk_text: String,
}

pub fn list_raw_chunks(
    tables: &CollectionTables,
    min_id: i64,
    limit: i64,
) -> Result<Vec<RawChunkRow>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "select id, file_id, chunk_index, chunk_text from {} where id > ?1 order by id asc limit ?2",
        tables.content_data
    ))?;
    let rows = stmt.query_map([min_id, limit], |row| {
        Ok(RawChunkRow {
            file_id: row.get(1)?,
            chunk_index: row.get(2)?,
            chunk_text: row.get(3)?,
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// All chunk texts of one file in a SPECIFIC collection (source collection of
/// the fast re-embed channel), ordered by chunk_index.
pub fn list_chunk_texts_by_file(
    tables: &CollectionTables,
    file_id: i64,
) -> Result<Vec<(i64, String)>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "select chunk_index, chunk_text from {} where file_id = ?1 order by chunk_index asc",
        tables.content_data
    ))?;
    let rows = stmt.query_map([file_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Use sqlite-vec ANN search to find semantically similar image candidates
/// 使用 sqlite-vec ANN 搜索查找语义相似的图片候选
pub fn find_image_similarity_candidates(
    source_file_id: i64,
    limit: usize,
) -> Result<Vec<ImageSimilarityCandidate>, RepositoryError> {
    let tables = active_tables();
    let conn = crate::db_init_manager::open_db()?;

    // Get source embedding
    let source_embedding: Option<Vec<u8>> = conn
        .query_row(
            &format!(
                "SELECT embedding FROM {} WHERE id IN (
            SELECT id FROM {} WHERE file_id = ?1
        ) ORDER BY id LIMIT 1",
                tables.content_vec, tables.content_data
            ),
            [source_file_id],
            |row| row.get(0),
        )
        .ok();

    match source_embedding {
        Some(emb) => find_image_candidates_by_embedding_bytes(&emb, limit),
        None => Ok(Vec::new()),
    }
}

/// Same as `find_image_similarity_candidates`, but the query vector is provided directly instead of being loaded from a stored source file embedding.
/// 与 `find_image_similarity_candidates` 相同，区别在于查询向量直接入参，而不是从已存储的源文件嵌入中加载（供未索引外部文件使用）。
pub fn find_image_similarity_candidates_by_vector(
    dense_embedding: &[f32],
    limit: usize,
) -> Result<Vec<ImageSimilarityCandidate>, RepositoryError> {
    let embedding_bytes = f32_bytes(dense_embedding);
    find_image_candidates_by_embedding_bytes(&embedding_bytes, limit)
}

/// Single implementation of the image-candidate ANN query (previously duplicated between the by-file-id and by-vector variants — any tuning of the SQL or score formula must stay in one place so indexed-file and external-file similarity behave identically).
/// 图片候选 ANN 查询的唯一实现(此前在 by-file-id 与 by-vector 两个变体间重复 —— SQL 或评分公式的任何调整只落一处,保证已索引文件与外部文件的相似行为一致)。
fn find_image_candidates_by_embedding_bytes(
    embedding_bytes: &[u8],
    limit: usize,
) -> Result<Vec<ImageSimilarityCandidate>, RepositoryError> {
    let tables = active_tables();
    let conn = crate::db_init_manager::open_db()?;

    // Use ANN search to find similar embeddings, then join with file_info for image_hash
    let sql = format!(
        r#"
        SELECT
            d.file_id,
            v.distance,
            f.image_hash
        FROM (
            SELECT id, distance
            FROM {vec}
            WHERE embedding MATCH :embedding
            ORDER BY distance
            LIMIT {}
        ) v
        JOIN {data} d ON v.id = d.id
        JOIN file_info f ON d.file_id = f.id
        WHERE f.category = 2
        GROUP BY d.file_id
    "#,
        limit * 2,
        vec = tables.content_vec,
        data = tables.content_data
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::named_params! {
            ":embedding": embedding_bytes
        },
        |row| {
            let file_id: i64 = row.get(0)?;
            let distance: f32 = row.get(1)?;
            let image_hash: Option<Vec<u8>> = row.get(2)?;

            // Convert distance to score (distance 0 = 100, distance 1 = 0)
            let semantic_score = ((1.0 - distance) * 100.0).max(0.0).min(100.0) as usize;

            Ok(ImageSimilarityCandidate {
                file_id,
                semantic_score,
                image_hash,
            })
        },
    )?;

    let mut candidates = Vec::new();
    for row in rows {
        if let Ok(candidate) = row {
            candidates.push(candidate);
        }
    }

    Ok(candidates)
}

fn build_file_content_embedding(row: &Row<'_>) -> Result<FileContentEmbedding, RepositoryError> {
    let sparse_blob: Vec<u8> = row.get("sparse_weights")?;
    return Ok(FileContentEmbedding {
        id: row.get("id")?,
        file_id: row.get("file_id")?,
        embedding: Vec::new(),
        chunk_index: row.get("chunk_index")?,
        chunk_text: row.get("chunk_text")?,
        distance: row.get("distance")?,
        sparse_vec: SparseVector::from_blob(&sparse_blob),
        sparse_score: 0.0,
        score: 0,
    });
}
