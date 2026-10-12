use crate::enums::{OnlineStatus, PairingStatus};
use crate::global::CLIENT_ID;
use crate::repositories::device_repo;
use crate::searcher;
use crate::similarity::similarity_service;
use crate::structs::search_result::SearchResult;
use serde::Serialize;
use tauri::command;

/// Search device info for UI
#[derive(Serialize)]
pub struct SearchDevice {
    pub device_id: String,
    pub device_name: String,
    pub is_local: bool,
    pub online_status: String,
    pub index_count: i64,
}

/// Local device search result
/// 本地设备搜索结果
#[derive(Debug, Clone, Serialize)]
pub struct LocalDeviceSearchResult {
    pub results: Vec<SearchResult>,
    pub total: usize,
}

/// Search local device only
/// 仅搜索本地设备
#[command]
pub async fn local_device_search(
    query: &str,
    search_type: &str,
) -> Result<LocalDeviceSearchResult, String> {
    // 与 CLI/daemon 同源分发(searcher::search_by_type),三处行为保持一致
    // Same dispatch source as CLI/daemon (searcher::search_by_type)
    let results = searcher::search_by_type(query, Some(search_type)).await;
    let total = results.len();
    Ok(LocalDeviceSearchResult { results, total })
}

#[command]
pub async fn remote_device_search(
    query: &str,
    search_type: &str,
    device_ids: Option<Vec<String>>,
) -> Result<crate::cluster::cluster_search::RemoteDeviceSearchResult, String> {
    let result =
        crate::cluster::cluster_search::remote_device_search(query, search_type, device_ids).await;
    Ok(result)
}

#[command]
pub async fn list_online_devices() -> Result<Vec<SearchDevice>, String> {
    let mut devices = Vec::new();

    // Add local device
    let client_id = crate::read_lock!(CLIENT_ID).clone();
    let device_name = crate::cluster::get_cluster_setting().await.device_name;
    let local_name = if device_name.is_empty() {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Local".to_string())
    } else {
        device_name
    };
    let index_count = crate::repositories::file_info_repo::count().unwrap_or(0);

    devices.push(SearchDevice {
        device_id: client_id,
        device_name: local_name,
        is_local: true,
        online_status: "online".to_string(),
        index_count,
    });

    // Add paired online devices only
    let remote_devices = device_repo::list().map_err(|e| e.to_string())?;

    for d in remote_devices {
        if d.pairing_status == PairingStatus::Paired && d.online_status == OnlineStatus::Online {
            devices.push(SearchDevice {
                device_id: d.device_id,
                device_name: d.name,
                is_local: false,
                online_status: d.online_status.to_string(),
                index_count: d.index_count,
            });
        }
    }

    Ok(devices)
}

/// Fetch file from remote device by file ID
/// Only indexed files can be fetched (security measure)
#[command]
pub async fn fetch_remote_file(device_id: String, file_id: i64) -> Result<Vec<u8>, String> {
    use reqwest::Client;

    // Get device info
    let device = device_repo::get_by_device_id(&device_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Device not found".to_string())?;

    if device.pairing_status != PairingStatus::Paired {
        return Err("Device is not paired".to_string());
    }

    // Build URL with file_id
    let url = format!(
        "http://{}:{}/cluster/file/{}",
        device.ip_address, device.port, file_id
    );

    log::info!(
        "Fetching remote file from {}: file_id={}",
        device.name,
        file_id
    );

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new());

    let device_id = crate::read_lock!(CLIENT_ID).clone();
    let response = client
        .get(&url)
        .header("X-Device-ID", device_id)
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

#[command]
pub async fn find_similars_by_file_id(
    file_id: i64,
    source_device_id: Option<String>,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    match source_device_id {
        Some(device_id) => {
            similarity_service::find_similars_for_remote_file(&device_id, file_id, limit).await
        }
        None => similarity_service::find_similars_for_local_file(file_id, limit).await,
    }
}
