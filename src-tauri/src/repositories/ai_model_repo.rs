use crate::entities::AiModel;
use crate::repositories::RepositoryError;
use crate::utils::datetime_util;
use rusqlite::{Result, Row, named_params};

pub fn get_one(platform: &str, name: &str) -> Result<Option<AiModel>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "SELECT * FROM ai_model where name =:name and platform=:platform and is_enable=1 limit 1",
    )?;
    let one = stmt
        .query_row(&[(":name", name), (":platform", platform)], |row| {
            Ok(Some(build_ai_model(row)?))
        })
        .unwrap_or_else(|e| {
            log::debug!("ai_model_repo.get_one() Error: {}", e);
            None
        });
    return Ok(one);
}

pub fn get_one_by_type(platform: &str, one_type: &str) -> Result<Option<AiModel>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "select * from ai_model where model_types like '%' || :one_type || '%' and platform=:platform and is_enable=1 limit 1",
    )?;
    let one = stmt
        .query_row(&[(":one_type", one_type), (":platform", platform)], |row| {
            Ok(Some(build_ai_model(row)?))
        })
        .unwrap_or_else(|e| {
            log::debug!("ai_model_repo.get_one_by_type() Error: {}", e);
            None
        });
    return Ok(one);
}

pub fn insert(ai_model: &AiModel) -> Result<AiModel, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "insert into ai_model(name,title,model_types,setting,remark,platform,context_window,max_input_tokens,max_output_tokens,input_types,properties,is_reasoner,is_thinking_closable,is_free,is_enable) values (:name,:title,:model_types,:setting,:remark,:platform,:context_window,:max_input_tokens,:max_output_tokens,:input_types,:properties,:is_reasoner,:is_thinking_closable,:is_free,:is_enable)"
    )?;
    let last_insert_rowid = stmt.insert(named_params! {
        ":name": &ai_model.name,
        ":title": &ai_model.title,
        ":model_types": &ai_model.model_types,
        ":setting": &ai_model.setting,
        ":remark": &ai_model.remark,
        ":platform": &ai_model.platform,
        ":context_window": &ai_model.context_window,
        ":max_input_tokens": &ai_model.max_input_tokens,
        ":max_output_tokens": &ai_model.max_output_tokens,
        ":input_types": &ai_model.input_types,
        ":properties": &ai_model.properties,
        ":is_reasoner": &ai_model.is_reasoner,
        ":is_thinking_closable": &ai_model.is_thinking_closable,
        ":is_free": &ai_model.is_free,
        ":is_enable": &ai_model.is_enable,
    })?;
    let mut query_stmt = conn.prepare("select * from ai_model where rowid = ?1")?;
    let ai_model = query_stmt.query_row([last_insert_rowid], |row| Ok(build_ai_model(row)?))?;

    Ok(ai_model)
}

pub fn update(ai_model: &AiModel) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "update ai_model set name =:name,title=:title,model_types=:model_types,setting=:setting,remark=:remark,platform=:platform,context_window=:context_window,max_input_tokens=:max_input_tokens,max_output_tokens=:max_output_tokens,input_types=:input_types,properties=:properties,is_reasoner=:is_reasoner,is_thinking_closable=:is_thinking_closable,is_free=:is_free,is_enable=:is_enable, update_time = datetime('now', 'localtime') where id = :id",
    )?;
    let affected = stmt.execute(named_params! {
        ":id": &ai_model.id,
        ":name": &ai_model.name,
        ":title": &ai_model.title,
        ":model_types": &ai_model.model_types,
        ":setting": &ai_model.setting,
        ":remark": &ai_model.remark,
        ":platform": &ai_model.platform,
        ":context_window": &ai_model.context_window,
        ":max_input_tokens": &ai_model.max_input_tokens,
        ":max_output_tokens": &ai_model.max_output_tokens,
        ":input_types": &ai_model.input_types,
        ":properties": &ai_model.properties,
        ":is_reasoner": &ai_model.is_reasoner,
        ":is_thinking_closable": &ai_model.is_thinking_closable,
        ":is_free": &ai_model.is_free,
        ":is_enable": &ai_model.is_enable,
    })?;
    log::debug!("update ai_model affected: {:?}", affected);
    Ok(affected)
}

pub fn update_basic(
    id: i64,
    name: &str,
    title: &str,
    remark: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "update ai_model set name = :name, title = :title, remark = :remark, update_time = datetime('now', 'localtime') where id = :id",
    )?;
    let affected = stmt.execute(named_params! {
        ":id": id,
        ":name": name,
        ":title": title,
        ":remark": remark,
    })?;
    Ok(affected)
}

pub fn list_by_platform(platform: &str) -> Result<Vec<AiModel>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("select * from ai_model where platform = :platform and is_enable = 1")?;
    let rows = stmt.query_map(&[(":platform", platform)], |row| Ok(build_ai_model(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

fn build_ai_model(row: &Row<'_>) -> Result<AiModel, RepositoryError> {
    let create_time_str: String = row.get("create_time")?;
    let update_time_str: String = row.get("update_time")?;
    return Ok(AiModel {
        id: row.get("id")?,
        name: row.get("name")?,
        title: row.get("title")?,
        model_types: row.get("model_types")?,
        setting: row.get("setting")?,
        remark: row.get("remark")?,
        platform: row.get("platform")?,
        context_window: row.get("context_window")?,
        max_input_tokens: row.get("max_input_tokens")?,
        max_output_tokens: row.get("max_output_tokens")?,
        input_types: row.get("input_types")?,
        properties: row.get("properties")?,
        is_reasoner: row.get("is_reasoner")?,
        is_thinking_closable: row.get("is_thinking_closable")?,
        is_free: row.get("is_free")?,
        is_enable: row.get("is_enable")?,
        create_time: datetime_util::str_to_datetime(create_time_str.as_str())?,
        update_time: datetime_util::str_to_datetime(update_time_str.as_str())?,
    });
}
