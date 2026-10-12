use anyhow::Result;
use log::info;
use rusqlite::Connection;

/// DB_VERSION = 5
/// Enforce one row per file_info.path: dedup legacy duplicates (a valid row
/// beats a smaller invalid id), then build the unique index.
/// file_info.path 每 path 一行:去重存量重复行(valid 行优先于更小的 invalid
/// id),再建唯一索引。
pub fn exec_ddl_with_conn(conn: &Connection) -> Result<()> {
    info!("dbv5.exec_ddl_with_conn");

    // content_vec/content_data are cascaded by trg_delete_file_content_data;
    // fts/metadata children and disk artifacts (parsed md/images) of loser
    // ids are NOT covered — disk leftovers are bounded and cleared by
    // `index clear`.
    // content_vec/content_data 由 trg_delete_file_content_data 级联清理;
    // fts/metadata 子表在此处理,败者 id 的磁盘产物(解析 md/图片)不覆盖 ——
    // 量极小,`index clear` 会清。
    conn.execute_batch(
        r#"
        begin immediate;

        -- Losers: same path but not the kept row (valid rows win, then min id)
        -- 败者:同 path 但非保留行(valid 优先,其次 min id)
        create temp table dup_loser_ids as
            select id from file_info
            where id not in (
                select min(id) from file_info where is_invalid = 0 group by path
                union all
                select min(id) from file_info f where is_invalid = 1
                    and not exists (
                        select 1 from file_info v
                        where v.path = f.path and v.is_invalid = 0
                    )
                group by path
            );

        delete from file_content_fts where file_id in (select id from dup_loser_ids);
        delete from file_metadata_vec where id in (
            select id from file_metadata_data
            where file_id in (select id from dup_loser_ids)
        );
        delete from file_metadata_data where file_id in (select id from dup_loser_ids);
        delete from file_info where id in (select id from dup_loser_ids);

        drop table dup_loser_ids;

        -- The unique index replaces the redundant non-unique idx_path
        -- 唯一索引替代冗余的非唯一 idx_path
        drop index if exists idx_path;
        create unique index if not exists uk_file_info_path on file_info(path);

        commit;
        "#,
    )?;

    Ok(())
}

pub fn init_data_with_conn(_conn: &Connection) -> Result<()> {
    info!("dbv5.init_data_with_conn");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn dedup_keeps_valid_row_and_enforces_unique_path() {
        // v1 creates vec0 virtual tables; the extension must be registered
        // v1 会建 vec0 虚拟表,需先注册扩展
        crate::init_sqlite_extensions();
        let conn = Connection::open_in_memory().unwrap();
        crate::db_migrations::db_v1::exec_ddl_with_conn(&conn).unwrap();

        let ins = |path: &str, invalid: i64| {
            conn.execute(
                "insert into file_info(path, is_invalid) values (?1, ?2)",
                (path, invalid),
            )
            .unwrap();
        };
        // a: invalid id=1 + valid id=2,3 → keep 2 (valid beats smaller invalid)
        // a: invalid id=1 + valid id=2,3 → 保留 2(valid 优先于更小的 invalid)
        ins("a", 1);
        ins("a", 0);
        ins("a", 0);
        // b: invalid only → keep min id
        // b: 仅 invalid → 保留 min id
        ins("b", 1);
        ins("b", 1);
        // c: single row, untouched
        ins("c", 0);

        exec_ddl_with_conn(&conn).unwrap();

        let id_of = |path: &str| {
            conn.query_row(
                "select id from file_info where path = ?1",
                [path],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(id_of("a"), 2);
        assert_eq!(id_of("b"), 4);
        assert_eq!(id_of("c"), 6);
        let total: i64 = conn
            .query_row("select count(*) from file_info", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 3);

        // Duplicate path now rejected by the unique index
        // 重复 path 被唯一索引拒绝
        let err = conn
            .execute("insert into file_info(path) values ('a')", [])
            .unwrap_err();
        assert!(err.to_string().contains("UNIQUE"));
    }
}
