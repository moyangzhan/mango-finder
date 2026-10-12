//! Remote device similarity search service
//!
//! This module provides functionality for finding similar files across remote devices.
//! It handles device discovery, network communication, and result aggregation.

use tokio::task::JoinSet;

use crate::cluster::http_client;
use crate::entities::{Device, FileInfo};
use crate::enums::{OnlineStatus, PairingStatus};
use crate::repositories::device_repo;
use crate::structs::search_result::{SearchResult, SourceDevice};

/// Find similar files across remote devices
/// 在远程设备上查找相似文件
pub async fn remote_find_similars(file_info: &FileInfo, limit: usize) -> Vec<SearchResult> {
    // Generate a new request_id for this initial search
    let request_id = uuid::Uuid::new_v4().to_string();
    remote_find_similars_with_exclude(file_info, limit, &[], &request_id).await
}

/// Find similar files across remote devices with exclude list (to prevent loops)
/// 在远程设备上查找相似文件（带排除列表防止循环）
pub async fn remote_find_similars_with_exclude(
    file_info: &FileInfo,
    limit: usize,
    exclude_device_ids: &[String],
    request_id: &str,
) -> Vec<SearchResult> {
    // Check test mode
    if crate::global::TEST_MODE_REMOTE_DEVICE.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::cluster::mock::generate_mock_similar_results(file_info, limit);
    }

    // Get searchable devices (paired and online), excluding specified devices
    let devices = get_searchable_devices_excluding(exclude_device_ids).await;

    if devices.is_empty() {
        return Vec::new();
    }

    // Build exclude list for forwarding (add ourselves)
    let mut forward_exclude = exclude_device_ids.to_vec();
    forward_exclude.push(crate::read_lock!(crate::global::CLIENT_ID).clone());

    // Execute remote similar searches concurrently
    let mut join_set = JoinSet::new();

    for device in devices {
        let device_id = device.device_id;
        let device_name = device.name;
        let ip = device.ip_address;
        let port = device.port;
        let file_id = file_info.id;
        let exclude_clone = forward_exclude.clone();
        let request_id_clone = request_id.to_string();

        join_set.spawn(async move {
            // Call the /file/:file_id/cluster_similars endpoint with same request_id
            let result = http_client::find_similars(
                file_id,
                Some(&request_id_clone),
                &exclude_clone,
                limit,
                &ip,
                port,
            )
            .await;
            match result {
                Ok(data) => (device_id, device_name, data.results),
                Err(e) => {
                    log::warn!("Remote similar search failed on {}: {}", device_name, e);
                    (device_id, device_name, Vec::new())
                }
            }
        });
    }

    // Collect results
    let mut all_results: Vec<SearchResult> = Vec::new();

    while let Some(task_result) = join_set.join_next().await {
        match task_result {
            Ok((device_id, device_name, results)) => {
                // Add source device info to all remote results
                for mut result in results {
                    result.source_device = Some(SourceDevice {
                        device_id: device_id.clone(),
                        device_name: device_name.clone(),
                    });
                    all_results.push(result);
                }
            }
            Err(e) => {
                log::error!("Task join error: {}", e);
            }
        }
    }

    // Sort results by score (descending) and limit
    all_results.sort_by(|a, b| b.score.cmp(&a.score));
    all_results.truncate(limit);

    all_results
}

// ============================================
// Helper functions
// ============================================

/// Get list of searchable devices (paired and online)
async fn get_searchable_devices(device_ids: Option<Vec<String>>) -> Vec<Device> {
    get_searchable_devices_excluding(&device_ids.unwrap_or_default()).await
}

/// Get list of searchable devices excluding specified device IDs
/// 获取可搜索设备列表（排除指定设备ID）
async fn get_searchable_devices_excluding(exclude_device_ids: &[String]) -> Vec<Device> {
    let all_devices = match device_repo::list() {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to load devices: {}", e);
            return Vec::new();
        }
    };

    // Filter devices
    all_devices
        .into_iter()
        .filter(|d| {
            // Must be paired and online
            if d.pairing_status != PairingStatus::Paired {
                return false;
            }
            if d.online_status != OnlineStatus::Online {
                return false;
            }
            // Exclude specified device IDs
            if exclude_device_ids.contains(&d.device_id) {
                return false;
            }
            true
        })
        .collect()
}
