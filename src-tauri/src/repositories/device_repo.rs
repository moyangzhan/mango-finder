use crate::entities::Device;
use crate::enums::{OnlineStatus, PairingStatus};
use crate::repositories::RepositoryError;
use crate::utils::datetime_util;
use rusqlite::{Row, named_params};

const ALL_COLUMNS: &str = "id, device_id, name, ip_address, port, version, online_status, pairing_status, pairing_remark, last_seen, first_discovered, index_count, capabilities, discovery_method, create_time, update_time";

/// Insert a new device
pub fn insert(device: &Device) -> Result<Device, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "INSERT INTO device (device_id, name, ip_address, port, version, online_status, pairing_status, last_seen, first_discovered, index_count, capabilities, discovery_method)
         VALUES (:device_id, :name, :ip_address, :port, :version, :online_status, :pairing_status, :last_seen, :first_discovered, :index_count, :capabilities, :discovery_method)"
    )?;
    stmt.execute(named_params! {
        ":device_id": &device.device_id,
        ":name": &device.name,
        ":ip_address": &device.ip_address,
        ":port": device.port,
        ":version": &device.version,
        ":online_status": <OnlineStatus as Into<&'static str>>::into(device.online_status),
        ":pairing_status": <PairingStatus as Into<&'static str>>::into(device.pairing_status),
        ":last_seen": datetime_util::datetime_to_str(&device.last_seen),
        ":first_discovered": datetime_util::datetime_to_str(&device.first_discovered),
        ":index_count": device.index_count,
        ":capabilities": &device.capabilities,
        ":discovery_method": &device.discovery_method,
    })?;
    let id = conn.last_insert_rowid();
    get_by_id(id)
}

/// Get device by ID
pub fn get_by_id(id: i64) -> Result<Device, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!("SELECT {} FROM device WHERE id = ?1", ALL_COLUMNS))?;
    let device = stmt.query_row([id], |row| Ok(build_device(row)?))?;
    Ok(device)
}

/// Get device by device_id (UUID)
pub fn get_by_device_id(device_id: &str) -> Result<Option<Device>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM device WHERE device_id = ?1",
        ALL_COLUMNS
    ))?;
    let result = stmt.query_row([device_id], |row| Ok(build_device(row)?));
    match result {
        Ok(device) => Ok(Some(device)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(RepositoryError::Database(e)),
    }
}

/// Get device by IP address and port
pub fn get_by_ip_and_port(ip_address: &str, port: i32) -> Result<Option<Device>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM device WHERE ip_address = ?1 AND port = ?2",
        ALL_COLUMNS
    ))?;
    let result = stmt.query_row(rusqlite::params![ip_address, port], |row| {
        Ok(build_device(row)?)
    });
    match result {
        Ok(device) => Ok(Some(device)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(RepositoryError::Database(e)),
    }
}

/// List all devices
pub fn list() -> Result<Vec<Device>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM device ORDER BY update_time DESC",
        ALL_COLUMNS
    ))?;
    let rows = stmt.query_map([], |row| Ok(build_device(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// List devices by pairing status
pub fn list_by_pairing_status(
    pairing_status: PairingStatus,
) -> Result<Vec<Device>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let status_str = <PairingStatus as Into<&'static str>>::into(pairing_status);
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM device WHERE pairing_status = ?1 ORDER BY update_time DESC",
        ALL_COLUMNS
    ))?;
    let rows = stmt.query_map([status_str], |row| Ok(build_device(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// List paired and online devices (for remote search)
pub fn list_paired_online() -> Result<Vec<Device>, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM device WHERE pairing_status = 'paired' AND online_status = 'online' ORDER BY name",
        ALL_COLUMNS
    ))?;
    let rows = stmt.query_map([], |row| Ok(build_device(row)?))?;
    let mut result = Vec::new();
    for item in rows {
        result.push(item?);
    }
    Ok(result)
}

/// Update device
pub fn update(device: &Device) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare(
        "UPDATE device SET
            name = :name,
            ip_address = :ip_address,
            port = :port,
            version = :version,
            online_status = :online_status,
            pairing_status = :pairing_status,
            last_seen = :last_seen,
            index_count = :index_count,
            capabilities = :capabilities
         WHERE id = :id",
    )?;
    let affected = stmt.execute(named_params! {
        ":id": device.id,
        ":name": &device.name,
        ":ip_address": &device.ip_address,
        ":port": device.port,
        ":version": &device.version,
        ":online_status": <OnlineStatus as Into<&'static str>>::into(device.online_status),
        ":pairing_status": <PairingStatus as Into<&'static str>>::into(device.pairing_status),
        ":last_seen": datetime_util::datetime_to_str(&device.last_seen),
        ":index_count": device.index_count,
        ":capabilities": &device.capabilities,
    })?;
    Ok(affected)
}

/// Update device online status
pub fn update_online_status(
    id: i64,
    online_status: OnlineStatus,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let status_str = <OnlineStatus as Into<&'static str>>::into(online_status);
    let affected = conn.execute(
        "UPDATE device SET online_status = ?1, last_seen = datetime('now', 'localtime') WHERE id = ?2",
        [status_str, &id.to_string()],
    )?;
    Ok(affected)
}

/// Update pairing status with transition validation
/// 带状态转换验证的更新配对状态
///
/// Status transition rules:
/// - None → PendingIn/PendingOut → Paired → Rejected
/// - Left side can transition to right side
/// - Right side cannot override left side (except manual reset)
/// 状态转换规则:
/// - None → PendingIn/PendingOut → Paired → Rejected
/// - 左侧可以转换到右侧
/// - 右侧不能覆盖左侧（手动重置除外）
pub fn update_pairing_status(
    id: i64,
    new_status: PairingStatus,
    is_manual: bool,
) -> Result<usize, RepositoryError> {
    update_pairing_status_with_remark(id, new_status, "", is_manual)
}

/// Update pairing status with remark (describes why the status changed)
/// 更新配对状态并附带说明（描述状态变化原因）
///
/// See `update_pairing_status` for transition rules.
/// 状态转换规则参见 `update_pairing_status`。
pub fn update_pairing_status_with_remark(
    id: i64,
    new_status: PairingStatus,
    remark: &str,
    is_manual: bool,
) -> Result<usize, RepositoryError> {
    // Get current device status
    let device = get_by_id(id)
        .map_err(|e| RepositoryError::NotFound(format!("Device {} not found: {}", id, e)))?;
    let current_status = device.pairing_status;

    // Validate status transition
    if !current_status.can_transition_to(&new_status, is_manual) {
        log::warn!(
            "Invalid pairing status transition: {:?} -> {:?} (is_manual: {})",
            current_status,
            new_status,
            is_manual
        );
        return Err(RepositoryError::InvalidOperation(format!(
            "Cannot transition from {:?} to {:?}",
            current_status, new_status
        )));
    }

    let conn = crate::db_init_manager::open_db()?;
    let status_str = <PairingStatus as Into<&'static str>>::into(new_status);
    let affected = conn.execute(
        "UPDATE device SET pairing_status = ?1, pairing_remark = ?2 WHERE id = ?3",
        rusqlite::params![status_str, remark, id],
    )?;
    Ok(affected)
}

/// Update device info (from remote ping response)
/// NOTE: This function does NOT update online_status.
/// online_status should only be updated by device_checker.
/// 注意：此函数不更新 online_status。
/// online_status 只能由 device_checker 更新.
pub fn update_device_info(
    device_id: &str,
    name: &str,
    index_count: i64,
    capabilities: &str,
) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute(
        "UPDATE device SET name = ?1, index_count = ?2, capabilities = ?3, last_seen = datetime('now', 'localtime') WHERE device_id = ?4",
        [name, &index_count.to_string(), capabilities, device_id],
    )?;
    Ok(affected)
}

/// Delete device by ID
pub fn delete_by_id(id: i64) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute("DELETE FROM device WHERE id = ?1", [id])?;
    Ok(affected)
}

/// Delete device by device_id
pub fn delete_by_device_id(device_id: &str) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let affected = conn.execute("DELETE FROM device WHERE device_id = ?1", [device_id])?;
    Ok(affected)
}

/// Upsert device (insert or update by device_id)
///
/// NOTE: This function preserves existing `pairing_status` and `online_status`.
/// - `pairing_status` should only be changed via HTTP pairing endpoints or user manual operations
/// - `online_status` should only be changed via device_checker
/// 注意：此函数保留现有的 pairing_status 和 online_status。
/// - pairing_status 只能通过 HTTP 配对端点或用户手动操作更改
/// - online_status 只能通过 device_checker 更改
pub fn upsert(device: &Device) -> Result<Device, RepositoryError> {
    let existing = get_by_device_id(&device.device_id)?;
    match existing {
        Some(mut existing) => {
            // Update existing device
            // Note: Don't override online_status from mDNS discovery
            // Let device_checker handle online/offline state
            existing.name = device.name.clone();
            existing.ip_address = device.ip_address.clone();
            existing.port = device.port;
            existing.version = device.version.clone();
            // Keep existing online_status - only status checker should update this
            // existing.online_status = device.online_status;
            // Keep existing pairing_status - only HTTP pairing endpoints or user operations should update this
            // existing.pairing_status = device.pairing_status;
            existing.index_count = device.index_count;
            existing.capabilities = device.capabilities.clone();
            existing.last_seen = device.last_seen;
            update(&existing)?;
            Ok(existing)
        }
        None => {
            // Insert new device
            insert(device)
        }
    }
}

/// Count all devices
pub fn count() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM device", [], |row| row.get(0))?;
    Ok(count)
}

/// Count paired devices
pub fn count_paired() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM device WHERE pairing_status = 'paired'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Count devices with pending_in pairing status (waiting for local user to accept)
pub fn count_pending_in() -> Result<i64, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM device WHERE pairing_status = 'pending_in'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn build_device(row: &Row<'_>) -> Result<Device, RepositoryError> {
    let last_seen_str: String = row.get("last_seen")?;
    let first_discovered_str: String = row.get("first_discovered")?;
    let create_time_str: String = row.get("create_time")?;
    let update_time_str: String = row.get("update_time")?;

    let online_status_str: String = row.get("online_status")?;
    let pairing_status_str: String = row.get("pairing_status")?;

    Ok(Device {
        id: row.get("id")?,
        device_id: row.get("device_id")?,
        name: row.get("name")?,
        ip_address: row.get("ip_address")?,
        port: row.get("port")?,
        version: row.get("version")?,
        online_status: OnlineStatus::from(online_status_str.as_str()),
        pairing_status: PairingStatus::from(pairing_status_str.as_str()),
        pairing_remark: row.get("pairing_remark")?,
        last_seen: datetime_util::str_to_datetime(&last_seen_str)?,
        first_discovered: datetime_util::str_to_datetime(&first_discovered_str)?,
        index_count: row.get("index_count")?,
        capabilities: row.get("capabilities")?,
        discovery_method: row.get("discovery_method")?,
        create_time: datetime_util::str_to_datetime(&create_time_str)?,
        update_time: datetime_util::str_to_datetime(&update_time_str)?,
    })
}
