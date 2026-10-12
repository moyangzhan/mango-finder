use crate::entities::FileInfo;
use crate::repositories::RepositoryError;
use crate::utils::datetime_util;
use chrono::{DateTime, Local};
use rusqlite::{Result, Row, named_params};

const ALL_COLUMNS: &str = "id, name, category, path, file_ext, file_size, content, content_ref_path, content_index_status, content_index_status_msg, meta_index_status, meta_index_status_msg, is_invalid, invalid_reason, md5, metadata, audio_type, image_hash, audio_fingerprint, file_create_time, file_update_time, create_time, update_time";
/// ALL_COLUMNS minus `content`, derived so new columns are included automatically
/// 从 ALL_COLUMNS 去掉 `content` 列,新增列自动纳入
fn meta_columns() -> String {
    ALL_COLUMNS
        .split(", ")
        .filter(|c| *c != "content")
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn insert(file_info: &FileInfo) -> Result<Option<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "insert into file_info(name,category,path,file_ext,file_size,content,content_ref_path,metadata,md5,is_invalid,invalid_reason,file_create_time,file_update_time) values (:name,:category,:path,:file_ext,:file_size,:content,:content_ref_path,:metadata,:md5,:is_invalid,:invalid_reason,:file_create_time,:file_update_time)"
    )?;
    let last_insert_rowid = stmt.insert(named_params! {
        ":name": &file_info.name,
        ":category": file_info.category,
        ":path": &file_info.path,
        ":file_ext": &file_info.file_ext,
        ":file_size": file_info.file_size,
        ":content": &file_info.content,
        ":content_ref_path": &file_info.content_ref_path,
        ":metadata": &file_info.metadata.to_json(),
        ":md5": &file_info.md5,
        ":is_invalid": file_info.is_invalid,
        ":invalid_reason": &file_info.invalid_reason,
        ":file_create_time": datetime_util::micro_datetime_to_str(&file_info.file_create_time),
        ":file_update_time": datetime_util::micro_datetime_to_str(&file_info.file_update_time),
    })?;
    let mut query_stmt = conn.prepare("select * from file_info where rowid = ?1")?;
    let file_info = query_stmt
        .query_row([last_insert_rowid], |row| Ok(Some(build_file_info(row)?)))
        .unwrap_or_else(|e| {
            log::debug!("file_info_repo.insert() Error: {}", e);
            None
        });

    Ok(file_info)
}

pub fn update(file_info: &FileInfo) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "update file_info set name =:name,path=:path,file_ext=:file_ext,file_size=:file_size,content=:content,content_ref_path=:content_ref_path,md5=:md5,is_invalid=:is_invalid,invalid_reason=:invalid_reason,metadata=:metadata,file_update_time=:file_update_time where id = :id",
    )?;
    let affected = stmt.execute(named_params! {
        ":id": &file_info.id,
        ":name": &file_info.name,
        ":path": &file_info.path,
        ":file_ext": &file_info.file_ext,
        ":file_size": &file_info.file_size,
        ":content": &file_info.content,
        ":content_ref_path": &file_info.content_ref_path,
        ":metadata": &file_info.metadata.to_json(),
        ":md5": &file_info.md5,
        ":is_invalid": &file_info.is_invalid,
        ":invalid_reason": &file_info.invalid_reason,
        ":file_update_time": datetime_util::micro_datetime_to_str(&file_info.file_update_time),
    })?;
    log::debug!("update file_info affected: {:?}", affected);
    Ok(affected)
}

pub fn update_content_meta(
    file_id: i64,
    content: &str,
    content_ref_path: Option<&str>,
    meta: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("update file_info set content = :content, content_ref_path = :content_ref_path, metadata = :meta where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":content": &content,
        ":content_ref_path": &content_ref_path,
        ":meta": &meta,
    })?;
    Ok(affected)
}

pub fn update_content_only(
    file_id: i64,
    content: &str,
    content_ref_path: Option<&str>,
) -> Result<(), RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    conn.execute(
        "update file_info set content = :content, content_ref_path = :content_ref_path where id = :id",
        named_params! { ":id": &file_id, ":content": &content, ":content_ref_path": &content_ref_path },
    )?;
    Ok(())
}

/// Count all indexed files (content_index_status = Indexed) in one query;
/// category-agnostic total matching the GUI's "indexed files" metric.
/// 单条查询统计全部已索引文件(content_index_status = Indexed),
/// 与 GUI 的 indexed_files 口径一致。
pub fn count_indexed() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row(
        "select count(*) from file_info where content_index_status = 3",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn count_indexed_by_category(category: i64) -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count = conn.query_row(
        "select count(*) from file_info where content_index_status = 3 and category = :category",
        named_params! { ":category": category },
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn update_invalid(
    file_id: i64,
    is_invalid: bool,
    invalid_reason: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("update file_info set is_invalid = :is_invalid,invalid_reason = :invalid_reason where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":is_invalid": &is_invalid,
        ":invalid_reason": &invalid_reason,
    })?;
    Ok(affected)
}

pub fn update_content_index_status(
    file_id: i64,
    index_status: i64,
    index_status_reason: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("update file_info set content_index_status = :index_status, content_index_status_msg = :index_status_msg where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":index_status": &index_status,
        ":index_status_msg": &index_status_reason,
    })?;
    Ok(affected)
}

pub fn update_meta_index_status(
    file_id: i64,
    index_status: i64,
    index_status_reason: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("update file_info set meta_index_status = :index_status, meta_index_status_msg = :index_status_msg where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":index_status": &index_status,
        ":index_status_msg": &index_status_reason,
    })?;
    Ok(affected)
}

pub fn list(page: i64, size: i64) -> Result<Vec<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("select * from file_info order by id desc limit :size offset :offset")?;
    let rows = stmt.query_map(
        named_params! {
            ":size": size,
            ":offset": (page - 1) * size,
        },
        |row| Ok(build_file_info(row)?),
    )?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn list_in_columns(
    select_columns: &str,
    page: i64,
    size: i64,
) -> Result<Vec<FileInfo>, RepositoryError> {
    if select_columns.is_empty() {
        return Err(RepositoryError::InvalidParam(
            "select_columns is empty".to_string(),
        ));
    }
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        format!(
            "select {} from file_info limit :size offset :offset",
            select_columns
        )
        .as_str(),
    )?;
    let rows = stmt.query_map(
        named_params! {
            ":size": size,
            ":offset": (page - 1) * size,
        },
        |row| Ok(build_file_info(row)?),
    )?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn count() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select count(*) from file_info")?;
    let count = stmt.query_row([], |row| row.get(0))?;
    Ok(count)
}

/// Returns (id, path) pairs so the path-search cache can carry the real DB id
/// (the cache previously fabricated ids from its line number, which drifted
/// after any deletion or incremental append).
/// 返回 (id, path) 二元组,让路径搜索缓存携带真实 DB id(此前缓存用行号
/// 伪造 id,删除或增量追加后即错位)。
pub fn list_paths(page: i64, size: i64, asc: bool) -> Result<Vec<(i64, String)>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let order_direction = if asc { "asc" } else { "desc" };
    let sql = format!(
        "select id, path from file_info order by id {} limit :size offset :offset",
        order_direction
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        named_params! {
            ":size": size,
            ":offset": (page - 1) * size,
        },
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    )?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn list_unindexed_files(
    min_id: i64,
    limit: i64,
    category: i64,
) -> Result<Vec<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let sql = format!(
        "select {} from file_info where id > :min_id and content_index_status = 1 and category = :category order by id asc limit :limit",
        ALL_COLUMNS
    );
    // File content is not included in the result
    let mut stmt = conn.prepare(sql.as_str())?;
    let rows = stmt.query_map(
        named_params! {
            ":min_id": min_id,
            ":category": category,
            ":limit": limit,
        },
        |row| Ok(build_file_info(row)?),
    )?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn count_unindexed() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select count(*) from file_info where content_index_status = 1")?;
    let count = stmt.query_row([], |row| row.get(0))?;
    Ok(count)
}

pub fn count_unindexed_files(category: i64) -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "select count(*) from file_info where content_index_status = 1 and category = :category",
    )?;
    let count = stmt.query_row(named_params! {":category": category}, |row| row.get(0))?;
    Ok(count)
}

pub fn list_by_ids(ids: &Vec<i64>) -> Result<Vec<FileInfo>, RepositoryError> {
    list_by_ids_with_columns(ids, ALL_COLUMNS)
}

/// 结果列表回表时不取 `content` 列(正文按需另取)
/// Omit the `content` column (fetched on demand) when hydrating result lists
pub fn list_meta_by_ids(ids: &Vec<i64>) -> Result<Vec<FileInfo>, RepositoryError> {
    list_by_ids_with_columns(ids, &meta_columns())
}

fn list_by_ids_with_columns(ids: &Vec<i64>, columns: &str) -> Result<Vec<FileInfo>, RepositoryError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let conn = crate::db_init_manager::open_db()?;

    // Process in batches of 500 to avoid oversized SQL statements
    let batch_size = 500;
    let mut result = Vec::new();

    for chunk in ids.chunks(batch_size) {
        let placeholders: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(i, _)| format!(":id{}", i))
            .collect();
        let sql = format!(
            "select {} from file_info where id in ({})",
            columns,
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<(String, i64)> = chunk
            .iter()
            .enumerate()
            .map(|(i, id)| (format!(":id{}", i), *id))
            .collect();
        let param_refs: Vec<(&str, &i64)> = params.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let rows = stmt.query_map(&param_refs[..], |row| Ok(build_file_info(row)?))?;
        for item in rows {
            result.push(item?);
        }
    }

    Ok(result)
}

pub fn list_by_min_update_time(
    select_columns: &str,
    min_update_time: &DateTime<Local>,
    page: i64,
    size: i64,
) -> Result<Vec<FileInfo>, RepositoryError> {
    let update_time = datetime_util::datetime_to_str(min_update_time);
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        format!(
            "select {} from file_info where update_time > :min_update_time order by id desc limit :size offset :offset",
            select_columns
        )
        .as_str(),
    )?;
    let rows = stmt.query_map(named_params! {":min_update_time": update_time, ":size": size, ":offset": (page - 1) * size, }, |row| Ok(build_file_info(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn list_paths_by_min_update_time(
    min_update_time: &DateTime<Local>,
    page: i64,
    size: i64,
) -> Result<Vec<(i64, String)>, RepositoryError> {
    let update_time = datetime_util::datetime_to_str(min_update_time);
    log::debug!("update_time: {}", update_time);
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "select id, path from file_info where update_time > :min_update_time order by id desc limit :size offset :offset",
    )?;
    let rows = stmt.query_map(named_params! {":min_update_time": update_time, ":size": size, ":offset": (page - 1) * size, }, |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn count_by_min_update_time(min_update_time: &DateTime<Local>) -> Result<i64, RepositoryError> {
    let update_time = datetime_util::datetime_to_str(min_update_time);
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("select count(*) from file_info where update_time > :min_update_time")?;
    let count = stmt.query_row(named_params! {":min_update_time": update_time}, |row| {
        row.get(0)
    })?;
    Ok(count)
}

/// Get all file IDs by a single category (for similarity search filtering)
pub fn list_ids_by_category(category: i64) -> Result<Vec<i64>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("select id from file_info where category = :category and is_invalid = 0")?;
    let rows = stmt.query_map(named_params! {":category": category}, |row| row.get(0))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// Get all file IDs by category and audio_type (for music similarity search)
/// 根据类别和音频类型获取文件ID列表（用于音乐相似性搜索）
pub fn list_ids_by_category_and_audio_type(
    category: i64,
    audio_type: i32,
) -> Result<Vec<i64>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "select id from file_info where category = :category and audio_type = :audio_type and is_invalid = 0"
    )?;
    let rows = stmt.query_map(
        named_params! {
            ":category": category,
            ":audio_type": audio_type,
        },
        |row| row.get(0),
    )?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// Update audio_type for a file
/// 更新文件的音频类型
pub fn update_audio_type(file_id: i64, audio_type: i32) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("update file_info set audio_type = :audio_type where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":audio_type": &audio_type,
    })?;
    Ok(affected)
}

/// Update image_hash for a file (for image similarity search)
/// 更新文件的图像哈希（用于图像相似性搜索）
pub fn update_image_hash(file_id: i64, image_hash: &[u8]) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("update file_info set image_hash = :image_hash where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":image_hash": image_hash,
    })?;
    Ok(affected)
}

/// Update audio_fingerprint for a file (for music similarity search)
/// 更新文件的音频指纹（用于音乐相似性搜索）
pub fn update_audio_fingerprint(
    file_id: i64,
    audio_fingerprint: &[u8],
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("update file_info set audio_fingerprint = :audio_fingerprint where id = :id")?;
    let affected = stmt.execute(named_params! {
        ":id": &file_id,
        ":audio_fingerprint": audio_fingerprint,
    })?;
    Ok(affected)
}

/// Get all files by a single category (for similarity search)
pub fn list_by_category(category: i64) -> Result<Vec<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let sql = format!(
        "select {} from file_info where category = :category and is_invalid = 0",
        ALL_COLUMNS
    );
    let mut stmt = conn.prepare(sql.as_str())?;
    let rows = stmt.query_map(named_params! {":category": category}, |row| {
        Ok(build_file_info(row)?)
    })?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// Count files by category (for migration progress)
pub fn count_by_category(category: i64) -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count = conn.query_row(
        "select count(*) from file_info where category = :category and is_invalid = 0",
        named_params! { ":category": category },
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get files by category with pagination (for batched migration)
pub fn list_by_category_paged(
    category: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let sql = format!(
        "select {} from file_info where category = :category and is_invalid = 0 order by id asc limit :limit offset :offset",
        ALL_COLUMNS
    );
    let mut stmt = conn.prepare(sql.as_str())?;
    let rows = stmt.query_map(
        named_params! {
            ":category": category,
            ":limit": limit,
            ":offset": offset,
        },
        |row| Ok(build_file_info(row)?),
    )?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// Get all files by multiple categories (for similarity search across document types)
pub fn list_by_categories(categories: &[i64]) -> Result<Vec<FileInfo>, RepositoryError> {
    if categories.is_empty() {
        return Ok(Vec::new());
    }
    let categories_str = categories
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let conn = crate::db_init_manager::open_db()?;
    let sql = format!(
        "select {} from file_info where category in ({}) and is_invalid = 0",
        ALL_COLUMNS, categories_str
    );
    let mut stmt = conn.prepare(sql.as_str())?;
    let rows = stmt.query_map([], |row| Ok(build_file_info(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn get_by_id(file_id: i64) -> Result<Option<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from file_info where id = ?1")?;
    match stmt.query_row([file_id], |row: &Row<'_>| Ok(build_file_info(row)?)) {
        Ok(hit) => return Ok(Some(hit)),
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => {
            log::error!("file_info_repo.get_by_id() Error: {}", e);
            return Err(RepositoryError::Database(e));
        }
    }
}

pub fn get_by_md5(md5: &str) -> Result<Option<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from file_info where md5 = ?1 limit 1")?;
    match stmt.query_row([md5], |row: &Row<'_>| Ok(build_file_info(row)?)) {
        Ok(hit) => return Ok(Some(hit)),
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => {
            log::error!("file_info_repo.get_by_md5(), md5: {}, Error: {}", md5, e);
            return Err(RepositoryError::Database(e));
        }
    }
}

pub fn get_by_path(path: &str) -> Result<Option<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from file_info where path = ?1 limit 1")?;
    match stmt.query_row([path], |row: &Row<'_>| Ok(build_file_info(row)?)) {
        Ok(hit) => return Ok(Some(hit)),
        Err(_) => {
            return Ok(None);
        }
    }
}

pub fn list_by_name(name: &str) -> Result<Vec<FileInfo>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select * from file_info where name = ?1 order by id asc")?;
    let rows = stmt.query_map([name], |row| Ok(build_file_info(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// List id + image_hash of all image files with a stored hash
/// 列出所有已存储感知哈希的图片文件（供外部文件纯视觉比对使用）
pub fn list_image_hashes() -> Result<Vec<(i64, Option<Vec<u8>>)>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "select id, image_hash from file_info where category = 2 and image_hash is not null",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

pub fn delete_by_id(file_id: i64) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_info where id = ?1")?;
    let affected = stmt.execute([file_id])?;
    log::debug!("delete file_info by id affected: {:?}", affected);
    Ok(affected)
}

pub fn delete_by_path(path: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_info where path = ?1")?;
    let affected = stmt.execute([path])?;
    log::debug!("delete file_info by path affected: {:?}", affected);
    Ok(affected)
}

pub fn clear() -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_info")?;
    let affected = stmt.execute([])?;
    Ok(affected)
}

pub fn delete_by_prefix_path(pre_path: &str) -> Result<usize, RepositoryError> {
    if pre_path.is_empty() {
        return Ok(0);
    }
    let pattern = if pre_path.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", pre_path)
    } else {
        format!("{}{}%", pre_path, std::path::MAIN_SEPARATOR)
    };
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute(
        "DELETE FROM file_info WHERE path = ?1 OR path LIKE ?2",
        (pre_path, pattern),
    )?;
    log::debug!("delete file_info by prefix path affected: {:?}", affected);
    Ok(affected)
}

pub fn count_by_prefix_path(pre_path: &str) -> Result<i64, RepositoryError> {
    if pre_path.is_empty() {
        return Ok(0);
    }
    let pattern = if pre_path.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", pre_path)
    } else {
        format!("{}{}%", pre_path, std::path::MAIN_SEPARATOR)
    };
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("select count(*) from file_info where path LIKE ?1")?;
    let count = stmt.query_row([pattern], |row| row.get(0))?;
    Ok(count)
}

pub fn list_paths_by_prefix_path(pre_path: &str) -> Result<Vec<String>, RepositoryError> {
    if pre_path.is_empty() {
        return Ok(vec![]);
    }
    let pattern = if pre_path.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", pre_path)
    } else {
        format!("{}{}%", pre_path, std::path::MAIN_SEPARATOR)
    };
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("SELECT path FROM file_info WHERE path = ?1 OR path LIKE ?2")?;
    let paths = stmt
        .query_map((pre_path, pattern), |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(paths)
}

pub fn replace_directory_prefix_path(
    old_pre_path: &str,
    new_pre_path: &str,
) -> Result<usize, RepositoryError> {
    let pattern = if old_pre_path.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", old_pre_path)
    } else {
        format!("{}{}%", old_pre_path, std::path::MAIN_SEPARATOR)
    };
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute(
        "UPDATE file_info SET path = REPLACE(path, ?1, ?2) WHERE path LIKE ?3",
        (old_pre_path, new_pre_path, pattern),
    )?;
    log::debug!("replace file_info by prefix path affected: {:?}", affected);
    Ok(affected)
}

pub fn rename(old_path: &str, new_path: &str, new_name: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute(
        "UPDATE file_info SET path = ?1, name = ?2 WHERE path = ?3",
        (new_path, new_name, old_path),
    )?;
    log::debug!("rename file_info affected: {:?}", affected);
    Ok(affected)
}

fn build_file_info(row: &Row<'_>) -> Result<FileInfo, RepositoryError> {
    let file_create_time: String = row.get("file_create_time").unwrap_or_default();
    let file_update_time: String = row.get("file_update_time").unwrap_or_default();
    let create_time_str: String = row.get("create_time").unwrap_or_default();
    let update_time_str: String = row.get("update_time").unwrap_or_default();
    let meta: String = row.get("metadata")?;
    let content = row.get("content").unwrap_or_default();
    let content_ref_path: Option<String> = row.get("content_ref_path").ok();
    let image_hash: Option<Vec<u8>> = row.get("image_hash").ok();
    return Ok(FileInfo {
        id: row.get("id")?,
        name: row.get("name").unwrap_or_default(),
        category: row.get("category").unwrap_or_default(),
        path: row.get("path").unwrap_or_default(),
        file_ext: row.get("file_ext").unwrap_or_default(),
        file_size: row.get("file_size").unwrap_or_default(),
        content: content,
        content_ref_path,
        content_index_status: row.get("content_index_status").unwrap_or_default(),
        content_index_status_msg: row.get("content_index_status_msg").unwrap_or_default(),
        meta_index_status: row.get("meta_index_status").unwrap_or_default(),
        meta_index_status_msg: row.get("meta_index_status_msg").unwrap_or_default(),
        is_invalid: row.get("is_invalid").unwrap_or_default(),
        invalid_reason: row.get("invalid_reason").unwrap_or_default(),
        md5: row.get("md5").unwrap_or_default(),
        metadata: crate::structs::file_metadata::FileMetadata::from_json(&meta),
        audio_type: row.get("audio_type").unwrap_or_default(),
        audio_fingerprint: row.get("audio_fingerprint").unwrap_or_default(),
        image_hash,
        file_create_time: datetime_util::str_to_micro_datetime(file_create_time.as_str())?,
        file_update_time: datetime_util::str_to_micro_datetime(file_update_time.as_str())?,
        create_time: datetime_util::str_to_datetime(create_time_str.as_str())?,
        update_time: datetime_util::str_to_datetime(update_time_str.as_str())?,
    });
}
