use crate::cluster::{self, CLUSTER_SETTING};
use crate::entities::Device;
use crate::enums::{OnlineStatus, PairingStatus};
use crate::global::APP_HANDLE;
use crate::repositories::{device_repo, pairing_request_repo};
use crate::structs::cluster_config::ClusterSetting;
use rust_i18n::t;
use tauri::{Emitter, command};

// ========== Device commands ==========

#[command]
pub fn get_local_ip() -> Result<String, String> {
    cluster::http_client::get_local_ip()
}

#[command]
pub fn load_devices() -> Result<Vec<Device>, String> {
    device_repo::list().map_err(|e| e.to_string())
}

#[command]
pub fn load_devices_by_status(pairing_status: &str) -> Result<Vec<Device>, String> {
    let status = PairingStatus::from(pairing_status);
    device_repo::list_by_pairing_status(status).map_err(|e| e.to_string())
}

#[command]
pub async fn reject_device(device_id: String) -> Result<(), String> {
    // Get device info first
    let device = device_repo::get_by_device_id(&device_id)
        .map_err(|e| e.to_string())?
        .ok_or("Device not found")?;

    // Update device status to Rejected with remark (local user rejected remote device)
    // 更新设备状态为 Rejected 并附带说明（本机用户拒绝了对方）
    let remark = t!("pairing.remark.user-rejected-device").to_string();
    device_repo::update_pairing_status_with_remark(
        device.id,
        PairingStatus::Rejected,
        &remark,
        true,
    )
    .map_err(|e| e.to_string())?;

    if let Ok(Some(request)) = pairing_request_repo::get_latest_by_device_id_any(&device_id) {
        log::info!("Rejecting pairing request {}", request.id);
        let _ = pairing_request_repo::reject_with_remark(request.id, &remark);
    }

    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit("device-rejected", &device_id);
    }

    // Send rejection response to the remote device
    // 发送拒绝响应给对方设备
    let device_id_clone = device_id.clone();
    let ip = device.ip_address.clone();
    let port = device.port;
    tokio::spawn(async move {
        if let Err(e) = crate::cluster::http_client::send_pairing_response(
            &device_id_clone,
            &ip,
            port,
            false, // rejected
        )
        .await
        {
            log::warn!(
                "Failed to send rejection response to {}: {}",
                device_id_clone,
                e
            );
        }
    });

    log::info!("Device {} rejected", device_id);
    Ok(())
}

#[command]
pub async fn unreject_device(device_id: String) -> Result<Device, String> {
    // Alias for reset_pairing_status for backward compatibility
    // 向后兼容的别名
    reset_pairing_status(device_id).await
}

/// Reset device pairing status to None (manual operation)
/// 重置设备配对状态为 None（手动操作）
///
/// This can reset ANY pairing status to None, including:
/// - pending_in / pending_out
/// - paired
/// - rejected
/// 可以将任意配对状态重置为 None，包括:
/// - pending_in / pending_out
/// - paired
/// - rejected
#[command]
pub async fn reset_pairing_status(device_id: String) -> Result<Device, String> {
    // Get device from database
    let mut device = device_repo::get_by_device_id(&device_id)
        .map_err(|e| e.to_string())?
        .ok_or("Device not found")?;

    // Store previous status before reset
    // 重置前保存之前的状态
    let previous_status = device.pairing_status;
    let previous_status_str = <PairingStatus as Into<&'static str>>::into(previous_status);

    // Try to ping the device to check if it's reachable (for info update and pairing decision)
    // 尝试 ping 设备来检查是否可达（用于信息更新和配对决策）
    let is_reachable = crate::cluster::http_client::ping_and_update_device_info(&mut device)
        .await
        .is_ok();

    // Reset pairing status to None (manual operation)
    let remark = t!("pairing.remark.status-reset").to_string();
    device_repo::update_pairing_status_with_remark(device.id, PairingStatus::None, &remark, true)
        .map_err(|e| e.to_string())?;
    device.pairing_status = PairingStatus::None;

    // Send reset notify to remote device
    // 发送重置通知给对方设备（前端已控制只有特定状态才显示重置按钮）
    {
        let remote_device_id = device.device_id.clone();
        let ip = device.ip_address.clone();
        let port = device.port;
        let previous_status_str = previous_status_str.to_string();

        tokio::spawn(async move {
            if let Err(e) = crate::cluster::http_client::send_reset_notify(
                &remote_device_id,
                &ip,
                port,
                &previous_status_str,
            )
            .await
            {
                log::warn!("Failed to send reset notify: {}", e);
            }
        });
    }

    // If reachable and auto_request_pairing is enabled, send pairing request
    if is_reachable {
        let setting = crate::cluster::get_cluster_setting().await;
        if setting.auto_request_pairing {
            let device_id = device.device_id.clone();
            let device_name = device.name.clone();
            let ip = device.ip_address.clone();
            let port = device.port;

            tokio::spawn(async move {
                if let Err(e) = crate::cluster::http_client::send_pairing_request(
                    &device_id,
                    &device_name,
                    &ip,
                    port,
                )
                .await
                {
                    log::error!("Failed to send pairing request: {}", e);
                }
            });
        }
    }

    log::info!(
        "Device {} status reset from {:?} to None, reachable={}",
        device_id,
        previous_status,
        is_reachable
    );
    Ok(device)
}

#[command]
pub async fn add_device_manually(
    name: String,
    ip_address: String,
    port: i32,
) -> Result<Device, String> {
    // Check if device with same ip:port already exists
    if let Ok(Some(existing)) = device_repo::get_by_ip_and_port(&ip_address, port) {
        return Err(format!(
            "Device with {}:{} already exists ({})",
            ip_address, port, existing.name
        ));
    }

    // Try to ping the device to get real device info
    let (device_id, device_name, version, index_count) =
        match crate::cluster::http_client::ping_device(&ip_address, port).await {
            Ok(info) => (
                info.device_id,
                info.device_name,
                info.version,
                info.index_count,
            ),
            Err(e) => {
                log::warn!(
                    "Failed to ping device at {}:{}, using provided info: {}",
                    ip_address,
                    port,
                    e
                );
                // Use provided name and generate a temporary device ID
                let temp_id = format!("manual_{}", uuid::Uuid::new_v4());
                (temp_id, name.clone(), String::new(), 0)
            }
        };

    // Check if device_id already exists (from another ip:port)
    if let Ok(Some(existing)) = device_repo::get_by_device_id(&device_id) {
        return Err(format!(
            "Device already exists with different address: {} (at {}:{})",
            existing.name, existing.ip_address, existing.port
        ));
    }

    // Create new device
    let device = Device {
        id: 0,
        device_id: device_id.clone(),
        name: device_name,
        ip_address,
        port,
        version,
        online_status: OnlineStatus::Unknown,
        pairing_status: PairingStatus::None,
        pairing_remark: String::new(),
        last_seen: chrono::Local::now(),
        first_discovered: chrono::Local::now(),
        index_count,
        capabilities: "{}".to_string(),
        discovery_method: "manual".to_string(),
        create_time: chrono::Local::now(),
        update_time: chrono::Local::now(),
    };

    // Insert into database
    let inserted_device = device_repo::insert(&device).map_err(|e| e.to_string())?;

    log::info!(
        "Manually added device: {} ({})",
        inserted_device.name,
        inserted_device.device_id
    );
    Ok(inserted_device)
}

// ========== Pairing request commands ==========

#[command]
pub fn load_pairing_requests() -> Result<Vec<crate::entities::PairingRequest>, String> {
    pairing_request_repo::list().map_err(|e| e.to_string())
}

#[command]
pub fn load_pending_pairing_requests() -> Result<Vec<crate::entities::PairingRequest>, String> {
    pairing_request_repo::list_pending().map_err(|e| e.to_string())
}

#[command]
pub fn count_pending_pairing_requests() -> Result<i64, String> {
    device_repo::count_pending_in().map_err(|e| e.to_string())
}

#[command]
pub fn delete_pairing_request(id: i64) -> Result<(), String> {
    pairing_request_repo::delete_by_id(id).map_err(|e| e.to_string())?;
    log::info!("Pairing request {} deleted", id);
    Ok(())
}

#[command]
pub fn delete_pairing_requests(ids: Vec<i64>) -> Result<(), String> {
    for id in &ids {
        pairing_request_repo::delete_by_id(*id).map_err(|e| e.to_string())?;
    }
    log::info!("{} pairing requests deleted", ids.len());
    Ok(())
}

#[command]
pub fn clear_pairing_requests() -> Result<(), String> {
    pairing_request_repo::delete_all().map_err(|e| e.to_string())?;
    log::info!("All pairing requests cleared");
    Ok(())
}

#[command]
pub async fn respond_pairing_request(id: i64, accept: bool) -> Result<(), String> {
    // Get the pairing request
    let request = pairing_request_repo::get_by_id(id).map_err(|e| e.to_string())?;

    // Update request status with remark
    let (status, remark) = if accept {
        (
            crate::enums::PairingRequestStatus::Accepted,
            t!("pairing.remark.user-accepted").to_string(),
        )
    } else {
        (
            crate::enums::PairingRequestStatus::Rejected,
            t!("pairing.remark.user-rejected").to_string(),
        )
    };
    pairing_request_repo::update_status_with_remark(id, status, &remark)
        .map_err(|e| e.to_string())?;

    // Update device pairing status with remark
    if let Ok(Some(device)) = device_repo::get_by_device_id(&request.device_id) {
        let new_status = if accept {
            PairingStatus::Paired
        } else {
            PairingStatus::Rejected
        };
        device_repo::update_pairing_status_with_remark(device.id, new_status, &remark, true)
            .map_err(|e| e.to_string())?;
    }

    // Send response to the requester (both accept and reject)
    // 发送响应给请求方（接受和拒绝都要发送）
    // Get latest ip and port from device table (not from pairing_request table)
    // 从 device 表获取最新的 ip 和 port（而不是从 pairing_request 表）
    let requester_id = request.device_id.clone();
    let requester_device = device_repo::get_by_device_id(&requester_id).ok().flatten();

    tokio::spawn(async move {
        if let Some(device) = requester_device {
            if let Err(e) = crate::cluster::http_client::send_pairing_response(
                &requester_id,
                &device.ip_address,
                device.port,
                accept,
            )
            .await
            {
                log::error!("Failed to send pairing response: {}", e);
            }
        } else {
            log::error!(
                "Failed to send pairing response: device {} not found in device table",
                requester_id
            );
        }
    });

    log::info!(
        "Pairing request {} {}",
        id,
        if accept { "accepted" } else { "rejected" }
    );
    Ok(())
}

#[command]
pub async fn send_pairing_request(
    device_id: String,
    device_name: String,
    ip: String,
    port: i32,
) -> Result<(), String> {
    crate::cluster::http_client::send_pairing_request(&device_id, &device_name, &ip, port).await
}

// ========== Cluster setting commands ==========

#[command]
pub async fn load_cluster_setting() -> Result<ClusterSetting, String> {
    let global_setting = CLUSTER_SETTING
        .get()
        .ok_or("Cluster setting not initialized")?
        .read()
        .await;
    Ok(global_setting.clone())
}

#[command]
pub async fn update_cluster_setting(setting: ClusterSetting) -> Result<(), String> {
    cluster::save_cluster_setting(&setting).await
}

#[command]
pub async fn toggle_cluster(start: bool) -> Result<(), String> {
    if start {
        cluster::start_cluster_service().await?;
    } else {
        cluster::stop_cluster_service().await?;
    }
    Ok(())
}

/// Check online status of existing devices only (lightweight, no mDNS restart)
/// 仅检查现有设备的在线状态（轻量级，不重启 mDNS）
#[command]
pub async fn check_devices_status() -> Result<(), String> {
    cluster::device_checker::trigger_status_check()
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn check_devices() -> Result<(), String> {
    // 1. Restart mDNS browsing to trigger fresh discovery
    cluster::mdns_service::stop_mdns_browsing().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    if let Err(e) = cluster::mdns_service::start_mdns_browsing().await {
        log::error!("Failed to restart mDNS browsing: {}", e);
    }

    // 2. Check online status of existing devices
    cluster::device_checker::trigger_status_check()
        .await
        .map_err(|e| e.to_string())
}

// ========== Test mode commands ==========

/// Mock device definitions for testing
/// 模拟设备定义，用于测试
const MOCK_DEVICES: &[(&str, &str, i32, i64)] = &[
    ("mock-remote-device-001", "模拟设备-文档", 15678, 1250),
    ("mock-remote-device-002", "模拟设备-图片", 15679, 3420),
    ("mock-remote-device-003", "模拟设备-音频", 15680, 856),
    ("mock-remote-device-004", "模拟设备-视频", 15681, 432),
    ("mock-remote-device-005", "模拟设备-混合", 15682, 2156),
];

/// Toggle test mode for remote device simulation
/// When enabled, adds mock devices to database if not exist, and remote device search returns mock data
/// 启用时会自动添加模拟设备到数据库（如不存在），搜索返回模拟数据
#[command]
pub fn toggle_test_mode(enabled: bool) -> Result<bool, String> {
    crate::global::TEST_MODE_REMOTE_DEVICE.store(enabled, std::sync::atomic::Ordering::Relaxed);
    log::info!("Test mode for remote device: {}", enabled);

    if enabled {
        // Add mock devices if not exist
        // 如果不存在则添加模拟设备
        for (device_id, name, port, index_count) in MOCK_DEVICES {
            if device_repo::get_by_device_id(device_id)
                .unwrap_or(None)
                .is_none()
            {
                let device = Device {
                    id: 0,
                    device_id: device_id.to_string(),
                    name: name.to_string(),
                    ip_address: "127.0.0.1".to_string(),
                    port: *port,
                    version: "1.0.0-mock".to_string(),
                    online_status: OnlineStatus::Online,
                    pairing_status: PairingStatus::Paired,
                    pairing_remark: String::new(),
                    last_seen: chrono::Local::now(),
                    first_discovered: chrono::Local::now(),
                    index_count: *index_count,
                    capabilities: "{}".to_string(),
                    discovery_method: "mock".to_string(),
                    create_time: chrono::Local::now(),
                    update_time: chrono::Local::now(),
                };

                match device_repo::insert(&device) {
                    Ok(inserted) => {
                        log::info!(
                            "Added mock device: {} ({})",
                            inserted.name,
                            inserted.device_id
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to add mock device {}: {}", device_id, e);
                    }
                }
            }
        }
    } else {
        // Remove mock devices when disabled
        // 禁用时删除模拟设备
        for (device_id, _, _, _) in MOCK_DEVICES {
            match device_repo::delete_by_device_id(device_id) {
                Ok(_) => {
                    log::info!("Removed mock device: {}", device_id);
                }
                Err(e) => {
                    log::warn!("Failed to remove mock device {}: {}", device_id, e);
                }
            }
        }
    }

    Ok(enabled)
}

/// Get current test mode status
#[command]
pub fn get_test_mode() -> Result<bool, String> {
    Ok(crate::global::TEST_MODE_REMOTE_DEVICE.load(std::sync::atomic::Ordering::Relaxed))
}
