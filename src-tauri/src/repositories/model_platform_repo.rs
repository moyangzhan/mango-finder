use crate::entities::ModelPlatform;
use crate::repositories::RepositoryError;
use crate::utils::datetime_util;
use rusqlite::{Result, Row, named_params};

pub fn get_one(name: &str) -> Result<ModelPlatform, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from model_platform where name = ?1 limit 1")?;
    let one = stmt.query_row([name], |row| Ok(build_model_platform(row)?))?;
    return Ok(one);
}

pub fn list(names: &Vec<String>) -> Result<Vec<ModelPlatform>, RepositoryError> {
    let names_str = names
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("','");
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        format!(
            "select * from model_platform where name in ('{}')",
            names_str
        )
        .as_str(),
    )?;
    let rows = stmt.query_map([], |row| Ok(build_model_platform(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn update_by_name(name: &str, platform: &ModelPlatform) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "update model_platform set title = :title, base_url= :base_url, api_key= :api_key, logo= :logo, remark= :remark, is_proxy_enable= :is_proxy_enable,is_openai_api_compatible= :is_openai_api_compatible, update_time = datetime('now', 'localtime') where name = :name",
    )?;
    let affected: usize = stmt.execute(named_params! {
        ":name": name,
        ":title": &platform.title,
        ":logo": &platform.logo,
        ":remark": &platform.remark,
        ":is_proxy_enable": &platform.is_proxy_enable,
        ":is_openai_api_compatible": &platform.is_openai_api_compatible,
        ":api_key": &platform.api_key,
        ":base_url": &platform.base_url,
    })?;
    Ok(affected)
}

fn build_model_platform(row: &Row<'_>) -> Result<ModelPlatform, RepositoryError> {
    let create_time_str: String = row.get("create_time")?;
    let update_time_str: String = row.get("update_time")?;
    return Ok(ModelPlatform {
        id: row.get("id")?,
        name: row.get("name")?,
        title: row.get("title")?,
        logo: row.get("logo")?,
        base_url: row.get("base_url")?,
        api_key: row.get("api_key")?,
        remark: row.get("remark")?,
        is_proxy_enable: row.get("is_proxy_enable")?,
        is_openai_api_compatible: row.get("is_openai_api_compatible")?,
        create_time: datetime_util::str_to_datetime(create_time_str.as_str())?,
        update_time: datetime_util::str_to_datetime(update_time_str.as_str())?,
    });
}
