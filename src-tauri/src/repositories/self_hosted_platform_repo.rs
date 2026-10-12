use crate::entities::SelfHostedPlatform;
use crate::repositories::RepositoryError;
use crate::utils::datetime_util;
use rusqlite::{Result, Row, named_params};

pub fn get_one(name: &str) -> Result<SelfHostedPlatform, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from self_hosted_platform where name = ?1 limit 1")?;
    let one = stmt.query_row([name], |row| Ok(build_self_hosted_platform(row)?))?;
    return Ok(one);
}

pub fn list() -> Result<Vec<SelfHostedPlatform>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from self_hosted_platform order by id")?;
    let rows = stmt.query_map([], |row| Ok(build_self_hosted_platform(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn update_by_name(name: &str, platform: &SelfHostedPlatform) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "update self_hosted_platform set title = :title, host = :host, port = :port, remark = :remark, update_time = datetime('now', 'localtime') where name = :name",
    )?;
    let affected: usize = stmt.execute(named_params! {
        ":name": name,
        ":title": &platform.title,
        ":host": &platform.host,
        ":port": &platform.port,
        ":remark": &platform.remark,
    })?;
    Ok(affected)
}

fn build_self_hosted_platform(row: &Row<'_>) -> Result<SelfHostedPlatform, RepositoryError> {
    let create_time_str: String = row.get("create_time")?;
    let update_time_str: String = row.get("update_time")?;
    return Ok(SelfHostedPlatform {
        id: row.get("id")?,
        name: row.get("name")?,
        title: row.get("title")?,
        host: row.get("host")?,
        port: row.get("port")?,
        remark: row.get("remark")?,
        create_time: datetime_util::str_to_datetime(create_time_str.as_str())?,
        update_time: datetime_util::str_to_datetime(update_time_str.as_str())?,
    });
}
