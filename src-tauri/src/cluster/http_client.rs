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

// ============================================
// Search API
// ============================================

/// Search on a remote device
/// 在远程设备上搜索
pub async fn search(
    query: &str,
    search_type: &str,
    ip: &str,
    port: i32,
) -> Result<SearchData, String> {
    let url = format!("http://{}:{}/cluster/search", ip, port);

    let request = RemoteSearchRequest {
        query: query.to_string(),
        search_type: search_type.to_string(),
        limit: MAX_RESULTS_PER_DEVICE,
    };

    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let response = HTTP_CLIENT
        .post(&url)
        .header("X-Device-ID", client_id)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let search_response: CommonResponse<SearchData> = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;

    if search_response.code != CODE_SUCCESS {
        return Err(format!(
            "API error: code {} - {}",
            search_response.code, search_response.msg
        ));
    }

    search_response
        .data
        .ok_or_else(|| "No data in response".to_string())
}

// ============================================
// Similarity Search API
// ============================================

/// Audio fingerprint for music similarity
/// 音乐相似性的音频指纹
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioFingerprint {
    pub spectral_histogram: Vec<f32>,
    pub energy_bands: Vec<f32>,
    pub avg_zcr: f32,
    pub tempo_estimate: f32,
}

impl From<&crate::utils::audio_util::MusicFingerprint> for AudioFingerprint {
    fn from(fp: &crate::utils::audio_util::MusicFingerprint) -> Self {
        AudioFingerprint {
            spectral_histogram: fp.spectral_histogram.to_vec(),
            energy_bands: fp.energy_bands.to_vec(),
            avg_zcr: fp.avg_zcr,
            tempo_estimate: fp.tempo_estimate,
        }
    }
}

impl From<&AudioFingerprint> for crate::utils::audio_util::MusicFingerprint {
    fn from(fp: &AudioFingerprint) -> Self {
        let mut spectral_histogram = [0.0f32; 10];
        let mut energy_bands = [0.0f32; 8];
        for (i, &v) in fp.spectral_histogram.iter().enumerate().take(10) {
            spectral_histogram[i] = v;
        }
        for (i, &v) in fp.energy_bands.iter().enumerate().take(8) {
            energy_bands[i] = v;
        }
        crate::utils::audio_util::MusicFingerprint {
            spectral_histogram,
            energy_bands,
            avg_zcr: fp.avg_zcr,
            tempo_estimate: fp.tempo_estimate,
        }
    }
}

/// Features for similarity search
/// 相似搜索特征
#[derive(Debug, Clone)]
pub struct SimilarFeatures {
    pub category: i64,
    pub image_hash: Option<String>, // base64 encoded
    pub embedding: Option<Vec<f32>>,
    pub sparse_indices: Option<Vec<u32>>,
    pub sparse_values: Option<Vec<f32>>,
    pub audio_type: crate::structs::file_metadata::AudioType,
    pub audio_fingerprint: Option<AudioFingerprint>, // For music similarity
}

/// Find similar files on a remote device by file ID (with exclude list for loop prevention)
/// 通过文件ID在远程设备上查找相似文件（带排除列表防止循环）
///
/// This calls the /file/:file_id/cluster_similars endpoint which handles cross-device search
/// on the remote device side, including forwarding to other devices.
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
    let url = format!("http://{}:{}/cluster/file/{}/similars", ip, port, file_id);

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

    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let response = HTTP_CLIENT
        .post(&url)
        .header("X-Device-ID", client_id)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let similar_response: CommonResponse<FindSimilarsData> = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;

    if similar_response.code != CODE_SUCCESS {
        return Err(format!(
            "API error: code {} - {}",
            similar_response.code, similar_response.msg
        ));
    }

    similar_response
        .data
        .ok_or_else(|| "No data in response".to_string())
}

// ============================================
// File Fetch API
// ============================================

/// Fetch file data from a remote device
/// 从远程设备获取文件数据
pub async fn fetch_file(file_id: i64, ip: &str, port: i32) -> Result<Vec<u8>, String> {
    let url = format!("http://{}:{}/cluster/file/{}", ip, port, file_id);

    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let response = HTTP_CLIENT
        .get(&url)
        .header("X-Device-ID", client_id)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let data = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    Ok(data.to_vec())
}

// ============================================
// Chunks API
// ============================================

/// Fetch chunks from a remote device by IDs
/// 从远程设备获取文本片段
pub async fn fetch_chunks(ids: &[u32], ip: &str, port: i32) -> Result<Vec<String>, String> {
    let url = format!("http://{}:{}/cluster/chunks", ip, port);

    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let response = HTTP_CLIENT
        .post(&url)
        .header("X-Device-ID", client_id)
        .json(&serde_json::json!({ "ids": ids }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let chunks_response: CommonResponse<ChunksData> = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;

    if chunks_response.code != CODE_SUCCESS {
        return Err(format!(
            "API error: code {} - {}",
            chunks_response.code, chunks_response.msg
        ));
    }

    chunks_response
        .data
        .map(|d| d.chunks)
        .ok_or_else(|| "No data in response".to_string())
}

// ============================================
// File Content API
// ============================================

/// Fetch file content from a remote device by file ID
/// 从远程设备获取文件内容
pub async fn fetch_file_content(
    file_id: i64,
    ip: &str,
    port: i32,
) -> Result<FileContentData, String> {
    let url = format!("http://{}:{}/cluster/file/{}/content", ip, port, file_id);

    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let response = HTTP_CLIENT
        .get(&url)
        .header("X-Device-ID", client_id)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let content_response: CommonResponse<FileContentData> = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;

    if content_response.code != CODE_SUCCESS {
        return Err(format!(
            "API error: code {} - {}",
            content_response.code, content_response.msg
        ));
    }

    content_response
        .data
        .ok_or_else(|| "No data in response".to_string())
}

// ============================================
// Utility functions - 工具函数
// ============================================

/// Get local IP address
pub fn get_local_ip() -> Result<String, String> {
    use local_ip_address::local_ip;

    local_ip()
        .map(|ip| ip.to_string())
        .map_err(|e| format!("Failed to get local IP: {}", e))
}

// ============================================
// Device Ping API - 设备 Ping 接口
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

// ============================================
// Pairing API - 配对接口
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

    let url = format!("http://{}:{}/cluster/pairing/request", ip, port);

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

    let url = format!(
        "http://{}:{}/cluster/pairing/respond",
        requester_ip, requester_port
    );

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

    let url = format!(
        "http://{}:{}/cluster/pairing/reset_notify",
        remote_ip, remote_port
    );

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
