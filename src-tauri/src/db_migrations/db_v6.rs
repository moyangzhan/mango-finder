use crate::repositories::embedding_collection_repo::{
    self, CollectionTables,
};
use crate::structs::embedding_identity::{EmbeddingIdentity, LOCAL_DIM};
use anyhow::Result;
use rusqlite::Connection;

/// v6: embedding vector collections.
///
/// - `embedding_collections` registry; legacy static tables registered as the
///   local collection (existing content index stays in place, zero rebuild).
/// - `active_embedding` / `indexed_embedding` config keys (identity single
///   source of truth, read directly by the repo layer).
/// - Metadata tables rebuilt at the local dense dim (1024): the old 256-dim
///   column could never hold the model's dense output (silent zero-vector bug),
///   so the metadata semantic index has been dead weight until now. Rows are
///   reset to Waiting and re-embedded by the startup catch-up job.
/// - `trg_delete_file_content_data` dropped: app-level deletes already cover
///   FTS + metadata + all collections in one transaction; the trigger covered
///   only the static content tables and turned any missed path into silent
///   ghost rows in the other collections.
/// - Seed embedding models for the remote/self-hosted channels.
///
/// v6:嵌入向量分库化。
/// - embedding_collections 注册表;存量静态表注册为本地向量库(内容索引原地保留,零重建)。
/// - active_embedding / indexed_embedding 配置键(身份单一事实源,repo 层直读)。
/// - metadata 表按本地 dense 维度(1024)重建:旧 256 维列恒无法容纳模型输出
///   (静默零向量 bug),metadata 语义索引一直无效;行状态重置为 Waiting,由启动
///   补嵌任务重新嵌入。
/// - 删除触发器 trg_delete_file_content_data:应用层删除已单事务覆盖 FTS +
///   metadata + 全部向量库;触发器只覆盖静态 content 表,任何漏调路径都会在
///   其他向量库留下幽灵行。
/// - 为远程/自托管通道预置 embedding 模型。
pub fn exec_ddl_with_conn(conn: &Connection) -> Result<()> {
    // Registry of vector collections
    conn.execute_batch(
        r#"
create table if not exists embedding_collections(
    id integer primary key autoincrement,
    platform text default '' not null,   -- 'local' | 'ollama'/'vllm' | model platform name
    model text default '' not null,      -- model name, e.g. 'bge-m3'
    dim integer default 0 not null,      -- dense dimension of this collection
    has_sparse integer default 0 not null,
    is_local integer default 0 not null, -- 1 = legacy static tables
    table_base text default '' not null, -- derived table base, '' for local static
    create_time text default '' not null,
    update_time text default '' not null,
    unique(platform, model)
);
create trigger if not exists embedding_collections_create_time
after insert on embedding_collections
for each row
begin
    update embedding_collections
    set create_time = datetime('now', 'localtime'),
        update_time = datetime('now', 'localtime')
    where id = new.id;
end;
create trigger if not exists embedding_collections_update_time
after update on embedding_collections
for each row
begin
    update embedding_collections set update_time = datetime('now', 'localtime')
    where id = new.id;
end;
"#,
    )?;

    // Rebuild metadata tables at the real local dense dim. The 256-dim tables only
    // ever held zero vectors (dense 1024 could not fit), nothing of value is lost.
    conn.execute_batch(
        r#"
drop trigger if exists trg_delete_file_content_data;
drop table if exists file_metadata_data;
drop table if exists file_metadata_vec;
"#,
    )?;

    // Full seeds the local collection as the out-of-box backend; lite starts
    // with no vector library (tables are created on demand at the first switch)
    if !crate::global::IS_LITE {
        let static_tables = CollectionTables::local_static();
        embedding_collection_repo::create_collection_tables(conn, &static_tables, LOCAL_DIM)?;

        // Register the local collection; the existing 1024 content index stays in place
        embedding_collection_repo::register_collection(conn, &EmbeddingIdentity::local())?;

        // Metadata rows are all zero vectors today: queue them for re-embedding.
        // Content statuses are untouched - the content index remains valid.
        conn.execute_batch(
            "update file_info set meta_index_status = 1, meta_index_status_msg = '';",
        )?;
    }

    Ok(())
}

pub fn init_data_with_conn(conn: &Connection) -> Result<()> {
    // Full defaults both identities to the local backend; lite leaves them unset
    if !crate::global::IS_LITE {
        let local = EmbeddingIdentity::local();
        let identity_json = serde_json::to_string(&local)?;
        conn.execute(
            "insert or ignore into config (name, value) values ('active_embedding', ?1)",
            [&identity_json],
        )?;
        conn.execute(
            "insert or ignore into config (name, value) values ('indexed_embedding', ?1)",
            [&identity_json],
        )?;
    }

    // Seed embedding models for the remote/self-hosted channels (editable via the UI)
    conn.execute_batch(
        r#"
insert or ignore into ai_model(name, title, model_types, platform, properties, is_enable)
values ('text-embedding-3-small', 'text-embedding-3-small', 'embedding', 'openai', '{"dimension": 1536}', 1);
insert or ignore into ai_model(name, title, model_types, platform, properties, is_enable)
values ('BAAI/bge-m3', 'BGE-M3', 'embedding', 'siliconflow', '{"dimension": 1024}', 1);
insert or ignore into ai_model(name, title, model_types, platform, properties, is_enable)
values ('bge-m3', 'BGE-M3', 'embedding', 'ollama', '{"dimension": 1024}', 1);
insert or ignore into ai_model(name, title, model_types, platform, properties, is_enable)
values ('BAAI/bge-m3', 'BGE-M3', 'embedding', 'vllm', '{"dimension": 1024}', 1);
"#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v6_recreates_metadata_tables_at_local_dim() {
        // vec0 virtual tables need the sqlite-vec extension registered
        crate::init_sqlite_extensions();
        let conn = Connection::open_in_memory().expect("open in-memory db");
        // Minimal prerequisites so the migration body can run standalone
        conn.execute_batch(
            r#"
create table config(
    id integer primary key autoincrement,
    name varchar(50) default '' unique not null,
    value text default '' not null,
    create_time text default '' not null,
    update_time text default '' not null
);
create table file_info(
    id integer primary key autoincrement,
    meta_index_status integer default 1,
    meta_index_status_msg text default '' not null
);
create table ai_model(
    id integer primary key autoincrement,
    name text default '' not null,
    title text default '' not null,
    model_types text default '' not null,
    platform text default '' not null,
    properties text default '{}' not null,
    is_enable integer default 1 not null,
    unique(name, platform)
);
"#,
        )
        .expect("prereq schema");
        exec_ddl_with_conn(&conn).expect("ddl");
        init_data_with_conn(&conn).expect("data");

        if crate::global::IS_LITE {
            let collections: i64 = conn
                .query_row("select count(*) from embedding_collections", [], |r| r.get(0))
                .expect("count collections");
            assert_eq!(collections, 0, "lite must not register the local collection");
            let active_rows: i64 = conn
                .query_row(
                    "select count(*) from config where name = 'active_embedding'",
                    [],
                    |r| r.get(0),
                )
                .expect("count active_embedding rows");
            assert_eq!(active_rows, 0, "lite must not seed an active identity");
            let models: i64 = conn
                .query_row(
                    "select count(*) from ai_model where model_types = 'embedding'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(models, 4, "embedding model seeds");
            return;
        }

        let dim: i64 = conn
            .query_row(
                "select dim from embedding_collections where platform = 'local'",
                [],
                |r| r.get(0),
            )
            .expect("local collection registered");
        assert_eq!(dim as usize, LOCAL_DIM);

        let trigger_count: i64 = conn
            .query_row(
                "select count(*) from sqlite_master where type='trigger' and name='trg_delete_file_content_data'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(trigger_count, 0, "legacy delete trigger must be dropped");

        let active: String = conn
            .query_row("select value from config where name='active_embedding'", [], |r| r.get(0))
            .unwrap();
        assert!(active.contains("\"local\""));

        let meta_dim: String = conn
            .query_row(
                "select sql from sqlite_master where name='file_metadata_vec'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(meta_dim.contains(&format!("float[{LOCAL_DIM}]")), "metadata vec0 rebuilt at local dim, got: {meta_dim}");

        let models: i64 = conn
            .query_row(
                "select count(*) from ai_model where model_types = 'embedding'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(models, 4, "embedding model seeds");
    }
}
