//! Remote HTTP client for inter-device communication
//!
//! This module provides HTTP request functions for communicating with remote devices.

use std::sync::LazyLock;
use std::time::Duration;

use reqwest::Client;
use rust_i18n::t;
use tauri::Emitter;

use crate::cluster::api_types::{
    CODE_SUCCESS, ChunksData, CommonResponse, FileContentData, FindSimilarsData, SearchData,
};
use crate::entities::PairingRequest;
use crate::enums::{PairingRequestStatus, PairingResponseStatus, PairingStatus};
use crate::global::{APP_HANDLE, CLIENT_ID};
use crate::repositories::{device_repo, pairing_request_repo};
use crate::structs::cluster_config::{
    DeviceInfoResponse, PairingRequestPayload, PairingResponsePayload, RemoteFindSimilarsRequest,
    RemoteSearchRequest, ResetNotifyPayload,
};

/// HTTP timeout for remote requests (seconds)
pub const HTTP_TIMEOUT_SECS: u64 = 10;

/// Max results per device for search
pub const MAX_RESULTS_PER_DEVICE: usize = 50;

/// Global HTTP client for remote device communication
/// 全局 HTTP 客户端，用于远程设备通信
static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| Client::new())
});

/// Build a /cluster endpoint URL for a remote device; `path` begins with `/`
pub fn cluster_url(ip: &str, port: i32, path: &str) -> String {
    format!("http://{}:{}{}{}", ip, port, super::CLUSTER_PREFIX, path)
}

// ============================================ Internal helpers
// ============================================

/// 统一请求前置:带设备头发送 + 状态码校验,错误串全局一致
/// Shared request prologue: send with the device header, check the status
async fn send_with_device_id(
    method: reqwest::Method,
    url: &str,
    body: Option<&impl serde::Serialize>,
) -> Result<reqwest::Response, String> {
    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let mut request = HTTP_CLIENT
        .request(method, url)
        .header("X-Device-ID", client_id);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }
    Ok(response)
}

/// 发起调用并取回 CommonResponse.data:解析 → code 校验 → 解包
/// Make the call and return CommonResponse.data: parse → check code → extract
async fn call_and_get_data<T: serde::de::DeserializeOwned>(
    method: reqwest::Method,
    url: &str,
    body: Option<&impl serde::Serialize>,
) -> Result<T, String> {
    let response = send_with_device_id(method, url, body).await?;
    let common: CommonResponse<T> = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;
    if common.code != CODE_SUCCESS {
        return Err(format!("API error: code {} - {}", common.code, common.msg));
    }
    common.data.ok_or_else(|| "No data in response".to_string())
}

// ============================================ Search API
// ============================================

/// Search on a remote device
/// 在远程设备上搜索
pub async fn search(
    query: &str,
    search_type: &str,
    ip: &str,
    port: i32,
) -> Result<SearchData, String> {
    let url = cluster_url(ip, port, "/search");
    let request = RemoteSearchRequest {
        query: query.to_string(),
        search_type: search_type.to_string(),
        limit: MAX_RESULTS_PER_DEVICE,
    };
    call_and_get_data(reqwest::Method::POST, &url, Some(&request)).await
}

// ============================================ Similarity Search API
// ============================================

/// Find similar files on a remote device by file ID (with exclude list for loop prevention)
/// 通过文件ID在远程设备上查找相似文件（带排除列表防止循环）
///
/// This calls the /cluster/file/:file_id/similars endpoint which handles cross-device search on the remote device side, including forwarding to other devices.
///
/// # Arguments
/// * `request_id` - Unique request ID for loop prevention. If None, generates a new UUID.
pub async fn find_similars(
    file_id: i64,
    request_id: Option<&str>,
    exclude_device_ids: &[String],
    limit: usize,
    ip: &str,
    port: i32,
) -> Result<FindSimilarsData, String> {
    let url = cluster_url(ip, port, &format!("/file/{}/similars", file_id));

    // Generate request_id if not provided
    let request_id = match request_id {
        Some(id) => id.to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };

    let request = RemoteFindSimilarsRequest {
        request_id: request_id.clone(),
        exclude_device_ids: exclude_device_ids.to_vec(),
        limit,
    };

    log::info!(
        "find_similars: url={}, request_id={}, exclude={:?}, limit={}",
        url,
        request_id,
        exclude_device_ids,
        limit
    );

    call_and_get_data(reqwest::Method::POST, &url, Some(&request)).await
}

// ============================================ File Fetch API
// ============================================

/// Fetch file data from a remote device
/// 从远程设备获取文件数据
pub async fn fetch_file(file_id: i64, ip: &str, port: i32) -> Result<Vec<u8>, String> {
    let url = cluster_url(ip, port, &format!("/file/{}", file_id));
    let response =
        send_with_device_id(reqwest::Method::GET, &url, None::<&serde_json::Value>).await?;
    let data = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    Ok(data.to_vec())
}

// ============================================ Chunks API
// ============================================

/// Fetch chunks from a remote device by IDs
/// 从远程设备获取文本片段
pub async fn fetch_chunks(ids: &[u32], ip: &str, port: i32) -> Result<Vec<String>, String> {
    let url = cluster_url(ip, port, "/chunks");
    call_and_get_data::<ChunksData>(
        reqwest::Method::POST,
        &url,
        Some(&serde_json::json!({ "ids": ids })),
    )
    .await
    .map(|d| d.chunks)
}

// ============================================ File Content API
// ============================================

/// Fetch file content from a remote device by file ID
/// 从远程设备获取文件内容
pub async fn fetch_file_content(
    file_id: i64,
    ip: &str,
    port: i32,
) -> Result<FileContentData, String> {
    let url = cluster_url(ip, port, &format!("/file/{}/content", file_id));
    call_and_get_data(reqwest::Method::GET, &url, None::<&serde_json::Value>).await
}

// ============================================ Utility functions
// ============================================

/// Get local IP address
pub fn get_local_ip() -> Result<String, String> {
    use local_ip_address::local_ip;

    local_ip()
        .map(|ip| ip.to_string())
        .map_err(|e| format!("Failed to get local IP: {}", e))
}

// ============================================ Device Ping API
// ============================================

/// Ping a remote device to check status
pub async fn ping_device(ip: &str, port: i32) -> Result<DeviceInfoResponse, String> {
    let url = format!("http://{}:{}/ping", ip, port);

    let response = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Ping failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Ping failed: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse ping response: {}", e))?;

    let data = json.get("data").ok_or("Invalid ping response")?;

    // Parse capabilities from response
    let capabilities = data
        .get("capabilities")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_default();

    Ok(DeviceInfoResponse {
        device_id: data
            .get("deviceId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        device_name: data
            .get("deviceName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        version: data
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        index_count: data.get("indexCount").and_then(|v| v.as_i64()).unwrap_or(0),
        last_index_time: data
            .get("lastIndexTime")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        capabilities,
    })
}

/// Ping a remote device to check status and update device info in database
/// Ping 远程设备并更新数据库中的设备信息
pub async fn ping_and_update_device_info(
    device: &mut crate::entities::Device,
) -> Result<DeviceInfoResponse, String> {
    let info = ping_device(&device.ip_address, device.port).await?;

    // Update device info from response
    device.name = info.device_name.clone();
    device.version = info.version.clone();
    device.index_count = info.index_count;
    device.capabilities = info.capabilities.to_json_string();

    // Also update device info in database
    let _ = device_repo::update_device_info(
        &device.device_id,
        &info.device_name,
        info.index_count,
        &info.capabilities.to_json_string(),
    );

    Ok(info)
}

// ============================================ Pairing API
// ============================================

/// Send pairing request to a remote device
pub async fn send_pairing_request(
    device_id: &str,
    device_name: &str,
    ip: &str,
    port: i32,
) -> Result<(), String> {
    let my_device_id = crate::read_lock!(CLIENT_ID).clone();
    let my_device_name = super::get_cluster_setting().await.device_name;
    let my_device_name = if my_device_name.is_empty() {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "MangoFinder".to_string())
    } else {
        my_device_name
    };

    let my_ip = get_local_ip()?;
    let my_port = super::get_cluster_setting().await.port;

    let payload = PairingRequestPayload {
        device_id: my_device_id,
        device_name: my_device_name,
        ip_address: my_ip,
        port: my_port,
    };

    let url = cluster_url(ip, port, "/pairing/request");

    let response = HTTP_CLIENT
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send pairing request: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Pairing request failed: {}", response.status()));
    }

    // Parse response to check if auto-approved
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let status_str = json
        .get("data")
        .and_then(|d| d.get("status"))
        .and_then(|s| s.as_str())
        .unwrap_or("");

    let response_status = PairingResponseStatus::from_str(status_str);

    // Log the outgoing request with appropriate remark
    let (request_status, request_remark) = match response_status {
        PairingResponseStatus::Approved | PairingResponseStatus::AlreadyPaired => (
            PairingRequestStatus::Accepted,
            t!("pairing.remark.remote-accepted").to_string(),
        ),
        PairingResponseStatus::Rejected => (
            PairingRequestStatus::Rejected,
            t!("pairing.remark.remote-rejected").to_string(),
        ),
        _ => (
            PairingRequestStatus::Pending,
            t!("pairing.remark.request-sent").to_string(),
        ),
    };

    let request = PairingRequest {
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        ip_address: ip.to_string(),
        port,
        direction: "out".to_string(),
        status: request_status,
        remark: request_remark,
        ..Default::default()
    };
    let _ = pairing_request_repo::insert(&request);

    // Notify frontend to update pairing request list
    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit("pairing-request-sent", &request);
    }

    // Update device status based on response (automatic process, not manual)
    if let Ok(Some(device)) = device_repo::get_by_device_id(device_id) {
        match response_status {
            PairingResponseStatus::Approved | PairingResponseStatus::AlreadyPaired => {
                let remark = t!("pairing.remark.remote-accepted").to_string();
                let _ = device_repo::update_pairing_status_with_remark(
                    device.id,
                    PairingStatus::Paired,
                    &remark,
                    false,
                );
                log::info!("Pairing auto-approved by {} ({})", device_name, ip);
            }
            PairingResponseStatus::Rejected => {
                let remark = t!("pairing.remark.remote-rejected").to_string();
                let _ = device_repo::update_pairing_status_with_remark(
                    device.id,
                    PairingStatus::Blocked,
                    &remark,
                    false,
                );
                log::info!("Pairing rejected by {} ({})", device_name, ip);
            }
            _ => {
                let remark = t!("pairing.remark.request-sent").to_string();
                let _ = device_repo::update_pairing_status_with_remark(
                    device.id,
                    PairingStatus::PendingOut,
                    &remark,
                    false,
                );
                log::info!(
                    "Pairing request sent to {} ({}), waiting for approval",
                    device_name,
                    ip
                );
            }
        }
    }

    Ok(())
}

/// Send pairing response to a remote device
pub async fn send_pairing_response(
    requester_id: &str,
    requester_ip: &str,
    requester_port: i32,
    approved: bool,
) -> Result<(), String> {
    let my_device_id = crate::read_lock!(CLIENT_ID).clone();

    let payload = PairingResponsePayload {
        requester_id: requester_id.to_string(),
        responder_id: my_device_id,
        approved,
    };

    let url = cluster_url(requester_ip, requester_port, "/pairing/respond");

    let response = HTTP_CLIENT
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send pairing response: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Pairing response failed: {}", response.status()));
    }

    log::info!(
        "Pairing response sent to {}: approved={}",
        requester_id,
        approved
    );
    Ok(())
}

/// Send reset notify to a remote device
/// 发送重置通知给远程设备
///
/// Called when local device resets its pairing status, to notify the remote device
/// 当本机设备重置配对状态时调用，通知远程设备
pub async fn send_reset_notify(
    remote_device_id: &str,
    remote_ip: &str,
    remote_port: i32,
    previous_status: &str,
) -> Result<(), String> {
    let my_device_id = crate::read_lock!(CLIENT_ID).clone();

    let payload = ResetNotifyPayload {
        from_device_id: my_device_id,
        previous_status: previous_status.to_string(),
    };

    let url = cluster_url(remote_ip, remote_port, "/pairing/reset_notify");

    let response = HTTP_CLIENT
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send reset notify: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Reset notify failed: {}", response.status()));
    }

    log::info!(
        "Reset notify sent to {} (previous status: {})",
        remote_device_id,
        previous_status
    );
    Ok(())
}
