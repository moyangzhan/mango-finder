use crate::global::{
    CONFIG_NAME_ACTIVE_EMBEDDING, CONFIG_NAME_INDEXED_EMBEDDING, CONFIG_NAME_SERVICE_UNREACHABLE,
};
use crate::repositories::RepositoryError;
use crate::structs::embedding_identity::{
    EmbeddingIdentity, LOCAL_DIM, LOCAL_MODEL, LOCAL_PLATFORM,
};
use rusqlite::{Connection, Row, Transaction};
use rusqlite::OptionalExtension;
use serde::Serialize;

/// Physical table names of one vector collection.
/// The local backend keeps the legacy static tables in place (upgrade = zero rebuild);
/// every remote/self-hosted identity derives its own table set from the identity.
///
/// 单个向量库的物理表名。
/// 本地后端沿用既有静态表(升级零重建);远程/自托管身份各自按身份派生表集。
#[derive(Debug, Clone)]
pub struct CollectionTables {
    pub content_vec: String,
    pub content_data: String,
    pub meta_vec: String,
    pub meta_data: String,
    /// Prefix used for the file_id lookup indexes (names must be unique per table)
    pub index_prefix: String,
}

impl CollectionTables {
    /// The legacy static tables, used by the local identity.
    pub fn local_static() -> Self {
        CollectionTables {
            content_vec: "file_content_vec".to_string(),
            content_data: "file_content_data".to_string(),
            meta_vec: "file_metadata_vec".to_string(),
            meta_data: "file_metadata_data".to_string(),
            index_prefix: "fcd".to_string(),
        }
    }
}

/// Slugify a model/platform name into the [a-z0-9_] charset allowed for table names.
/// 将模型/平台名转为表名允许的 [a-z0-9_] 字符集。
fn slugify(input: &str) -> String {
    let mut slug: String = input
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Collapse repeated underscores and trim
    let mut collapsed = String::with_capacity(slug.len());
    let mut prev_underscore = false;
    for c in slug.chars() {
        if c == '_' {
            if !prev_underscore {
                collapsed.push(c);
            }
            prev_underscore = true;
        } else {
            collapsed.push(c);
            prev_underscore = false;
        }
    }
    slug = collapsed.trim_matches('_').to_string();
    if slug.is_empty() {
        slug = "m".to_string();
    }
    // Keep the derived table name readable and bounded
    if slug.len() > 40 {
        slug.truncate(40);
    }
    slug
}

/// Stable (run-to-run) FNV-1a 64-bit hash. std's DefaultHasher is randomly seeded
/// and would derive different table names across processes.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Derive the table set for an identity.
/// Local identity maps to the static tables; every other identity gets
/// `emb_{model_slug}_{dim}_{hash}` based tables (hash guards against
/// same-name-different-platform collisions).
///
/// 按身份派生表集:本地身份映射静态表;其余身份派生 emb_{模型slug}_{维度}_{哈希}。
pub fn collection_tables(identity: &EmbeddingIdentity) -> CollectionTables {
    if identity.is_local() {
        return CollectionTables::local_static();
    }
    let base = derived_table_base(identity);
    CollectionTables {
        content_vec: format!("{base}_content_vec"),
        content_data: format!("{base}_content_data"),
        meta_vec: format!("{base}_meta_vec"),
        meta_data: format!("{base}_meta_data"),
        index_prefix: base,
    }
}

/// Create the four tables of a collection (idempotent).
/// Shared by the v6 migration (local static tables) and identity switching (dynamic tables).
pub fn create_collection_tables(
    conn: &Connection,
    tables: &CollectionTables,
    dim: usize,
) -> Result<(), RepositoryError> {
    conn.execute_batch(&format!(
        r#"
create virtual table if not exists {content_vec} using vec0(
    id integer primary key autoincrement,
    embedding float[{dim}] distance_metric=cosine
);
create table if not exists {content_data}(
    id INTEGER PRIMARY KEY,
    file_id integer default 0 not null,
    chunk_index INTEGER default 0 not null,
    chunk_text text default '' NOT NULL,
    sparse_weights BLOB NOT NULL
);
create index if not exists idx_{ip}_file_id on {content_data}(file_id);

create virtual table if not exists {meta_vec} using vec0(
    id integer primary key autoincrement,
    embedding float[{dim}] distance_metric=cosine
);
create table if not exists {meta_data}(
    id INTEGER PRIMARY KEY,
    file_id integer default 0 not null,
    sparse_weights blob
);
create index if not exists idx_{ip}_meta_file_id on {meta_data}(file_id);
"#,
        content_vec = tables.content_vec,
        content_data = tables.content_data,
        meta_vec = tables.meta_vec,
        meta_data = tables.meta_data,
        ip = tables.index_prefix,
        dim = dim.max(1),
    ))?;
    Ok(())
}

/// Register a collection in the registry table (idempotent on platform+model).
/// The derived table base is stored so the management UI never has to re-derive it.
/// Re-registering the same pair with a DIFFERENT dim/has_sparse is rejected: the
/// tables were created at the old dim and re-ensuring at a new dim would fork the
/// collection (server-side model drift must surface as an explicit error, never
/// as a silent second table set).
pub fn register_collection(
    conn: &Connection,
    identity: &EmbeddingIdentity,
) -> Result<(), RepositoryError> {
    let base = derived_table_base(identity);
    conn.execute(
        "insert or ignore into embedding_collections(platform, model, dim, has_sparse, is_local, table_base) values (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            &identity.platform,
            &identity.model,
            identity.dim as i64,
            identity.has_sparse as i64,
            identity.is_local() as i64,
            base,
        ),
    )?;
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "select dim, has_sparse from embedding_collections where platform = ?1 and model = ?2",
            [&identity.platform, &identity.model],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(RepositoryError::from)?;
    if let Some((dim, has_sparse)) = existing
        && (dim as usize != identity.dim || (has_sparse != 0) != identity.has_sparse)
    {
        return Err(RepositoryError::InvalidOperation(format!(
            "collection {} · {} already exists with dim {dim} (requested {}), server-side model drift must be resolved explicitly",
            identity.model, identity.platform, identity.dim
        )));
    }
    Ok(())
}

/// The derived table base name for an identity ('' for the local static tables).
fn derived_table_base(identity: &EmbeddingIdentity) -> String {
    if identity.is_local() {
        return "".to_string();
    }
    format!(
        "emb_{}_{}_{:x}",
        slugify(&identity.model),
        identity.dim.max(1),
        fnv1a64(format!("{}|{}", identity.platform, identity.model).as_bytes())
    )
}

/// Ensure a collection exists: create tables if needed + register.
/// 建库:必要时建表并注册。
pub fn ensure_collection(identity: &EmbeddingIdentity) -> Result<CollectionTables, RepositoryError> {
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    let tables = collection_tables(identity);
    create_collection_tables(&tx, &tables, identity.dim)?;
    register_collection(&tx, identity)?;
    tx.commit()?;
    Ok(tables)
}

/// One row of the collection registry, enriched with live stats for the management UI.
#[derive(Debug, Serialize)]
pub struct EmbeddingCollectionInfo {
    pub platform: String,
    pub model: String,
    pub dim: usize,
    pub has_sparse: bool,
    pub is_local: bool,
    pub content_rows: i64,
    pub meta_rows: i64,
    pub file_count: i64,
    /// Estimated on-disk footprint in bytes (vec0 + data tables, row-count based)
    pub estimated_bytes: i64,
    pub create_time: String,
    pub update_time: String,
}

fn build_collection_row(
    row: &Row<'_>,
) -> rusqlite::Result<(String, String, usize, bool, bool, String, String)> {
    Ok((
        row.get("platform")?,
        row.get("model")?,
        row.get::<_, i64>("dim")? as usize,
        row.get::<_, i64>("has_sparse")? != 0,
        row.get::<_, i64>("is_local")? != 0,
        row.get("create_time")?,
        row.get("update_time")?,
    ))
}

/// List all registered collections with live row counts and a size estimate.
pub fn list_collections() -> Result<Vec<EmbeddingCollectionInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("select platform, model, dim, has_sparse, is_local, create_time, update_time from embedding_collections order by id asc")?;
    let rows = stmt.query_map([], build_collection_row)?;
    let mut result = Vec::new();
    for row in rows {
        let (platform, model, dim, has_sparse, is_local, create_time, update_time) = row?;
        let identity = EmbeddingIdentity {
            platform: platform.clone(),
            model: model.clone(),
            dim,
            has_sparse,
        };
        let tables = collection_tables(&identity);
        let content_rows: i64 = conn
            .query_row(
                &format!("select count(*) from {}", tables.content_vec),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let meta_rows: i64 = conn
            .query_row(
                &format!("select count(*) from {}", tables.meta_vec),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let file_count: i64 = conn
            .query_row(
                &format!("select count(distinct file_id) from {}", tables.content_data),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        // Estimate: dense f32 blob per row + data row overhead (chunk text averaged in)
        let estimated_bytes = content_rows * (dim as i64 * 4 + 256) + meta_rows * (dim as i64 * 4 + 64);
        result.push(EmbeddingCollectionInfo {
            platform,
            model,
            dim,
            has_sparse,
            is_local,
            content_rows,
            meta_rows,
            file_count,
            estimated_bytes,
            create_time,
            update_time,
        });
    }
    Ok(result)
}

/// Drop a non-active collection: four tables + registry row.
/// The local static tables are emptied instead of dropped (schema stays for the bundled backend).
pub fn drop_collection(identity: &EmbeddingIdentity) -> Result<(), RepositoryError> {
    let tables = collection_tables(identity);
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    if identity.is_local() {
        // Keep the static vec0 schema, just remove all rows
        for table in [
            &tables.content_vec,
            &tables.content_data,
            &tables.meta_vec,
            &tables.meta_data,
        ] {
            tx.execute(&format!("delete from {table}"), [])?;
        }
    } else {
        for table in [
            &tables.content_vec,
            &tables.content_data,
            &tables.meta_vec,
            &tables.meta_data,
        ] {
            tx.execute(&format!("drop table if exists {table}"), [])?;
        }
        tx.execute(
            "delete from embedding_collections where platform = ?1 and model = ?2",
            [&identity.platform, &identity.model],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Get a collection by identity, if registered.
pub fn get_collection(
    identity: &EmbeddingIdentity,
) -> Result<Option<EmbeddingCollectionInfo>, RepositoryError> {
    Ok(list_collections()?
        .into_iter()
        .find(|c| c.platform == identity.platform && c.model == identity.model))
}

// ---------------------------------------------------------------------------
// Identity persistence: the config table is the single source of truth.
// Repos read the active identity directly on every call so that GUI, CLI and
// the mfd daemon all agree without cross-process globals.
// ---------------------------------------------------------------------------

/// Active embedding identity, read from the config table (µs-level point query).
/// None = no vector library configured yet (fresh lite installs seed nothing;
/// full installs seed the local identity in the v6 migration). A corrupt value
/// logs and reads as unconfigured rather than silently pretending to be local.
pub fn active_identity() -> Option<EmbeddingIdentity> {
    let value = crate::repositories::config_repo::get_val(CONFIG_NAME_ACTIVE_EMBEDDING);
    if value.is_empty() {
        return None;
    }
    serde_json::from_str(&value)
        .map(Some)
        .unwrap_or_else(|e| {
            log::error!("Invalid active_embedding config, treated as unconfigured: {e}");
            None
        })
}

pub fn set_active_identity(identity: &EmbeddingIdentity) -> Result<(), RepositoryError> {
    let json = serde_json::to_string(identity)
        .map_err(|e| RepositoryError::InvalidInput(e.to_string()))?;
    crate::repositories::config_repo::upsert(CONFIG_NAME_ACTIVE_EMBEDDING, &json)?;
    Ok(())
}

/// Clear the active identity (back to the unconfigured state).
pub fn clear_active_identity() -> Result<(), RepositoryError> {
    crate::repositories::config_repo::upsert(CONFIG_NAME_ACTIVE_EMBEDDING, "")?;
    Ok(())
}

/// Identity whose collection currently holds a complete, searchable index.
/// Used by the guardrails (semantic search refuses while a rebuild is running)
/// and by the "switch back = instant" judgement.
pub fn indexed_identity() -> Option<EmbeddingIdentity> {
    let value = crate::repositories::config_repo::get_val(CONFIG_NAME_INDEXED_EMBEDDING);
    if value.is_empty() {
        return None;
    }
    serde_json::from_str(&value).ok()
}

/// Whether the active backend was unreachable at switch time (declared-dim
/// fallback used). Any later successful embed clears it.
pub fn service_unreachable() -> bool {
    crate::repositories::config_repo::get_val(CONFIG_NAME_SERVICE_UNREACHABLE) == "true"
}

pub fn set_service_unreachable(value: bool) -> Result<(), RepositoryError> {
    let value = if value { "true" } else { "" };
    crate::repositories::config_repo::upsert(CONFIG_NAME_SERVICE_UNREACHABLE, value)?;
    Ok(())
}

pub fn set_indexed_identity(identity: Option<&EmbeddingIdentity>) -> Result<(), RepositoryError> {
    match identity {
        Some(identity) => {
            let json = serde_json::to_string(identity)
                .map_err(|e| RepositoryError::InvalidInput(e.to_string()))?;
            crate::repositories::config_repo::upsert(CONFIG_NAME_INDEXED_EMBEDDING, &json)?;
        }
        None => {
            crate::repositories::config_repo::upsert(CONFIG_NAME_INDEXED_EMBEDDING, "")?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Delete cascade: one transaction, all collections
// ---------------------------------------------------------------------------

/// Delete a single file and every index row it owns: file_info + FTS + content/meta
/// rows in ALL registered collections. Single transaction - no orphan rows if a
/// delete is interrupted midway (the legacy path used one connection per table).
///
/// 单文件删除联动:file_info + FTS + 全部向量库的行,单事务收敛。
pub fn delete_file_with_indexes(file_id: i64) -> Result<(), RepositoryError> {
    if file_id < 1 {
        return Ok(());
    }
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    delete_file_index_rows_tx(&tx, file_id)?;
    tx.execute("delete from file_info where id = :file_id", &[(":file_id", &file_id)])?;
    tx.commit()?;
    Ok(())
}

/// Delete a file's index rows only (FTS + every collection), keeping the
/// file_info row - used before re-embedding an existing file.
///
/// 仅删除文件的索引行(FTS + 全部向量库),保留 file_info 行 — 重新嵌入前调用。
pub fn delete_index_rows_for_file(file_id: i64) -> Result<(), RepositoryError> {
    if file_id < 1 {
        return Ok(());
    }
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    delete_file_index_rows_tx(&tx, file_id)?;
    tx.commit()?;
    Ok(())
}

fn delete_file_index_rows_tx(tx: &Transaction, file_id: i64) -> Result<(), RepositoryError> {
    let collections = registered_table_sets(tx)?;
    for tables in &collections {
        tx.execute(
            &format!(
                "delete from {vec} where id in (select id from {data} where file_id = :file_id)",
                vec = tables.content_vec,
                data = tables.content_data
            ),
            &[(":file_id", &file_id)],
        )?;
        tx.execute(
            &format!("delete from {data} where file_id = :file_id", data = tables.content_data),
            &[(":file_id", &file_id)],
        )?;
        tx.execute(
            &format!(
                "delete from {vec} where id in (select id from {data} where file_id = :file_id)",
                vec = tables.meta_vec,
                data = tables.meta_data
            ),
            &[(":file_id", &file_id)],
        )?;
        tx.execute(
            &format!("delete from {data} where file_id = :file_id", data = tables.meta_data),
            &[(":file_id", &file_id)],
        )?;
    }
    tx.execute(
        "delete from file_content_fts where file_id = :file_id",
        &[(":file_id", &file_id)],
    )?;
    Ok(())
}

/// Delete every file under a directory prefix, same single-transaction coverage.
/// Returns the number of deleted file_info rows.
pub fn delete_files_with_prefix(prefix: &str) -> Result<usize, RepositoryError> {
    if prefix.is_empty() {
        return Ok(0);
    }
    let pattern = if prefix.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", prefix)
    } else {
        format!("{}{}%", prefix, std::path::MAIN_SEPARATOR)
    };
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    let collections = registered_table_sets(&tx)?;
    for tables in &collections {
        tx.execute(
            &format!(
                "delete from {vec} where id in (
                    select d.id from {data} d join file_info f on d.file_id = f.id where f.path like :pattern)",
                vec = tables.content_vec,
                data = tables.content_data
            ),
            &[(":pattern", &pattern)],
        )?;
        tx.execute(
            &format!(
                "delete from {data} where file_id in (select id from file_info where path like :pattern)",
                data = tables.content_data
            ),
            &[(":pattern", &pattern)],
        )?;
        tx.execute(
            &format!(
                "delete from {vec} where id in (
                    select d.id from {data} d join file_info f on d.file_id = f.id where f.path like :pattern)",
                vec = tables.meta_vec,
                data = tables.meta_data
            ),
            &[(":pattern", &pattern)],
        )?;
        tx.execute(
            &format!(
                "delete from {data} where file_id in (select id from file_info where path like :pattern)",
                data = tables.meta_data
            ),
            &[(":pattern", &pattern)],
        )?;
    }
    tx.execute(
        "delete from file_content_fts where file_id in (select id from file_info where path like :pattern)",
        &[(":pattern", &pattern)],
    )?;
    let affected = tx.execute(
        "delete from file_info where path like :pattern",
        &[(":pattern", &pattern)],
    )?;
    tx.commit()?;
    Ok(affected)
}

/// Clear the whole index in ONE transaction: every collection's rows (dynamic
/// tables dropped, static tables emptied), registry rows for non-local
/// collections, FTS and file_info. Interrupting midway can never leave rows
/// that the file_info-join delete paths could no longer reach.
///
/// 单事务清空全部索引:全部向量库(动态表 drop、静态表清空)、非本地注册行、
/// FTS 与 file_info —— 中途打断不会留下按 file_info 关联的删除路径再也够不到的孤儿行。
pub fn clear_all_collections() -> Result<(), RepositoryError> {
    let mut conn = crate::db_init_manager::open_db()?;
    let tx = conn.transaction()?;
    let collections = registered_table_sets(&tx)?;
    for tables in &collections {
        // Static local tables keep their vec0 schema; dynamic tables go away entirely
        if tables.content_vec == CollectionTables::local_static().content_vec {
            for table in [
                &tables.content_vec,
                &tables.content_data,
                &tables.meta_vec,
                &tables.meta_data,
            ] {
                tx.execute(&format!("delete from {table}"), [])?;
            }
        } else {
            for table in [
                &tables.content_vec,
                &tables.content_data,
                &tables.meta_vec,
                &tables.meta_data,
            ] {
                tx.execute(&format!("drop table if exists {table}"), [])?;
            }
        }
    }
    tx.execute("delete from embedding_collections where is_local = 0", [])?;
    tx.execute("delete from file_content_fts", [])?;
    tx.execute("delete from file_info", [])?;
    tx.commit()?;
    Ok(())
}

/// Recompute content/meta index statuses from the target collection's actual rows.
/// Used when switching back to an existing collection: files with vectors in the
/// target tables become Indexed instantly, everything else returns to Waiting.
/// Status messages are cleared so a previously failed file does not keep showing
/// a stale error after it becomes searchable again.
///
/// 切回已有向量库时按目标库现算 status:有向量的即 Indexed,其余回 Waiting;
/// 同时清空状态消息,避免失败文件恢复后仍显示旧错误。
pub fn restore_status_for(tables: &CollectionTables) -> Result<(), RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    conn.execute(
        &format!(
            "update file_info set
                content_index_status = case when exists(select 1 from {cd} where file_id = file_info.id) then 3 else 1 end,
                content_index_status_msg = '',
                meta_index_status = case when exists(select 1 from {md} where file_id = file_info.id) then 3 else 1 end,
                meta_index_status_msg = ''",
            cd = tables.content_data,
            md = tables.meta_data
        ),
        [],
    )?;
    Ok(())
}

/// Reset all files back to Waiting so the standard pipeline re-indexes them
/// (identity switch to a fresh collection).
pub fn reset_all_status_waiting() -> Result<(), RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    conn.execute(
        "update file_info set content_index_status = 1, meta_index_status = 1,
            content_index_status_msg = '', meta_index_status_msg = ''",
        [],
    )?;
    Ok(())
}

/// File ids that own at least one content chunk in the given collection,
/// paginated - drives the fast re-embed rebuild channel.
pub fn list_file_ids_in_collection(
    tables: &CollectionTables,
    min_file_id: i64,
    limit: i64,
) -> Result<Vec<i64>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "select distinct file_id from {} where file_id > ?1 order by file_id asc limit ?2",
        tables.content_data
    ))?;
    let rows = stmt.query_map([min_file_id, limit], |row| row.get::<_, i64>(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Table sets of every registered collection (used by the delete cascades).
fn registered_table_sets(conn: &Connection) -> Result<Vec<CollectionTables>, RepositoryError> {
    let mut stmt =
        conn.prepare("select platform, model, dim, has_sparse from embedding_collections")?;
    let rows = stmt.query_map([], |row| {
        Ok(EmbeddingIdentity {
            platform: row.get(0)?,
            model: row.get(1)?,
            dim: row.get::<_, i64>(2)? as usize,
            has_sparse: row.get::<_, i64>(3)? != 0,
        })
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .map(|identity| collection_tables(&identity))
        .collect())
}

/// Names used by the v6 migration to register the legacy static collection.
pub fn local_identity_for_migration() -> EmbeddingIdentity {
    EmbeddingIdentity {
        platform: LOCAL_PLATFORM.to_string(),
        model: LOCAL_MODEL.to_string(),
        dim: LOCAL_DIM,
        has_sparse: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_keeps_allowed_charset() {
        assert_eq!(slugify("BAAI/bge-m3"), "baai_bge_m3");
        assert_eq!(slugify("text-embedding-3-small"), "text_embedding_3_small");
        assert_eq!(slugify("///"), "m");
    }

    #[test]
    fn table_names_are_stable_and_disjoint() {
        let a = EmbeddingIdentity {
            platform: "openai".into(),
            model: "text-embedding-3-small".into(),
            dim: 1536,
            has_sparse: false,
        };
        let b = a.clone();
        assert_eq!(collection_tables(&a).content_vec, collection_tables(&b).content_vec);
        // Same model on another platform must derive a different table set
        let c = EmbeddingIdentity {
            platform: "siliconflow".into(),
            model: a.model.clone(),
            dim: 1536,
            has_sparse: false,
        };
        assert_ne!(collection_tables(&a).content_vec, collection_tables(&c).content_vec);
    }

    #[test]
    fn local_identity_maps_to_static_tables() {
        let tables = collection_tables(&EmbeddingIdentity::local());
        assert_eq!(tables.content_vec, "file_content_vec");
        assert_eq!(tables.meta_data, "file_metadata_data");
    }
}
