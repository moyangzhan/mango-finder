use crate::entities::PairingRequest;
use crate::enums::PairingRequestStatus;
use crate::repositories::RepositoryError;
use crate::utils::datetime_util;
use rusqlite::{Row, named_params};
use rust_i18n::t;

const ALL_COLUMNS: &str = "id, device_id, device_name, ip_address, port, direction, status, remark, response_time, create_time, update_time";

/// Insert a new pairing request log
pub fn insert(request: &PairingRequest) -> Result<PairingRequest, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "INSERT INTO pairing_request (device_id, device_name, ip_address, port, direction, status, remark)
         VALUES (:device_id, :device_name, :ip_address, :port, :direction, :status, :remark)"
    )?;
    stmt.execute(named_params! {
        ":device_id": &request.device_id,
        ":device_name": &request.device_name,
        ":ip_address": &request.ip_address,
        ":port": request.port,
        ":direction": &request.direction,
        ":status": <PairingRequestStatus as Into<&'static str>>::into(request.status),
        ":remark": &request.remark,
    })?;
    let id = conn.last_insert_rowid();
    get_by_id(id)
}

/// Get pairing request by ID
pub fn get_by_id(id: i64) -> Result<PairingRequest, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request WHERE id = ?1",
        ALL_COLUMNS
    ))?;
    let request = stmt.query_row([id], |row| Ok(build_pairing_request(row)?))?;
    Ok(request)
}

/// List all pairing requests
pub fn list() -> Result<Vec<PairingRequest>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request ORDER BY create_time DESC",
        ALL_COLUMNS
    ))?;
    let rows = stmt.query_map([], |row| Ok(build_pairing_request(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// List pairing requests by direction
pub fn list_by_direction(direction: &str) -> Result<Vec<PairingRequest>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request WHERE direction = ?1 ORDER BY create_time DESC",
        ALL_COLUMNS
    ))?;
    let rows = stmt.query_map([direction], |row| Ok(build_pairing_request(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// List pending pairing requests
pub fn list_pending() -> Result<Vec<PairingRequest>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request WHERE status = 'pending' ORDER BY create_time DESC",
        ALL_COLUMNS
    ))?;
    let rows = stmt.query_map([], |row| Ok(build_pairing_request(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// Get pending request by device ID and direction
pub fn get_pending_by_device_id(
    device_id: &str,
    direction: &str,
) -> Result<Option<PairingRequest>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request WHERE device_id = ?1 AND direction = ?2 AND status = 'pending' ORDER BY create_time DESC LIMIT 1",
        ALL_COLUMNS
    ))?;
    let result = stmt.query_row([device_id, direction], |row| {
        Ok(build_pairing_request(row)?)
    });
    match result {
        Ok(request) => Ok(Some(request)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(RepositoryError::Database(e)),
    }
}

/// Get latest request by device ID and direction (any status)
/// 获取指定方向的最新请求（不限状态）
///
/// This returns the most recent request for a device in the specified direction, regardless of its status.
/// Useful when you want to update the latest request even if older requests exist.
pub fn get_latest_by_device_id(
    device_id: &str,
    direction: &str,
) -> Result<Option<PairingRequest>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request WHERE device_id = ?1 AND direction = ?2 ORDER BY create_time DESC LIMIT 1",
        ALL_COLUMNS
    ))?;
    let result = stmt.query_row([device_id, direction], |row| {
        Ok(build_pairing_request(row)?)
    });
    match result {
        Ok(request) => Ok(Some(request)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(RepositoryError::Database(e)),
    }
}

/// Get latest request by device ID (any direction, any status)
/// 获取该设备的最新请求（不限方向、不限状态）
pub fn get_latest_by_device_id_any(
    device_id: &str,
) -> Result<Option<PairingRequest>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pairing_request WHERE device_id = ?1 ORDER BY create_time DESC LIMIT 1",
        ALL_COLUMNS
    ))?;
    let result = stmt.query_row([device_id], |row| Ok(build_pairing_request(row)?));
    match result {
        Ok(request) => Ok(Some(request)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(RepositoryError::Database(e)),
    }
}

/// Update pairing request status
pub fn update_status(id: i64, status: PairingRequestStatus) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let status_str = <PairingRequestStatus as Into<&'static str>>::into(status);
    let affected = conn.execute(
        "UPDATE pairing_request SET status = ?1, response_time = datetime('now', 'localtime') WHERE id = ?2",
        [status_str, &id.to_string()],
    )?;
    Ok(affected)
}

/// Update pairing request status with remark
/// 更新配对请求状态并附带说明
pub fn update_status_with_remark(
    id: i64,
    status: PairingRequestStatus,
    remark: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let status_str = <PairingRequestStatus as Into<&'static str>>::into(status);
    let affected = conn.execute(
        "UPDATE pairing_request SET status = ?1, remark = ?2, response_time = datetime('now', 'localtime') WHERE id = ?3",
        rusqlite::params![status_str, remark, id],
    )?;
    Ok(affected)
}

/// Accept pairing request
pub fn accept(id: i64) -> Result<usize, RepositoryError> {
    update_status(id, PairingRequestStatus::Accepted)
}

/// Reject pairing request
pub fn reject(id: i64) -> Result<usize, RepositoryError> {
    update_status(id, PairingRequestStatus::Rejected)
}

/// Reject pairing request with remark
/// 拒绝配对请求并附带说明
pub fn reject_with_remark(id: i64, remark: &str) -> Result<usize, RepositoryError> {
    update_status_with_remark(id, PairingRequestStatus::Rejected, remark)
}

/// Delete pairing request by ID
pub fn delete_by_id(id: i64) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute("DELETE FROM pairing_request WHERE id = ?1", [id])?;
    Ok(affected)
}

/// Delete all requests for a specific device
pub fn delete_by_device_id(device_id: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute(
        "DELETE FROM pairing_request WHERE device_id = ?1",
        [device_id],
    )?;
    Ok(affected)
}

/// Delete all pairing requests
pub fn delete_all() -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute("DELETE FROM pairing_request", [])?;
    Ok(affected)
}

/// Expire old pending requests (older than 24 hours)
/// 过期旧的待处理请求（超过24小时）
///
/// This function also resets device pairing status:
/// 此函数还会重置设备配对状态：
/// - For outgoing requests: device pairing_status is reset from pending_out to none
/// - 对于发出请求的设备：将 pairing_status 从 pending_out 重置为 none
/// - For incoming requests: device pairing_status is reset from pending_in to none
/// - 对于接收请求的设备：将 pairing_status 从 pending_in 重置为 none
pub fn expire_old_requests() -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;

    // First, get device_ids for requests that will be expired, grouped by direction
    let out_device_ids: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT device_id FROM pairing_request
             WHERE status = 'pending' AND direction = 'out'
             AND datetime(create_time, '+24 hours') < datetime('now', 'localtime')",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let in_device_ids: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT device_id FROM pairing_request
             WHERE status = 'pending' AND direction = 'in'
             AND datetime(create_time, '+24 hours') < datetime('now', 'localtime')",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    // Get localized remark for expired requests
    let expired_remark = t!("pairing.remark.request-expired").to_string();

    // Expire the requests and update remark
    let affected = conn.execute(
        "UPDATE pairing_request SET status = 'expired', remark = ?1, response_time = datetime('now', 'localtime')
         WHERE status = 'pending' AND datetime(create_time, '+24 hours') < datetime('now', 'localtime')",
        [&expired_remark],
    )?;

    // Reset device pairing status from pending_out to none with remark for expired outgoing requests
    for device_id in out_device_ids {
        let _ = conn.execute(
            "UPDATE device SET pairing_status = 'none', pairing_remark = ?1 WHERE device_id = ?2 AND pairing_status = 'pending_out'",
            rusqlite::params![&expired_remark, &device_id],
        );
    }

    // Reset device pairing status from pending_in to none with remark for expired incoming requests
    for device_id in in_device_ids {
        let _ = conn.execute(
            "UPDATE device SET pairing_status = 'none', pairing_remark = ?1 WHERE device_id = ?2 AND pairing_status = 'pending_in'",
            rusqlite::params![&expired_remark, &device_id],
        );
    }

    Ok(affected)
}

/// Count pending requests
pub fn count_pending() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pairing_request WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Count pending incoming requests
pub fn count_pending_in() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pairing_request WHERE status = 'pending' AND direction = 'in'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Count pending outgoing requests
pub fn count_pending_out() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pairing_request WHERE status = 'pending' AND direction = 'out'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn build_pairing_request(row: &Row<'_>) -> Result<PairingRequest, RepositoryError> {
    let response_time_str: Option<String> = row.get("response_time")?;
    let create_time_str: String = row.get("create_time")?;
    let update_time_str: String = row.get("update_time")?;
    let status_str: String = row.get("status")?;

    Ok(PairingRequest {
        id: row.get("id")?,
        device_id: row.get("device_id")?,
        device_name: row.get("device_name")?,
        ip_address: row.get("ip_address")?,
        port: row.get("port")?,
        direction: row.get("direction")?,
        status: PairingRequestStatus::from(status_str.as_str()),
        remark: row.get("remark")?,
        response_time: response_time_str
            .filter(|s| !s.is_empty())
            .map(|s| datetime_util::str_to_datetime(&s))
            .transpose()?,
        create_time: datetime_util::str_to_datetime(&create_time_str)?,
        update_time: datetime_util::str_to_datetime(&update_time_str)?,
    })
}
