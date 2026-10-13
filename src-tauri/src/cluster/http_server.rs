use crate::cluster::handlers::{cli, pairing, remote};
use crate::global::{APP_HANDLE, CLIENT_ID};
use axum::{
    Router, middleware,
    routing::{get, post},
};
use socket2::{Domain, Protocol, Socket, Type};
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// HTTP server shutdown signal sender
static SHUTDOWN_SENDER: RwLock<Option<watch::Sender<bool>>> = RwLock::new(None);

/// Default port for cluster service
/// 集群服务默认端口
pub const DEFAULT_PORT: i32 = 15678;

/// Whether this process is the CLI daemon carrying the server (decides whether /cli/shutdown actually exits)
/// 当前进程是否为承载 server 的 CLI daemon(决定 /cli/shutdown 是否真正退出)
pub static IS_DAEMON: AtomicBool = AtomicBool::new(false);

/// Timestamp of the last /cli request (basis for daemon idle exit;
/// /ping probes do not count)
/// 最近一次 /cli 请求时间(daemon 空闲退出依据;/ping 探测不计入)
static LAST_CLI_REQUEST: RwLock<Option<Instant>> = RwLock::new(None);

/// Mark this process as the CLI daemon carrying the server, and start the idle timer
/// 标记当前进程为承载 server 的 CLI daemon,并启动空闲计时
pub fn mark_daemon() {
    IS_DAEMON.store(true, Ordering::Relaxed);
    touch_cli_request();
}

/// Record one /cli request (refreshes the daemon idle timer)
/// 记录一次 /cli 请求(刷新 daemon 空闲计时)
pub fn touch_cli_request() {
    if let Ok(mut guard) = LAST_CLI_REQUEST.write() {
        *guard = Some(Instant::now());
    }
}

/// Seconds since the last /cli request (None when there has never been one)
/// 距最近一次 /cli 请求的秒数(从未请求过返回 None)
pub fn cli_idle_secs() -> Option<u64> {
    LAST_CLI_REQUEST
        .read()
        .ok()
        .and_then(|g| g.map(|t| t.elapsed().as_secs()))
}

/// Wait (bounded at 10s) until the serve task has actually stopped — i.e.
/// all in-flight responses are written and the listener released.
/// Callers use this after `stop_http_server` before exiting the process.
/// 等待(上限 10s)serve 任务真正停止 —— 在途响应已写完、监听已释放。
/// 调用方在 `stop_http_server` 之后、进程退出之前使用。
pub async fn wait_server_stopped() {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !server_already_running() {
            return;
        }
    }
}

/// Server state
#[derive(Clone)]
pub struct ServerState {
    pub device_id: String,
    pub device_name: String,
    pub port: i32,
}

/// Get the actual port the server is running on
/// 从全局配置获取端口
pub async fn get_actual_port() -> i32 {
    super::get_cluster_setting().await.port
}

/// Check if a port is available for binding (with exclusive access)
/// 检查端口是否可用于绑定（独占模式）
///
/// On Windows, uses SO_EXCLUSIVEADDRUSE to ensure exclusive port binding.
/// 在 Windows 上使用 SO_EXCLUSIVEADDRUSE 确保端口独占绑定。
pub async fn is_port_available(port: i32) -> bool {
    create_exclusive_socket(port).is_ok()
}

/// Create a socket with exclusive address use, bound to 0.0.0.0
/// 创建独占地址的 socket（绑定 0.0.0.0）
fn create_exclusive_socket(port: i32) -> Result<TcpListener, Error> {
    create_exclusive_socket_on(SocketAddr::from(([0, 0, 0, 0], port as u16)))
}

/// Create a socket with exclusive address use on the given address
/// 创建独占地址的 socket，绑定到指定地址
fn create_exclusive_socket_on(addr: SocketAddr) -> Result<TcpListener, Error> {
    // Create socket with socket2
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;

    // On Windows, set SO_EXCLUSIVEADDRUSE to prevent other sockets from binding to the same port
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            SO_EXCLUSIVEADDRUSE, SOL_SOCKET, setsockopt,
        };

        socket.set_reuse_address(false)?;

        // Set SO_EXCLUSIVEADDRUSE via Windows API
        let raw_socket = socket.as_raw_socket() as usize;
        let exclusive: i32 = 1;
        let result = unsafe {
            setsockopt(
                raw_socket,
                SOL_SOCKET,
                SO_EXCLUSIVEADDRUSE,
                &exclusive as *const i32 as *const u8,
                std::mem::size_of::<i32>() as i32,
            )
        };

        if result != 0 {
            return Err(Error::new(
                ErrorKind::Other,
                "Failed to set SO_EXCLUSIVEADDRUSE",
            ));
        }
    }

    // Bind the socket
    socket.bind(&addr.into())?;
    socket.listen(128)?;

    // Convert to tokio TcpListener
    let std_listener: std::net::TcpListener = socket.into();
    std_listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(std_listener)
}

/// Start HTTP server with configured port
/// 启动 HTTP 服务器，使用配置的端口
///
/// Port is loaded from database (cluster_setting.port).
/// If binding fails, returns error and notifies frontend.
/// 端口从数据库加载（cluster_setting.port）。
/// 如果绑定失败，返回错误并通知前端。
///
/// Uses SO_EXCLUSIVEADDRUSE on Windows to ensure exclusive port binding.
/// 在 Windows 上使用 SO_EXCLUSIVEADDRUSE 确保端口独占绑定。
pub async fn start_http_server() -> Result<(), String> {
    // Check if server is already running
    if server_already_running() {
        log::info!("HTTP server is already running, skip start");
        return Ok(());
    }

    let (port, device_id, device_name) = resolve_identity().await;

    // Bind with exclusive access; if the port is held by our own CLI daemon, ask it to yield first (close-then-start, so the daemon's model memory is released before this process warms up its own).
    // Retry a few times: a concurrent CLI invocation may spawn a fresh daemon into the gap between the old daemon's exit and our rebind — yield it too instead of failing.
    let listener = match create_exclusive_socket(port) {
        Ok(l) => l,
        Err(e) => {
            let mut last_err = e;
            let mut bound = None;
            for _ in 0..5 {
                if !try_yield_to_cli_daemon(port).await {
                    break;
                }
                match create_exclusive_socket(port) {
                    Ok(l) => {
                        bound = Some(l);
                        break;
                    }
                    Err(retry_err) => last_err = retry_err,
                }
            }
            match bound {
                Some(l) => l,
                None => {
                    log::error!("Failed to bind port {}: {}", port, last_err);
                    emit_port_error(port);
                    return Err(format!("Port {} is already in use: {}", port, last_err));
                }
            }
        }
    };

    // Update cluster port in database and global state (ensures consistency)
    if let Err(e) = super::update_cluster_port(port).await {
        log::error!("Failed to update cluster port: {}", e);
    }

    serve_on(listener, device_id, device_name, port).await
}

/// Clamp to the valid port range (single source shared with the CLI's server_port): out-of-range values fall back to the default so `as u16` truncation can never make bind/probe/check disagree.
/// 钳制到合法端口区间(与 CLI 的 server_port 共用的单一来源):超范围值统一回退默认端口,`as u16` 截断不会让绑定/探测/check 落在不同端口上。
pub fn clamp_port(port: i32) -> i32 {
    if (1..=65535).contains(&port) {
        port
    } else {
        DEFAULT_PORT
    }
}

/// Resolve the shared server identity: port (DB cluster_setting with the typed default) plus device_id/device_name for ServerState.
/// 解析公共服务身份:端口(DB cluster_setting,类型默认值兜底)与 ServerState 所需的 device_id/device_name。
async fn resolve_identity() -> (i32, String, String) {
    let setting = super::get_cluster_setting().await;
    let port = clamp_port(setting.port);
    let device_id = crate::read_lock!(CLIENT_ID).clone();
    let device_name = if setting.device_name.is_empty() {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "MangoFinder".to_string())
    } else {
        setting.device_name
    };
    (port, device_id, device_name)
}

/// Whether the HTTP server is already running in this process
/// 本进程内服务是否已在运行
fn server_already_running() -> bool {
    SHUTDOWN_SENDER.read().map(|g| g.is_some()).unwrap_or(false)
}

/// Start the HTTP server bound to loopback only (CLI daemon, `mfd serve`)
/// 以 CLI daemon(`mfd serve`)身份启动,仅绑定回环地址,不向局域网暴露
pub async fn start_http_server_loopback() -> Result<(), String> {
    if server_already_running() {
        log::info!("HTTP server is already running, skip start");
        return Ok(());
    }
    let (port, device_id, device_name) = resolve_identity().await;

    // The daemon never yields: a busy port means a server already exists (GUI-carried, or a concurrent daemon that won the bind race) — this process is the loser and exits (see daemon_server::run).
    let addr = SocketAddr::from(([127, 0, 0, 1], port as u16));
    let listener = create_exclusive_socket_on(addr)
        .map_err(|e| format!("Port {} is already in use: {}", port, e))?;

    serve_on(listener, device_id, device_name, port).await
}

/// Notify frontend about port binding failure (user-friendly message)
/// 通知前端端口绑定失败（用户友好的信息）
fn emit_port_error(port: i32) {
    if let Some(app_handle) = APP_HANDLE.get() {
        if let Err(emit_err) =
            app_handle.emit("cluster-port-error", &serde_json::json!({ "port": port }))
        {
            log::error!("Failed to emit cluster-port-error event: {}", emit_err);
        }
    }
}

/// Shared server setup: routes, shutdown channel, background serve task
/// 公共启动逻辑:路由、关闭通道、后台 serve
async fn serve_on(
    listener: TcpListener,
    device_id: String,
    device_name: String,
    port: i32,
) -> Result<(), String> {
    let local_addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| format!("port {}", port));
    log::info!("HTTP server listening on {}", local_addr);

    remote::init_request_cache();

    let state = Arc::new(ServerState {
        device_id,
        device_name,
        port,
    });

    // /cli API: local clients only, loopback + local CLIENT_ID double-check
    let cli_routes = Router::new()
        .route("/cli/search", post(cli::cli_search))
        .route("/cli/similar/:file_id", post(cli::cli_similar))
        .route("/cli/shutdown", post(cli::cli_shutdown))
        .layer(middleware::from_fn(cli::verify_loopback));

    // Inter-device protocol gets a /cluster prefix, separated from the
    // /cli (local) namespace.
    // No legacy-path compatibility: devices are expected to upgrade together.
    let cluster_routes = Router::new()
        .route("/search", post(remote::handle_search))
        .route("/chunks", post(remote::handle_chunks))
        .route(
            "/file/:file_id/similars",
            post(remote::handle_find_similars),
        )
        .route("/file/:file_id", get(remote::handle_file_download))
        .route("/file/:file_id/content", get(remote::handle_file_content))
        .route("/pairing/request", post(pairing::handle_pairing_request))
        .route("/pairing/respond", post(pairing::handle_pairing_respond))
        .route("/pairing/reset_notify", post(pairing::handle_reset_notify));

    let app = Router::new()
        .route("/ping", get(remote::handle_ping))
        .nest(super::CLUSTER_PREFIX, cluster_routes)
        .merge(cli_routes)
        .with_state(state);

    // Setup shutdown channel
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(false);
    if let Ok(mut guard) = SHUTDOWN_SENDER.write() {
        *guard = Some(shutdown_sender);
    } else {
        log::error!("Failed to write SHUTDOWN_SENDER: lock poisoned");
    }

    // Spawn the server in a background task so this function returns immediately
    tokio::spawn(async move {
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown_receiver.changed().await.ok();
            log::info!("HTTP server shutdown signal received");
        })
        .await
        {
            log::error!("HTTP server error: {}", e);
        }
        // The server has actually stopped: clear the sender so a later start_http_server (port change, disable→re-enable) can bind again instead of seeing a stale "already running" flag.
        if let Ok(mut guard) = SHUTDOWN_SENDER.write() {
            *guard = None;
        }
    });

    Ok(())
}

/// Port held by our own CLI daemon: ask it to exit and wait until it really does (probe no longer `Mine`) before the caller rebinds — close first, then start.
/// The probe/shutdown handshake is shared with the CLI via `cluster::server`.
/// 端口被自家 CLI daemon 占用时:请求其退出并等待端口真正释放(探测不再是自家服务),再由调用方重绑 —— 先关后开。
/// 探测/关闭握手与 CLI 共用 `cluster::server`。
async fn try_yield_to_cli_daemon(port: i32) -> bool {
    use super::server::{self, ProbeOutcome, ReleaseOutcome};

    // Only our own daemon yields; the GUI process or another holder does not
    if server::probe(port).await != ProbeOutcome::Mine {
        return false;
    }
    let _ = server::request_shutdown(port).await;

    // The daemon's graceful shutdown is bounded at 10s (in-flight responses); allow 12s before declaring it stuck
    match server::wait_until_released(port, Duration::from_secs(12)).await {
        ReleaseOutcome::Released => {
            log::info!("CLI daemon on port {} has yielded", port);
            true
        }
        ReleaseOutcome::StillHeld => {
            log::warn!("CLI daemon on port {} did not exit in time", port);
            false
        }
    }
}

/// Stop HTTP server
pub async fn stop_http_server() {
    // Send shutdown signal
    if let Ok(guard) = SHUTDOWN_SENDER.read() {
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(true);
        }
    }
    // Note: Don't clear SHUTDOWN_SENDER here, let it be cleared when server actually stops
}
