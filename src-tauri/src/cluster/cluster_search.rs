//! Cluster search service
//!
//! This module provides functionality for searching files across cluster devices.
//! For similarity search, see `crate::similarity::remote_similarity_service`.

use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::entities::Device;
use crate::enums::{OnlineStatus, PairingStatus};
use crate::repositories::device_repo;
use crate::structs::search_result::{SearchResult, SourceDevice};

use super::http_client::{self, MAX_RESULTS_PER_DEVICE};

/// Search status for tracking progress
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStatus {
    pub device_id: String,
    pub device_name: String,
    pub status: SearchOnlineStatus,
    pub result_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchOnlineStatus {
    Pending,
    Searching,
    Completed,
    Failed,
}

/// Remote device search result (alias for backward compatibility)
/// 远程设备搜索结果
pub type RemoteDeviceSearchResult = ClusterSearchResult;

/// Cluster search result
/// 集群搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSearchResult {
    pub results: Vec<SearchResult>,
    pub statuses: Vec<SearchStatus>,
    pub total: usize,
}

/// Perform cluster search (only remote devices, not local)
/// 执行集群搜索（仅远程设备，不包括本地）
///
/// This is called separately from local search for progressive loading
pub async fn remote_device_search(
    query: &str,
    search_type: &str,
    device_ids: Option<Vec<String>>,
) -> RemoteDeviceSearchResult {
    // Check test mode
    if crate::global::TEST_MODE_REMOTE_DEVICE.load(std::sync::atomic::Ordering::Relaxed) {
        return mock_remote_device_search(query, search_type);
    }

    // Get devices to search (only remote devices)
    let devices = get_searchable_devices(device_ids).await;

    // Initialize statuses for remote devices only
    let mut statuses: Vec<SearchStatus> = Vec::new();

    for device in &devices {
        let status = SearchStatus {
            device_id: device.device_id.clone(),
            device_name: device.name.clone(),
            status: SearchOnlineStatus::Pending,
            result_count: 0,
            error: None,
        };
        statuses.push(status);
    }

    // If no remote devices, return empty result
    if devices.is_empty() {
        return ClusterSearchResult {
            results: Vec::new(),
            statuses,
            total: 0,
        };
    }

    // Execute remote searches concurrently
    let mut join_set = JoinSet::new();

    for device in devices {
        let query_remote = query.to_string();
        let search_type_remote = search_type.to_string();
        let device_id = device.device_id;
        let device_name = device.name;
        let ip = device.ip_address;
        let port = device.port;

        join_set.spawn(async move {
            let result = http_client::search(&query_remote, &search_type_remote, &ip, port).await;
            match result {
                Ok(data) => (device_id, device_name, data.results, None),
                Err(e) => (device_id, device_name, Vec::new(), Some(e)),
            }
        });
    }

    // Collect results and update statuses
    let mut all_results: Vec<SearchResult> = Vec::new();

    while let Some(task_result) = join_set.join_next().await {
        match task_result {
            Ok((device_id, device_name, results, error)) => {
                // Update status for this device
                let status = if error.is_some() {
                    SearchStatus {
                        device_id: device_id.clone(),
                        device_name: device_name.clone(),
                        status: SearchOnlineStatus::Failed,
                        result_count: 0,
                        error,
                    }
                } else {
                    let count = results.len();
                    SearchStatus {
                        device_id: device_id.clone(),
                        device_name: device_name.clone(),
                        status: SearchOnlineStatus::Completed,
                        result_count: count,
                        error: None,
                    }
                };

                // Find and update the status
                for s in &mut statuses {
                    if s.device_id == device_id {
                        *s = status;
                        break;
                    }
                }

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

    // Sort results by score (descending)
    all_results.sort_by(|a, b| b.score.cmp(&a.score));

    // Limit total results
    all_results.truncate(MAX_RESULTS_PER_DEVICE * 3);

    let total = all_results.len();

    ClusterSearchResult {
        results: all_results,
        statuses,
        total,
    }
}

/// Get list of searchable devices
async fn get_searchable_devices(device_ids: Option<Vec<String>>) -> Vec<Device> {
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
            // If device_ids filter is specified, check it
            if let Some(ref ids) = device_ids {
                if !ids.contains(&d.device_id) {
                    return false;
                }
            }
            true
        })
        .collect()
}

// ============================================
// Mock cluster search for testing
// ============================================

/// Mock cluster search for testing
fn mock_remote_device_search(query: &str, _search_type: &str) -> ClusterSearchResult {
    let status = SearchStatus {
        device_id: super::mock::MOCK_DEVICE_ID.to_string(),
        device_name: super::mock::MOCK_DEVICE_NAME.to_string(),
        status: SearchOnlineStatus::Completed,
        result_count: 5,
        error: None,
    };

    let results = super::mock::generate_mock_search_results(query, MAX_RESULTS_PER_DEVICE);

    ClusterSearchResult {
        results,
        statuses: vec![status],
        total: 5,
    }
}
