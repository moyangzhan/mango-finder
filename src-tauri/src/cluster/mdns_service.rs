use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use tokio::sync::RwLock as AsyncRwLock;

use crate::entities::Device;
use crate::enums::PairingStatus;
use crate::global::{APP_HANDLE, CLIENT_ID};
use crate::repositories::{device_repo, file_info_repo};

/// mDNS service type for MangoFinder
const SERVICE_TYPE: &str = "_mangofinder._tcp.local.";

// ============================================================================ Broadcast (Advertising)
// ============================================================================

/// mDNS broadcast daemon instance
static MDNS_BROADCAST_DAEMON: OnceLock<Arc<AsyncRwLock<Option<ServiceDaemon>>>> = OnceLock::new();

/// Start mDNS broadcast (advertise this device)
/// 启动 mDNS 广播（让其他设备发现本设备）
pub async fn start_mdns_broadcast(device_name: String, port: i32) -> Result<(), String> {
    // Check if already running
    if let Some(daemon_arc) = MDNS_BROADCAST_DAEMON.get() {
        let guard = daemon_arc.read().await;
        if guard.is_some() {
            log::warn!("mDNS broadcast already running");
            return Ok(());
        }
    }

    // Create service daemon
    let daemon =
        ServiceDaemon::new().map_err(|e| format!("Failed to create mDNS daemon: {}", e))?;

    // Get local IP address
    let local_ip = super::http_client::get_local_ip()?;
    log::info!("Starting mDNS broadcast on {}", local_ip);

    // Prepare TXT records
    let mut txt_records = HashMap::new();
    txt_records.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
    let index_count = file_info_repo::count().unwrap_or(0);
    txt_records.insert("indexCount".to_string(), index_count.to_string());
    let device_id = crate::read_lock!(CLIENT_ID).clone();
    txt_records.insert("deviceId".to_string(), device_id);

    // Create service info
    let ip_addr: IpAddr = local_ip
        .parse()
        .map_err(|e| format!("Failed to parse local IP: {}", e))?;

    let service_info = ServiceInfo::new(
        SERVICE_TYPE,
        &device_name,
        &format!("{}.local.", device_name.replace(" ", "-")),
        ip_addr,
        port as u16,
        txt_records,
    )
    .map_err(|e| format!("Failed to create service info: {}", e))?;

    // Register service
    daemon
        .register(service_info)
        .map_err(|e| format!("Failed to register mDNS service: {}", e))?;

    log::info!(
        "mDNS broadcast started: {} at {}:{}",
        device_name,
        local_ip,
        port
    );

    // Store daemon reference
    let service = Arc::new(AsyncRwLock::new(Some(daemon)));
    let _ = MDNS_BROADCAST_DAEMON.set(service);

    Ok(())
}

/// Stop mDNS broadcast
/// 停止 mDNS 广播
pub async fn stop_mdns_broadcast() {
    if let Some(daemon_arc) = MDNS_BROADCAST_DAEMON.get() {
        let mut guard = daemon_arc.write().await;
        if let Some(d) = guard.take() {
            let _ = d.shutdown();
            log::info!("mDNS broadcast stopped");
        }
    }
}

/// Check if mDNS broadcast is running
pub fn is_mdns_broadcast_running() -> bool {
    if let Some(daemon_arc) = MDNS_BROADCAST_DAEMON.get() {
        // Use try_read to avoid blocking
        if let Ok(guard) = daemon_arc.try_read() {
            return guard.is_some();
        }
    }
    false
}

// ============================================================================ Browsing (Discovery)
// ============================================================================

/// mDNS browsing running flag
static MDNS_BROWSING: AtomicBool = AtomicBool::new(false);

/// mDNS browsing daemon instance (separate from broadcast)
static MDNS_BROWSING_DAEMON: OnceLock<Arc<AsyncRwLock<Option<ServiceDaemon>>>> = OnceLock::new();

/// Start browsing for other MangoFinder devices
/// 启动 mDNS 浏览（发现其他设备）
pub async fn start_mdns_browsing() -> Result<(), String> {
    if MDNS_BROWSING.load(Ordering::SeqCst) {
        log::warn!("mDNS browsing already running");
        return Ok(());
    }

    MDNS_BROWSING.store(true, Ordering::SeqCst);

    // Get local IP for logging
    let local_ip = super::http_client::get_local_ip().unwrap_or_else(|_| "unknown".to_string());
    log::info!("Starting mDNS browsing, local IP: {}", local_ip);

    // Create a separate daemon for browsing
    let daemon =
        ServiceDaemon::new().map_err(|e| format!("Failed to create mDNS browser daemon: {}", e))?;

    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| format!("Failed to start mDNS browsing: {}", e))?;

    // Store browsing daemon reference
    let browsing_daemon = Arc::new(AsyncRwLock::new(Some(daemon)));
    let _ = MDNS_BROWSING_DAEMON.set(browsing_daemon);

    tokio::spawn(async move {
        while MDNS_BROWSING.load(Ordering::SeqCst) {
            match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(event) => {
                    log::debug!("mDNS event received: {:?}", event);
                    handle_mdns_event(event).await;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("timeout")
                        || err_str.contains("timed out")
                        || err_str.contains("TimedOut")
                    {
                        // Continue waiting - these are normal timeout events
                        continue;
                    } else if err_str.contains("valid addrs")
                        || err_str.contains("link-local")
                        || err_str.contains("IPv6")
                    {
                        // IPv6 link-local address resolution issues - log warning but continue
                        log::warn!(
                            "mDNS address resolution issue (likely IPv6 link-local): {}",
                            e
                        );
                        continue;
                    } else {
                        log::error!("mDNS browsing error: {}", e);
                        break;
                    }
                }
            }
        }
        log::info!("mDNS browsing stopped");
    });

    log::info!("mDNS browsing started for {}", SERVICE_TYPE);
    Ok(())
}

/// Stop browsing for devices
/// 停止 mDNS 浏览
pub async fn stop_mdns_browsing() {
    MDNS_BROWSING.store(false, Ordering::SeqCst);

    // Shutdown browsing daemon
    if let Some(daemon_arc) = MDNS_BROWSING_DAEMON.get() {
        let mut guard = daemon_arc.write().await;
        if let Some(d) = guard.take() {
            let _ = d.shutdown();
        }
    }
    log::info!("mDNS browsing stopped");
}

/// Check if mDNS browsing is running
pub fn is_mdns_browsing_running() -> bool {
    MDNS_BROWSING.load(Ordering::SeqCst)
}

// ============================================================================ Legacy compatibility functions
// ============================================================================

/// Start mDNS service (broadcast + browsing) - for backward compatibility
/// 启动 mDNS 服务（广播 + 浏览）- 向后兼容
pub async fn start_mdns_service(device_name: String, port: i32) -> Result<(), String> {
    start_mdns_broadcast(device_name, port).await?;
    start_mdns_browsing().await?;
    Ok(())
}

/// Stop mDNS service (broadcast + browsing) - for backward compatibility
/// 停止 mDNS 服务（广播 + 浏览）- 向后兼容
pub async fn stop_mdns_service() {
    stop_mdns_browsing().await;
    stop_mdns_broadcast().await;
}

// ============================================================================ Event handling
// ============================================================================

/// Handle mDNS discovery events
async fn handle_mdns_event(event: ServiceEvent) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let name = info.get_fullname();
            let addresses = info.get_addresses();
            let port = info.get_port();
            let hostname = info.get_hostname();

            // Classify: IPv4 / IPv6 link-local / IPv6 global
            let ipv4_addrs: Vec<_> = addresses.iter().filter(|a| a.is_ipv4()).collect();
            let ipv6_link_local: Vec<_> = addresses
                .iter()
                .filter(|a| a.is_ipv6() && a.to_string().starts_with("fe80::"))
                .collect();
            let ipv6_global: Vec<_> = addresses
                .iter()
                .filter(|a| a.is_ipv6() && !a.to_string().starts_with("fe80::"))
                .collect();

            log::debug!(
                "Device {} (hostname: {}) resolved: {} IPv4, {} IPv6 link-local, {} IPv6 global",
                name,
                hostname,
                ipv4_addrs.len(),
                ipv6_link_local.len(),
                ipv6_global.len()
            );

            // Preference: IPv4 > IPv6 global > IPv6 link-local
            let host = ipv4_addrs
                .first()
                .or_else(|| ipv6_global.first())
                .or_else(|| ipv6_link_local.first())
                .map(|a| a.to_string())
                .unwrap_or_default();

            // No address resolved: warn and skip
            if host.is_empty() {
                log::warn!(
                    "Device {} resolved but no valid address found, skipping",
                    name
                );
                return;
            }

            // Log the chosen address type
            let selected_type = if ipv4_addrs.first().map(|a| a.to_string()) == Some(host.clone()) {
                "IPv4"
            } else if host.starts_with("fe80::") {
                "IPv6 link-local"
            } else {
                "IPv6 global"
            };
            log::debug!(
                "Device {} selected address: {} ({})",
                name,
                host,
                selected_type
            );

            // Extract TXT records
            let txt = info.get_properties();
            let device_id = txt
                .iter()
                .find(|p| p.key() == "deviceId")
                .map(|p| p.val_str().to_string())
                .unwrap_or_default();

            let device_name = txt
                .iter()
                .find(|p| p.key() == "name")
                .map(|p| p.val_str().to_string())
                .unwrap_or_else(|| name.replace("._mangofinder._tcp.local.", ""));

            let version = txt
                .iter()
                .find(|p| p.key() == "version")
                .map(|p| p.val_str().to_string())
                .unwrap_or_default();

            let index_count = txt
                .iter()
                .find(|p| p.key() == "indexCount")
                .and_then(|p| p.val_str().parse().ok())
                .unwrap_or(0);

            // Skip self
            let my_device_id = crate::read_lock!(CLIENT_ID).clone();
            if device_id == my_device_id || device_id.is_empty() {
                return;
            }

            // Check for existing device
            let existing_device = device_repo::get_by_device_id(&device_id).ok().flatten();

            // Skip devices that were rejected by local user (Rejected)
            // Skip devices the local user rejected
            // Note: Blocked devices (remote blocked us) are still discoverable, they can send new requests
            // Blocked devices (they blacklisted us) stay discoverable; they may send new requests
            if let Some(ref existing) = existing_device {
                if existing.pairing_status == PairingStatus::Rejected {
                    log::debug!("Skipping rejected device: {}", device_id);
                    return;
                }
            }

            log::info!(
                "Discovered device: {} ({}) at {}:{}",
                device_name,
                device_id,
                host,
                port
            );

            let is_new_device = existing_device.is_none();

            // Create or update device record, preserving existing fields
            // NOTE: mDNS only discovers devices, it does NOT determine online_status.
            // online_status is determined by device_checker via HTTP requests.
            let device = if let Some(mut existing) = existing_device {
                // Update existing device with new discovery info
                existing.name = device_name;
                existing.ip_address = host;
                existing.port = port as i32;
                existing.version = version;
                existing.index_count = index_count;
                // Don't set online_status here - let device_checker handle it
                existing.last_seen = chrono::Local::now();
                existing
            } else {
                // New device, create with default values online_status defaults to Unknown, will be set by device_checker
                Device {
                    device_id: device_id.clone(),
                    name: device_name,
                    ip_address: host,
                    port: port as i32,
                    version,
                    // online_status defaults to Unknown
                    index_count,
                    discovery_method: "mdns".to_string(),
                    ..Default::default()
                }
            };

            match device_repo::upsert(&device) {
                Ok(_) => {
                    // Notify frontend about new device
                    if let Some(app_handle) = APP_HANDLE.get() {
                        let _ = app_handle.emit("device-discovered", &device);
                    }

                    // Auto send pairing request if configured
                    if is_new_device {
                        let setting = super::get_cluster_setting().await;
                        if setting.auto_request_pairing {
                            log::info!(
                                "Auto sending pairing request to {} ({})",
                                device.name,
                                device_id
                            );
                            let device_id_clone = device_id.clone();
                            let device_name = device.name.clone();
                            let ip = device.ip_address.clone();
                            let port = device.port;

                            tokio::spawn(async move {
                                if let Err(e) = super::http_client::send_pairing_request(
                                    &device_id_clone,
                                    &device_name,
                                    &ip,
                                    port,
                                )
                                .await
                                {
                                    log::error!("Failed to auto send pairing request: {}", e);
                                }
                            });
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to save discovered device: {}", e);
                }
            }
        }
        ServiceEvent::ServiceRemoved(_, name) => {
            log::info!("Device removed: {}", name);
            // We don't remove the device, just mark as offline
            // The heartbeat mechanism will handle status updates
        }
        ServiceEvent::ServiceFound(service_type, name) => {
            log::debug!("Service found: {} ({})", name, service_type);
        }
        _ => {
            log::debug!("Unhandled mDNS event: {:?}", event);
        }
    }
}
