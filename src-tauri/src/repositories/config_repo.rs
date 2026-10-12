use crate::entities::Config;
use crate::repositories::RepositoryError;
use rusqlite::Result;

pub fn get_one(config_name: &str) -> Result<Option<Config>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from config where name = ?1 limit 1")?;
    let hit_config = match stmt.query_row([config_name], |row| {
        Ok(Config {
            id: row.get("id")?,
            name: row.get("name")?,
            value: row.get("value")?,
        })
    }) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("get one error, config_name: {}, error: {}", config_name, e);
            return Ok(None);
        }
    };
    return Ok(Some(hit_config));
}

pub fn get_val(config_name: &str) -> String {
    let conn = match crate::db_init_manager::open_db() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open db: {}", e);
            return "".to_string();
        }
    };
    let mut stmt = match conn.prepare("select value from config where name = ?1 limit 1") {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to prepare stmt: {}", e);
            return "".to_string();
        }
    };
    let config = match stmt.query_row([config_name], |row| Ok(row.get(0)?)) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("Failed to query map: {}", e);
            return "".to_string();
        }
    };
    return config;
}

pub fn insert(config_name: &str, config_value: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("insert into config (name, value) values (?1, ?2)")?;
    let affected = stmt.execute([config_name, config_value])?;
    Ok(affected)
}

pub fn insert_or_ignore(config_name: &str, config_value: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("insert or ignore into config (name, value) values (?1, ?2)")?;
    let affected = stmt.execute([config_name, config_value])?;
    Ok(affected)
}

pub fn update_by_name(config_name: &str, new_value: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("update config set value = ?1 where name = ?2")?;
    let affected = stmt.execute([new_value, config_name])?;
    Ok(affected)
}

/// Get config value as Option<String>
pub fn get_value(config_name: &str) -> Result<Option<String>, String> {
    let conn =
        crate::db_init_manager::open_db().map_err(|e| format!("Failed to open db: {}", e))?;
    let mut stmt = conn
        .prepare("select value from config where name = ?1 limit 1")
        .map_err(|e| format!("Failed to prepare stmt: {}", e))?;
    let result = stmt.query_row([config_name], |row| {
        let value: String = row.get(0)?;
        Ok(value)
    });
    match result {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to query: {}", e)),
    }
}

/// Save or update a config value (upsert)
/// 保存或更新配置值（upsert）
pub fn upsert(config_name: &str, config_value: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "INSERT INTO config (name, value) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET value = ?2"
    )?;
    let affected = stmt.execute([config_name, config_value])?;
    Ok(affected)
}

/// Atomically increment a numeric config value (creates it at 1); returns the
/// new value. Used as a cache-generation counter shared across processes.
/// 原子递增数值型配置(首次创建为 1),返回新值。用作跨进程共享的缓存版本号。
pub fn increment(config_name: &str) -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let n: i64 = conn.query_row(
        "INSERT INTO config (name, value) VALUES (?1, '1')
         ON CONFLICT(name) DO UPDATE SET value = CAST(config.value AS INTEGER) + 1
         RETURNING CAST(value AS INTEGER)",
        [config_name],
        |row| row.get(0),
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn test_update_by_name() {
        let result = update_by_name("active_locale", "zh-CN");
        println!("update result: {:?}", result);
    }
}
