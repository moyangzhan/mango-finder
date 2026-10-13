pub mod api_types;
pub mod cluster_search;
pub mod device_checker;
pub mod http_client;
pub mod mdns_service;
pub mod mock;

use crate::server::http_server;
use crate::structs::cluster_config::ClusterSetting;
use std::sync::OnceLock;
use tauri::Emitter;
use tokio::sync::RwLock as AsyncRwLock;

/// URL prefix of the inter-device endpoints, shared by server routing (Router::nest) and client URL building
pub const CLUSTER_PREFIX: &str = "/cluster";

/// Global cluster setting
pub static CLUSTER_SETTING: OnceLock<AsyncRwLock<ClusterSetting>> = OnceLock::new();

/// Initialize cluster setting from database
pub async fn init_cluster_setting() {
    let setting = load_cluster_setting_from_db().unwrap_or_default();
    let _ = CLUSTER_SETTING.set(AsyncRwLock::new(setting));
}

/// Load cluster setting from database
fn load_cluster_setting_from_db() -> Result<ClusterSetting, String> {
    use crate::repositories::config_repo;

    let json_str = config_repo::get_value("cluster_setting")?.unwrap_or_default();

    let mut setting = if json_str.is_empty() {
        ClusterSetting::default()
    } else {
        ClusterSetting::from_json_string(&json_str)
    };

    // Auto-generate device name from hostname if empty
    if setting.device_name.is_empty() {
        setting.device_name = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "MangoFinder".to_string());
        // Save the auto-generated name to database
        config_repo::upsert("cluster_setting", &setting.to_json_string())
            .map_err(|e| e.to_string())?;
    }

    Ok(setting)
}

/// Save cluster setting to database
pub async fn save_cluster_setting(setting: &ClusterSetting) -> Result<(), String> {
    use crate::repositories::config_repo;

    // Get old setting to detect changes
    let old_setting = get_cluster_setting().await;
    let old_discoverable = old_setting.allow_to_be_discovered;
    let old_enabled = old_setting.enabled;
    let old_port = old_setting.port;

    // Check if port changed and cluster is enabled
    let port_changed = old_port != setting.port && old_enabled;
    if port_changed {
        // Check if new port is available before making any changes
        if !http_server::is_port_available(setting.port).await {
            let error_msg = format!("Port {} is already in use", setting.port);
            log::error!("Failed to change port: {}", error_msg);

            // Notify frontend about port binding failure (user-friendly message)
            if let Some(app_handle) = crate::global::APP_HANDLE.get() {
                let _ = app_handle.emit(
                    "cluster-port-error",
                    &serde_json::json!({
                        "port": setting.port
                    }),
                );
            }

            return Err(error_msg);
        }
    }

    config_repo::upsert("cluster_setting", &setting.to_json_string()).map_err(|e| e.to_string())?;

    // Update global setting
    let mut global_setting = CLUSTER_SETTING
        .get()
        .ok_or("Cluster setting not initialized")?
        .write()
        .await;
    *global_setting = setting.clone();
    drop(global_setting); // Release lock before async operations

    // Notify frontend that setting saved successfully (clear any previous port error)
    if let Some(app_handle) = crate::global::APP_HANDLE.get() {
        let _ = app_handle.emit(
            "cluster-setting-saved",
            &serde_json::json!({
                "port": setting.port
            }),
        );
    }

    // Dynamic adjustment based on changes
    let new_discoverable = setting.allow_to_be_discovered;
    let new_enabled = setting.enabled;

    // Handle cluster enabled/disabled change
    if old_enabled != new_enabled {
        if new_enabled {
            start_cluster_service().await?;
        } else {
            stop_cluster_service().await?;
        }
        return Ok(());
    }

    // If cluster is running, adjust services dynamically
    if is_cluster_enabled() {
        // Handle port change - restart HTTP server and mDNS
        if port_changed {
            log::info!(
                "Port changed from {} to {}, restarting services...",
                old_port,
                setting.port
            );

            // Stop mDNS broadcast first
            mdns_service::stop_mdns_broadcast().await;

            // Stop HTTP server
            http_server::stop_http_server().await;

            // Wait until the server has actually stopped (graceful shutdown finishes the in-flight requests) instead of a fixed 500ms sleep:
            // shutdown can outlive the sleep, after which start_http_server would short-circuit as "already running" and never rebind.
            http_server::wait_server_stopped().await;

            // Start HTTP server with new port
            if let Err(e) = http_server::start_http_server().await {
                log::error!("Failed to restart HTTP server: {}", e);
                return Err(e);
            }

            // Restart mDNS broadcast if discovery is enabled
            if new_discoverable {
                let device_name = if setting.device_name.is_empty() {
                    hostname::get()
                        .map(|h| h.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "MangoFinder".to_string())
                } else {
                    setting.device_name.clone()
                };
                let port = setting.port;

                tokio::spawn(async move {
                    if let Err(e) = mdns_service::start_mdns_broadcast(device_name, port).await {
                        log::error!("Failed to restart mDNS broadcast: {}", e);
                    }
                });
            }

            return Ok(());
        }

        // Handle discoverable flag change
        if old_discoverable != new_discoverable {
            if new_discoverable {
                // Start mDNS broadcast
                let device_name = if setting.device_name.is_empty() {
                    hostname::get()
                        .map(|h| h.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "MangoFinder".to_string())
                } else {
                    setting.device_name.clone()
                };
                let port = http_server::get_actual_port().await;

                if port > 0 {
                    tokio::spawn(async move {
                        if let Err(e) = mdns_service::start_mdns_broadcast(device_name, port).await
                        {
                            log::error!("Failed to start mDNS broadcast: {}", e);
                        }
                    });
                }
            } else {
                // Stop mDNS broadcast
                mdns_service::stop_mdns_broadcast().await;
            }
        }
    }

    Ok(())
}

/// Get current cluster setting
pub async fn get_cluster_setting() -> ClusterSetting {
    if let Some(setting) = CLUSTER_SETTING.get() {
        setting.read().await.clone()
    } else {
        ClusterSetting::default()
    }
}

/// Update only the port field in cluster setting
/// 只更新 cluster setting 中的 port 字段
pub async fn update_cluster_port(new_port: i32) -> Result<(), String> {
    use crate::repositories::config_repo;

    let mut setting = get_cluster_setting().await;

    if setting.port == new_port {
        return Ok(()); // No change needed
    }

    setting.port = new_port;

    // Save to database
    config_repo::upsert("cluster_setting", &setting.to_json_string()).map_err(|e| e.to_string())?;

    // Update global setting
    if let Some(global_setting) = CLUSTER_SETTING.get() {
        let mut guard = global_setting.write().await;
        guard.port = new_port;
    }

    log::info!("Updated cluster port to {}", new_port);
    Ok(())
}

/// Check if cluster service is enabled
/// 检查 cluster 服务是否启用
pub fn is_cluster_enabled() -> bool {
    if let Some(setting) = CLUSTER_SETTING.get() {
        // Use try_read to avoid blocking, default to false if lock is held
        if let Ok(guard) = setting.try_read() {
            return guard.enabled;
        }
    }
    false
}

/// Start cluster services (HTTP + mDNS)
pub async fn start_cluster_service() -> Result<(), String> {
    let setting = get_cluster_setting().await;
    if !setting.enabled {
        log::info!("Cluster feature is disabled, skip start");
        return Ok(());
    }

    log::info!("Starting cluster service...");

    // Start HTTP server (loads port from database, no fallback)
    // If port is occupied, returns error and notifies frontend
    let http_result = http_server::start_http_server().await;
    if let Err(e) = &http_result {
        log::error!("HTTP server failed to start: {}", e);
        // Error already emitted to frontend via 'cluster-port-error' event
        return Err(e.clone());
    }

    // Get actual bound port
    let actual_port = http_server::get_actual_port().await;

    // Start mDNS broadcast if discovery is enabled
    if setting.allow_to_be_discovered && actual_port > 0 {
        let device_name = if setting.device_name.is_empty() {
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "MangoFinder".to_string())
        } else {
            setting.device_name.clone()
        };

        tokio::spawn(async move {
            // Start mDNS broadcast (advertise this device)
            if let Err(e) = mdns_service::start_mdns_broadcast(device_name, actual_port).await {
                log::error!("mDNS broadcast error: {}", e);
            }
        });
    }

    // Always start mDNS browsing when cluster is enabled (to discover other devices)
    tokio::spawn(async move {
        if let Err(e) = mdns_service::start_mdns_browsing().await {
            log::error!("mDNS browsing error: {}", e);
        }
    });

    // Start device online checker
    let check_interval = setting.online_check_interval;
    tokio::spawn(async move {
        device_checker::start_status_checker(check_interval).await;
    });

    log::info!("Cluster service started on port {}", actual_port);
    Ok(())
}

/// Stop cluster services
pub async fn stop_cluster_service() -> Result<(), String> {
    if !is_cluster_enabled() {
        return Ok(());
    }

    // Stop device checker
    device_checker::stop_status_checker().await;

    // Signal HTTP server to stop
    http_server::stop_http_server().await;

    // Signal mDNS service to stop
    mdns_service::stop_mdns_service().await;

    log::info!("Cluster service stopped");
    Ok(())
}

/// Restart cluster services (after setting change)
pub async fn restart_cluster_service() -> Result<(), String> {
    stop_cluster_service().await?;
    // Small delay to allow ports to be released
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    start_cluster_service().await
}
