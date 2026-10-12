pub mod db_v1;
pub mod db_v2;
pub mod db_v3;
pub mod db_v4;
pub mod db_v5;

use crate::global::{CONFIG_NAME_DB_VERSION, DB_VERSION};
use anyhow::Result;
use log::{error, info};
use rusqlite::Connection;

/// Run all migrations using the provided connection (for fresh database)
pub fn run_all_migrations(conn: &Connection) -> Result<()> {
    info!("Running all migrations...");

    // Create config table first (if not exists)
    conn.execute_batch(
        r#"create table if not exists config(
            id    integer primary key autoincrement,
            name  varchar(50) default '' unique not null,
            value text        default ''        not null,
            create_time text  default '' not null,
            update_time text  default '' not null
        );
        create trigger if not exists config_create_time
        after insert on config
        for each row
        begin
            update config
            set create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        create trigger if not exists config_update_time
        after update on config
        for each row
        begin
            update config set update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        "#,
    )?;

    // Run all version migrations
    db_v1::exec_ddl_with_conn(conn)?;
    db_v1::init_data_with_conn(conn)?;

    db_v2::exec_ddl_with_conn(conn)?;
    db_v2::init_data_with_conn(conn)?;

    db_v3::exec_ddl_with_conn(conn)?;
    db_v3::init_data_with_conn(conn)?;

    db_v4::exec_ddl_with_conn(conn)?;
    db_v4::init_data_with_conn(conn)?;

    db_v5::exec_ddl_with_conn(conn)?;
    db_v5::init_data_with_conn(conn)?;

    // Update version number
    conn.execute(
        "insert or replace into config (name, value) VALUES (?1, ?2)",
        (CONFIG_NAME_DB_VERSION, DB_VERSION.to_string()),
    )?;

    info!("All migrations completed, version: {}", DB_VERSION);
    Ok(())
}

/// Initialize database with incremental migrations (for existing database)
pub fn init_with_conn(conn: &Connection) -> Result<()> {
    let mut current_db_version: i32 = 0;

    // Check if config table exists
    let config_exists: bool = conn
        .query_row(
            "select count(*) from sqlite_master where type='table' and name='config'",
            [],
            |row| row.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;

    if config_exists {
        current_db_version = conn
            .query_row(
                &format!(
                    "select value from config where name='{}'",
                    CONFIG_NAME_DB_VERSION
                ),
                [],
                |row| {
                    let value: String = row.get(0)?;
                    Ok(value.parse().unwrap_or(0))
                },
            )
            .unwrap_or_else(|e| {
                error!("db_version not found, set to 0, error:{:?}", e);
                0
            });
    }

    info!(
        "Current DB version: {}, target version: {}",
        current_db_version, DB_VERSION
    );

    if current_db_version == DB_VERSION {
        return Ok(());
    }

    // Run required migrations
    for version in current_db_version + 1..=DB_VERSION {
        info!("Running migration to version: {}", version);
        match version {
            1 => {
                db_v1::exec_ddl_with_conn(conn)?;
                db_v1::init_data_with_conn(conn)?;
            }
            2 => {
                db_v2::exec_ddl_with_conn(conn)?;
                db_v2::init_data_with_conn(conn)?;
            }
            3 => {
                db_v3::exec_ddl_with_conn(conn)?;
                db_v3::init_data_with_conn(conn)?;
            }
            4 => {
                db_v4::exec_ddl_with_conn(conn)?;
                db_v4::init_data_with_conn(conn)?;
            }
            5 => {
                db_v5::exec_ddl_with_conn(conn)?;
                db_v5::init_data_with_conn(conn)?;
            }
            _ => {}
        }
    }

    // Update version number
    conn.execute(
        "insert or replace into config (name, value) VALUES (?1, ?2)",
        (CONFIG_NAME_DB_VERSION, DB_VERSION.to_string()),
    )?;

    Ok(())
}
