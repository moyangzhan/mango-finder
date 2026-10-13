//! Device checker module
//! 设备检测模块
//!
//! This module contains various device status checkers:
//! - Online status checker (via HTTP ping)
//! - Pairing request expiration checker (hourly)
//! - Future: capability checker, sync checker, etc.
//!
//! 此模块包含各种设备状态检测器：
//! - 在线状态检测器（通过 HTTP ping）
//! - 配对请求过期检测器（每小时）
//! - 未来：能力检测器、同步检测器等

use crate::entities::Device;
use crate::enums::OnlineStatus;
use crate::repositories::{device_repo, pairing_request_repo};
use crate::structs::cluster_config::DeviceInfoResponse;
use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Emitter;

/// Wrapper for /ping response (nested format: { "code": 0, "data": {...} })
#[derive(Debug, Deserialize)]
struct PingResponse {
    code: i32,
    data: DeviceInfoResponse,
}

/// Online checker running state
static ONLINE_CHECKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Shutdown signal for online checker
static ONLINE_CHECKER_SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Expiration checker running state
static EXPIRATION_CHECKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Shutdown signal for expiration checker
static EXPIRATION_CHECKER_SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Expiration check interval in seconds (1 hour)
/// 过期检查间隔（1小时）
const EXPIRATION_CHECK_INTERVAL_SECS: u64 = 60 * 60;

// ============================================
// Public API - Start/Stop all checkers
// ============================================

/// Start all device checkers
/// 启动所有设备检测器
pub async fn start_status_checker(online_check_interval_secs: i32) {
    // Use compare_exchange to atomically check and set the running flag
    // This prevents race condition when multiple tasks call this function concurrently
    let mut retries = 5;
    while ONLINE_CHECKER_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        retries -= 1;
        if retries == 0 {
            log::warn!("Device checker start timeout, forcing start");
            break;
        }
        // Another instance is running, wait for it to stop
        log::info!("Device checker is already running, waiting for it to stop");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Reset shutdown flag before starting
    ONLINE_CHECKER_SHUTDOWN.store(false, Ordering::SeqCst);

    // Start online timer (RUNNING flag is already set)
    start_online_timer_inner(online_check_interval_secs).await;

    // Start expiration timer
    start_expiration_timer().await;
}

/// Stop all device checkers
/// 停止所有设备检测器
pub async fn stop_status_checker() {
    stop_online_timer().await;
    stop_expiration_timer().await;
}

/// Check if any checker is running
pub fn is_checker_running() -> bool {
    ONLINE_CHECKER_RUNNING.load(Ordering::SeqCst)
        || EXPIRATION_CHECKER_RUNNING.load(Ordering::SeqCst)
}

// ============================================
// Online Status Timer
// ============================================

/// Start the online status timer (internal, RUNNING flag must be set by caller)
/// 启动在线状态定时检测（内部函数，RUNNING 标志必须由调用者设置）
async fn start_online_timer_inner(check_interval_secs: i32) {
    log::info!(
        "Device online timer started (interval: {}s)",
        check_interval_secs
    );

    tokio::spawn(async move {
        // Check immediately on startup
        if let Err(e) = check_all_devices_online().await {
            log::error!("Error checking device online status on startup: {}", e);
        }

        let mut interval = tokio::time::interval(Duration::from_secs(check_interval_secs as u64));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = check_all_devices_online().await {
                        log::error!("Error checking device online status: {}", e);
                    }
                }
                _ = async {
                    while !ONLINE_CHECKER_SHUTDOWN.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                } => {
                    log::info!("Device online timer received shutdown signal");
                    break;
                }
            }
        }

        ONLINE_CHECKER_RUNNING.store(false, Ordering::SeqCst);
        log::info!("Device online timer stopped");
    });
}

/// Stop the online status timer
/// 停止在线状态定时检测
async fn stop_online_timer() {
    if !ONLINE_CHECKER_RUNNING.load(Ordering::SeqCst) {
        return;
    }

    ONLINE_CHECKER_SHUTDOWN.store(true, Ordering::SeqCst);

    // Wait for checker to stop
    let mut retries = 0;
    while ONLINE_CHECKER_RUNNING.load(Ordering::SeqCst) && retries < 10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        retries += 1;
    }

    log::info!("Device online timer stop requested");
}

// ============================================
// Expiration Timer
// ============================================

/// Start the pairing request expiration timer (runs every hour)
/// 启动配对请求过期定时器（每小时运行一次）
async fn start_expiration_timer() {
    EXPIRATION_CHECKER_SHUTDOWN.store(false, Ordering::SeqCst);
    EXPIRATION_CHECKER_RUNNING.store(true, Ordering::SeqCst);

    log::info!(
        "Pairing request expiration timer started (interval: {}s)",
        EXPIRATION_CHECK_INTERVAL_SECS
    );

    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(EXPIRATION_CHECK_INTERVAL_SECS));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Ok(expired_count) = pairing_request_repo::expire_old_requests() {
                        if expired_count > 0 {
                            log::info!("Expired {} old pairing requests", expired_count);
                        }
                    }
                }
                _ = async {
                    while !EXPIRATION_CHECKER_SHUTDOWN.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                } => {
                    log::info!("Expiration timer received shutdown signal");
                    break;
                }
            }
        }

        EXPIRATION_CHECKER_RUNNING.store(false, Ordering::SeqCst);
        log::info!("Pairing request expiration timer stopped");
    });
}

/// Stop the expiration timer
/// 停止过期定时器
async fn stop_expiration_timer() {
    if !EXPIRATION_CHECKER_RUNNING.load(Ordering::SeqCst) {
        return;
    }

    EXPIRATION_CHECKER_SHUTDOWN.store(true, Ordering::SeqCst);

    // Wait for checker to stop
    let mut retries = 0;
    while EXPIRATION_CHECKER_RUNNING.load(Ordering::SeqCst) && retries < 10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        retries += 1;
    }

    log::info!("Expiration timer stop requested");
}

/// Manually trigger an online status check for all devices
/// 手动触发所有设备的在线状态检测
pub async fn trigger_status_check() -> Result<()> {
    log::info!("Manually triggering device online status check");
    check_all_devices_online().await
}

/// Manually trigger pairing request expiration check
/// 手动触发配对请求过期检查
pub fn trigger_expiration_check() -> Result<usize, String> {
    log::info!("Manually triggering pairing request expiration check");
    pairing_request_repo::expire_old_requests().map_err(|e| e.to_string())
}

/// Check online status of all devices
/// 检查所有设备的在线状态
async fn check_all_devices_online() -> Result<()> {
    // Get all devices
    let devices = device_repo::list().map_err(|e| anyhow!("Failed to get devices: {}", e))?;

    if devices.is_empty() {
        return Ok(());
    }

    log::debug!("Checking online status of {} devices", devices.len());

    // Check each device concurrently
    let futures: Vec<_> = devices
        .into_iter()
        .map(|device| verify_device_online(device))
        .collect();

    let results = futures::future::join_all(futures).await;

    // Log any errors
    for result in results {
        if let Err(e) = result {
            log::warn!("Device online check error: {}", e);
        }
    }

    Ok(())
}

/// Verify if a device is online by sending HTTP request
/// 通过 HTTP 请求验证设备是否在线
///
/// NOTE: This function only updates `online_status`, `name`, `index_count`, `capabilities`, and `last_seen`.
/// It does NOT update `pairing_status`. Pairing status should only be changed through:
/// 1. HTTP pairing endpoints (pairing request/response)
/// 2. User manual operations via frontend
/// 注意：此函数只更新 online_status、name、index_count、capabilities 和 last_seen。
/// 不更新 pairing_status。配对状态只能通过以下方式更改：
/// 1. HTTP 配对端点（配对请求/响应）
/// 2. 用户通过前端手动操作
async fn verify_device_online(device: Device) -> Result<()> {
    let old_status = device.online_status;
    let url = format!("http://{}:{}/ping", device.ip_address, device.port);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

    // Try request with one retry on failure
    let response = client.get(&url).send().await;
    let response = match response {
        Ok(resp) => Ok(resp),
        Err(e) => {
            // Retry once on failure
            log::debug!("Device {} request failed, retrying: {}", device.name, e);
            tokio::time::sleep(Duration::from_secs(1)).await;
            client.get(&url).send().await
        }
    };

    let new_status = match response {
        Ok(resp) if resp.status().is_success() => {
            // Parse response and update device info
            match resp.json::<PingResponse>().await {
                Ok(ping_resp) => {
                    let info = ping_resp.data;
                    // Update device with latest info
                    device_repo::update_device_info(
                        &device.device_id,
                        &info.device_name,
                        info.index_count,
                        &info.capabilities.to_json_string(),
                    )
                    .map_err(|e| anyhow!("Failed to update device info: {}", e))?;

                    // Update online status to Online
                    device_repo::update_online_status(device.id, OnlineStatus::Online)
                        .map_err(|e| anyhow!("Failed to update device status: {}", e))?;

                    log::debug!(
                        "Device {} is online (index_count: {})",
                        device.name,
                        info.index_count
                    );
                }
                Err(e) => {
                    // Response received but failed to parse - still mark as online
                    log::warn!(
                        "Failed to parse device info response from {}: {}",
                        device.name,
                        e
                    );
                    device_repo::update_online_status(device.id, OnlineStatus::Online)
                        .map_err(|e| anyhow!("Failed to update device status: {}", e))?;
                }
            }
            OnlineStatus::Online
        }
        Ok(resp) => {
            // Non-success status code (e.g., 403 Forbidden)
            // Device is still online (we got a response), but may have auth issues
            log::warn!(
                "Device {} returned status {} (device is online but request was rejected)",
                device.name,
                resp.status()
            );
            device_repo::update_online_status(device.id, OnlineStatus::Online)
                .map_err(|e| anyhow!("Failed to update device status: {}", e))?;
            OnlineStatus::Online
        }
        Err(e) => {
            // Request failed - device is offline
            log::debug!("Device {} is offline: {}", device.name, e);
            device_repo::update_online_status(device.id, OnlineStatus::Offline)
                .map_err(|e| anyhow!("Failed to update device status: {}", e))?;
            OnlineStatus::Offline
        }
    };

    // Emit event if status changed
    if old_status != new_status {
        log::info!(
            "Device {} status changed: {:?} -> {:?}",
            device.name,
            old_status,
            new_status
        );
        if let Some(app_handle) = crate::global::APP_HANDLE.get() {
            match new_status {
                OnlineStatus::Online => {
                    let _ = app_handle.emit("device-online", &device.device_id);
                }
                OnlineStatus::Offline => {
                    let _ = app_handle.emit("device-offline", &device.device_id);
                }
                _ => {}
            }
        }
    }

    Ok(())
}
