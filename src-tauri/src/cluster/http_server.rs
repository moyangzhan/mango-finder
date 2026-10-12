use crate::cluster::api_types::{
    CODE_DEVICE_NOT_PAIRED, CODE_INTERNAL_ERROR, CODE_MISSING_DEVICE_ID, CODE_NOT_FOUND,
    ChunksData, ChunksRequest, CliSearchData, CliSearchRequest, CliSimilarData, CliSimilarRequest,
    CommonResponse, FindSimilarsData, PingData, SearchData,
};
use crate::entities::{Device, PairingRequest};
use crate::enums::{OnlineStatus, PairingRequestStatus, PairingResponseStatus, PairingStatus};
use crate::global::{APP_HANDLE, CLIENT_ID, STORAGE_PATH};
use crate::repositories::{
    device_repo, file_content_embedding_repo, file_info_repo, pairing_request_repo,
};
use crate::searcher;
use crate::similarity::similarity_service;
use crate::structs::cluster_config::{
    DeviceCapabilities, PairingRequestPayload, PairingResponsePayload, RemoteFindSimilarsRequest,
    RemoteSearchRequest, ResetNotifyPayload,
};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use rust_i18n::t;
use serde::Serialize;
use serde_json::{Value, json};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::sync::watch;

/// HTTP server shutdown signal sender
static SHUTDOWN_SENDER: std::sync::RwLock<Option<watch::Sender<bool>>> =
    std::sync::RwLock::new(None);

/// Default port for cluster service
/// 集群服务默认端口
pub const DEFAULT_PORT: i32 = 15678;

/// Cache for processed request IDs to prevent loops
/// 用于防止循环的已处理请求ID缓存
/// Key: request_id, Value: timestamp when processed
static PROCESSED_REQUESTS: std::sync::RwLock<Option<HashMap<String, Instant>>> =
    std::sync::RwLock::new(None);

/// Cache expiration time in seconds (10 minutes)
/// 缓存过期时间（10分钟）
const REQUEST_CACHE_TTL_SECS: u64 = 600;

/// Whether this process runs as the CLI daemon host
/// (decides whether /cli/shutdown actually exits)
/// 当前进程是否以 CLI daemon 宿主身份运行(决定 /cli/shutdown 是否真正退出)
pub static IS_DAEMON: AtomicBool = AtomicBool::new(false);

/// Timestamp of the last /cli request (basis for daemon idle exit;
/// /ping probes do not count)
/// 最近一次 /cli 请求时间(daemon 空闲退出依据;/ping 探测不计入)
static LAST_CLI_REQUEST: std::sync::RwLock<Option<Instant>> = std::sync::RwLock::new(None);

/// Mark this process as the CLI daemon host and start the idle timer
/// 标记当前进程为 CLI daemon 宿主,并启动空闲计时
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

// ============================================================================
// Helper Functions / 辅助函数
// ============================================================================

/// Verify that the requester device is paired (returns CommonResponse error)
/// 验证请求者设备是否已配对（返回 CommonResponse 错误）
fn verify_paired_device<T: Serialize>(headers: &HeaderMap) -> Result<&str, CommonResponse<T>> {
    let requester_id = headers
        .get("X-Device-ID")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            log::warn!("Missing X-Device-ID header");
            CommonResponse::error(CODE_MISSING_DEVICE_ID, "Missing X-Device-ID header")
        })?;

    let device = device_repo::get_by_device_id(requester_id).map_err(|e| {
        log::error!("Failed to get device: {}", e);
        CommonResponse::error(CODE_INTERNAL_ERROR, "Failed to get device")
    })?;

    match device {
        Some(d) if d.pairing_status == PairingStatus::Paired => Ok(requester_id),
        Some(d) => {
            log::warn!(
                "Device {} not paired, status: {:?}",
                requester_id,
                d.pairing_status
            );
            Err(CommonResponse::error(
                CODE_DEVICE_NOT_PAIRED,
                "Device not paired",
            ))
        }
        None => {
            log::warn!("Device {} not found", requester_id);
            Err(CommonResponse::error(
                CODE_DEVICE_NOT_PAIRED,
                "Device not found",
            ))
        }
    }
}

/// Verify that the requester device is paired (returns StatusCode for binary responses)
/// 验证请求者设备是否已配对（为二进制响应返回 StatusCode）
fn verify_paired_device_status(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let requester_id = headers
        .get("X-Device-ID")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            log::warn!("Missing X-Device-ID header");
            StatusCode::BAD_REQUEST
        })?;

    let device = device_repo::get_by_device_id(requester_id).map_err(|e| {
        log::error!("Failed to get device: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    match device {
        Some(d) if d.pairing_status == PairingStatus::Paired => Ok(requester_id),
        Some(d) => {
            log::warn!(
                "Device {} not paired, status: {:?}",
                requester_id,
                d.pairing_status
            );
            Err(StatusCode::FORBIDDEN)
        }
        None => {
            log::warn!("Device {} not found", requester_id);
            Err(StatusCode::FORBIDDEN)
        }
    }
}

// ============================================================================
// Local API /cli - endpoints for local (on-machine) clients
// Local API /cli - 本机客户端(CLI)端点
// ============================================================================

/// /cli auth: loopback connections only, X-Device-ID must be the local
/// CLIENT_ID, and the Host header must be a loopback literal. Local processes
/// share the same trust domain as the database (they can read it directly).
/// The Host check blocks DNS-rebinding pages: a rebound browser origin
/// connects from 127.0.0.1 and can read device_id from the unauthenticated
/// /ping, but its Host header carries the attacker hostname, not a loopback
/// literal. Remote devices do not use these endpoints and go through cluster
/// pairing.
/// /cli 鉴权:仅允许回环连接、X-Device-ID 须为本机 CLIENT_ID,且 Host 头
/// 必须是回环字面量。本机进程与数据库同信任域(可直接读库)。Host 校验
/// 用于阻断 DNS rebinding 页面:重绑定源的浏览器请求虽来自 127.0.0.1、
/// 也能从无鉴权的 /ping 读到 device_id,但其 Host 头是攻击者域名而非回环
/// 字面量。远程设备不走这组端点,仍走 cluster 配对校验。
async fn verify_loopback(req: Request, next: Next) -> Response {
    let own_id = crate::read_lock!(CLIENT_ID).clone();
    let allowed = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false)
        && req
            .headers()
            .get("X-Device-ID")
            .and_then(|v| v.to_str().ok())
            .map(|id| id == own_id)
            .unwrap_or(false)
        && req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(is_loopback_host)
            .unwrap_or(false);
    if !allowed {
        return StatusCode::FORBIDDEN.into_response();
    }
    touch_cli_request();
    next.run(req).await
}

/// Whether a Host header value (e.g. "127.0.0.1:10678") names a loopback
/// literal ("127.0.0.1", "localhost", "[::1]"), ignoring the port.
/// Host 头值(如 "127.0.0.1:10678")是否为回环字面量("127.0.0.1"、
/// "localhost"、"[::1]"),忽略端口。
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    // IPv6 字面量带端口时形如 "[::1]:port"
    if let Some(rest) = host.strip_prefix("[::1]") {
        return rest.is_empty() || rest.starts_with(':');
    }
    let name = host.split(':').next().unwrap_or("");
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1"
}

/// POST /cli/search - query entry for local clients.
/// Calls the same searcher function as the CLI's in-process path,
/// so behavior is identical field by field.
/// POST /cli/search - 本地客户端查询入口。
/// 与 CLI 进程内路径调用相同的 searcher 函数,行为逐字段一致
async fn cli_search(Json(req): Json<CliSearchRequest>) -> CommonResponse<CliSearchData> {
    let results = searcher::search_by_type(&req.query, req.search_type.as_deref()).await;
    let total = results.len();
    let results: Vec<_> = results.into_iter().take(req.limit.unwrap_or(10)).collect();
    CommonResponse::success(CliSearchData { results, total })
}

/// POST /cli/similar/:file_id - local similarity query.
/// Calls the same similarity_service function as the CLI's in-process path
/// (includes cross-device aggregation when the cluster is enabled).
/// POST /cli/similar/:file_id - 本地相似查询。
/// 与 CLI 进程内路径调用相同的 similarity_service 函数(集群开启时含跨设备聚合)
async fn cli_similar(
    Path(file_id): Path<i64>,
    Json(req): Json<CliSimilarRequest>,
) -> CommonResponse<CliSimilarData> {
    match similarity_service::find_similars_for_local_file(file_id, req.limit.unwrap_or(10)).await {
        Ok(results) => CommonResponse::success(CliSimilarData { results }),
        Err(e) => CommonResponse::error(
            CODE_INTERNAL_ERROR,
            &format!("Failed to find similar files: {}", e),
        ),
    }
}

/// POST /cli/shutdown - ask the daemon host to exit; a GUI host ignores it
/// (its service follows the GUI's lifetime). Callers distinguish the host
/// by pinging again after shutdown. Auth is the loopback + X-Device-ID
/// middleware on /cli/*.
/// POST /cli/shutdown - 请求 daemon 宿主退出;GUI 宿主忽略(服务随 GUI 生命周期)。
/// 调用方通过 shutdown 后的二次 ping 感知宿主身份。鉴权由 /cli/* 的
/// 回环 + X-Device-ID 中间件承担
/// Wait (bounded at 10s) until the serve task has actually stopped — i.e.
/// all in-flight responses are written and the listener released. Callers
/// use this after `stop_http_server` before exiting the process.
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

async fn cli_shutdown() -> CommonResponse<()> {
    if IS_DAEMON.load(Ordering::Relaxed) {
        tokio::spawn(async {
            // Graceful drain instead of a fixed sleep: stop sends the signal,
            // then we wait until the serve task actually finished (it clears
            // SHUTDOWN_SENDER once all in-flight responses are written). This
            // both flushes our own response and avoids killing other CLIs'
            // in-flight queries. Bounded at 10s for safety.
            // 优雅排水而非固定 sleep:stop 发信号后,等待 serve 任务真正结束
            // (全部在途响应写完后会清空 SHUTDOWN_SENDER)。既保证本响应写回,
            // 也不会杀掉其他 CLI 的在途查询;上限 10s 兜底。
            stop_http_server().await;
            wait_server_stopped().await;
            log::info!("CLI daemon exiting on /cli/shutdown");
            std::process::exit(0);
        });
    }
    CommonResponse::success(())
}

/// Initialize the processed requests cache
fn init_request_cache() {
    let mut cache = PROCESSED_REQUESTS.write().unwrap_or_else(|e| {
        log::error!("PROCESSED_REQUESTS lock poisoned, recovering: {}", e);
        e.into_inner()
    });
    if cache.is_none() {
        *cache = Some(HashMap::new());
    }
}

/// Check if request has been processed, and mark as processed if not
/// Returns true if the request was already processed (should be ignored)
/// Returns false if this is a new request (should be processed)
fn check_and_mark_request(request_id: &str) -> bool {
    let mut cache_guard = PROCESSED_REQUESTS.write().unwrap_or_else(|e| {
        log::error!("PROCESSED_REQUESTS lock poisoned, recovering: {}", e);
        e.into_inner()
    });
    if let Some(ref mut cache) = *cache_guard {
        let now = Instant::now();

        // Check if already processed
        if let Some(&timestamp) = cache.get(request_id) {
            if now.duration_since(timestamp).as_secs() < REQUEST_CACHE_TTL_SECS {
                log::warn!(
                    "Request {} already processed, ignoring to prevent loop",
                    request_id
                );
                return true;
            }
            // Expired, remove old entry
            cache.remove(request_id);
        }

        // Mark as processed
        cache.insert(request_id.to_string(), now);

        // Periodically clean up expired entries (every 100 requests or so)
        if cache.len() > 100 {
            cache.retain(|_, &mut timestamp| {
                now.duration_since(timestamp).as_secs() < REQUEST_CACHE_TTL_SECS
            });
        }

        false
    } else {
        // Cache not initialized, allow request
        false
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
fn create_exclusive_socket(port: i32) -> Result<tokio::net::TcpListener, std::io::Error> {
    create_exclusive_socket_on(SocketAddr::from(([0, 0, 0, 0], port as u16)))
}

/// Create a socket with exclusive address use on the given address
/// 创建独占地址的 socket，绑定到指定地址
fn create_exclusive_socket_on(addr: SocketAddr) -> Result<tokio::net::TcpListener, std::io::Error> {
    // Create socket with socket2
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;

    // On Windows, set SO_EXCLUSIVEADDRUSE to prevent other sockets from binding to the same port
    // 在 Windows 上设置 SO_EXCLUSIVEADDRUSE 防止其他 socket 绑定同一端口
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
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
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
    // 检查服务器是否已经在运行
    if server_already_running() {
        log::info!("HTTP server is already running, skip start");
        return Ok(());
    }

    let (port, device_id, device_name) = resolve_identity().await;

    // Bind with exclusive access; if the port is held by our own CLI daemon,
    // ask it to yield first (close-then-start, so the daemon's model memory is
    // released before this process warms up its own). Retry a few times: a
    // concurrent CLI invocation may spawn a fresh daemon into the gap between
    // the old daemon's exit and our rebind — yield it too instead of failing.
    // 绑定失败时先尝试请自家 CLI daemon 让位(先关后开,避免双份模型常驻)。
    // 重试数次:并发 CLI 命令可能在旧 daemon 退出与本进程重绑的间隙里拉起新
    // daemon —— 对它继续让位,而不是直接失败。
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

/// Clamp to the valid port range (single source shared with the CLI's
/// local_port): out-of-range values fall back to the default so `as u16`
/// truncation can never make bind/probe/check disagree.
/// 钳制到合法端口区间(与 CLI 的 local_port 共用的单一来源):超范围值统一
/// 回退默认端口,`as u16` 截断不会让绑定/探测/check 落在不同端口上。
pub fn clamp_port(port: i32) -> i32 {
    if (1..=65535).contains(&port) {
        port
    } else {
        DEFAULT_PORT
    }
}

/// Resolve the shared server identity: port (DB cluster_setting with the
/// typed default) plus device_id/device_name for ServerState.
/// 解析公共服务身份:端口(DB cluster_setting,类型默认值兜底)与
/// ServerState 所需的 device_id/device_name。
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

/// Start the HTTP server bound to loopback only (CLI daemon host, `mfd serve`)
/// 以 CLI daemon 宿主身份启动,仅绑定回环地址,不向局域网暴露
pub async fn start_http_server_loopback() -> Result<(), String> {
    if server_already_running() {
        log::info!("HTTP server is already running, skip start");
        return Ok(());
    }
    let (port, device_id, device_name) = resolve_identity().await;

    // The daemon never yields: a busy port means a host already exists,
    // which the CLI-side single-instance check guards against.
    // daemon 不做让位:端口被占说明已有宿主,由 CLI 侧的单实例检查兜底
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
    listener: tokio::net::TcpListener,
    device_id: String,
    device_name: String,
    port: i32,
) -> Result<(), String> {
    let local_addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| format!("port {}", port));
    log::info!("HTTP server listening on {}", local_addr);

    init_request_cache();

    let state = Arc::new(ServerState {
        device_id,
        device_name,
        port,
    });

    // /cli local API: local clients only, loopback + local CLIENT_ID double-check
    // /cli 本地 API:仅供本机客户端,回环 + 本机 CLIENT_ID 双校验
    let cli_routes = Router::new()
        .route("/cli/search", post(cli_search))
        .route("/cli/similar/:file_id", post(cli_similar))
        .route("/cli/shutdown", post(cli_shutdown))
        .layer(middleware::from_fn(verify_loopback));

    let app = Router::new()
        .route("/ping", get(handle_ping))
        // Inter-device protocol gets a /cluster prefix, separated from the
        // /cli (local) namespace. No legacy-path compatibility: devices are
        // expected to upgrade together.
        // 设备间协议加 /cluster 前缀,与 /cli(本机)命名空间区分;
        // 不保留旧路径兼容:多设备约定一起升级
        .route("/cluster/search", post(handle_search))
        .route("/cluster/chunks", post(handle_chunks))
        .route(
            "/cluster/file/:file_id/similars",
            post(handle_find_similars),
        )
        .route("/cluster/file/:file_id", get(handle_file_download))
        .route("/cluster/file/:file_id/content", get(handle_file_content))
        .route("/cluster/pairing/request", post(handle_pairing_request))
        .route("/cluster/pairing/respond", post(handle_pairing_respond))
        .route("/cluster/pairing/reset_notify", post(handle_reset_notify))
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
        // The server has actually stopped: clear the sender so a later
        // start_http_server (port change, disable→re-enable) can bind again
        // instead of seeing a stale "already running" flag.
        // 服务已实际停止:清空 sender,后续 start_http_server(改端口、
        // 关闭再开启集群)才能重新绑定,而不是被残留标记误判为已运行
        if let Ok(mut guard) = SHUTDOWN_SENDER.write() {
            *guard = None;
        }
    });

    Ok(())
}

/// Port held by our own CLI daemon: ask it to exit and wait until it really
/// does (probe no longer `Ours`) before the caller rebinds — close first,
/// then start. The probe/shutdown handshake is shared with the CLI via
/// `cluster::local_probe`.
/// 端口被自家 CLI daemon 占用时:请求其退出并等待端口真正释放(探测不再是
/// 自家服务),再由调用方重绑 —— 先关后开。探测/关闭握手与 CLI 共用
/// `cluster::local_probe`。
async fn try_yield_to_cli_daemon(port: i32) -> bool {
    use super::local_probe::{self, LocalProbe};

    // Only our own daemon yields; a GUI host or a foreign holder does not
    // 只有自家 daemon 会退出;GUI 宿主或外部占用者不会让位
    if local_probe::probe(port).await != LocalProbe::Ours {
        return false;
    }
    let _ = local_probe::request_shutdown(port).await;

    // Wait for the real exit (up to 12s, two consecutive non-Ours probes):
    // the daemon's graceful drain is itself bounded at 10s (in-flight
    // responses), so a 5s budget would misjudge a legitimately draining
    // daemon as stuck. Mirrors handle_shutdown's guard so a momentarily slow
    // /ping cannot be misread as "exited".
    // 等待真正退出(上限 12s,连续两次探测非自家):daemon 的优雅排水上限
    // 10s(在途响应),5s 预算会把正常排水中的 daemon 误判为卡死。与
    // handle_shutdown 的防护一致,单次慢 /ping 不会被误读为"已退出"。
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut consecutive_gone = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if local_probe::probe(port).await != LocalProbe::Ours {
            consecutive_gone += 1;
            if consecutive_gone >= 2 {
                log::info!("CLI daemon on port {} has yielded", port);
                return true;
            }
        } else {
            consecutive_gone = 0;
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    log::warn!("CLI daemon on port {} did not exit in time", port);
    false
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
    // 注意：不要在这里清空 SHUTDOWN_SENDER，让服务器实际停止时再清空
}

/// GET /ping - Health check and device info (merged with /device/info)
/// GET /ping - 健康检查和设备信息（合并了 /device/info）
async fn handle_ping(State(state): State<Arc<ServerState>>) -> CommonResponse<PingData> {
    log::info!("GET /ping");
    let index_count = file_info_repo::count().unwrap_or(0);
    let capabilities = DeviceCapabilities::default();

    CommonResponse::success(PingData {
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
        status: "online".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        index_count,
        last_index_time: None,
        capabilities,
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

/// POST /search - Remote search request
async fn handle_search(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(req): Json<RemoteSearchRequest>,
) -> CommonResponse<SearchData> {
    log::info!(
        "POST /cluster/search query={}, type={}, limit={}",
        req.query,
        req.search_type,
        req.limit
    );
    // Verify requester is paired
    let _requester_id: &str = match verify_paired_device(&headers) {
        Ok(id) => id,
        Err(e) => return e,
    };

    // Execute local search
    let results = match req.search_type.as_str() {
        "keyword" => searcher::keyword_search(&req.query).await,
        _ => {
            // Default to semantic search
            searcher::semantic_search(&req.query).await
        }
    };

    // Limit results
    let results: Vec<_> = results.into_iter().take(req.limit).collect();
    let total = results.len();

    CommonResponse::success(SearchData {
        results,
        total,
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
    })
}

/// POST /chunks - Get text chunks by IDs for remote devices
/// POST /chunks - 根据ID获取文本片段（供远程设备调用）
async fn handle_chunks(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(req): Json<ChunksRequest>,
) -> CommonResponse<ChunksData> {
    log::info!("POST /chunks ids={:?}", req.ids);
    // Verify requester is paired
    let _requester_id: &str = match verify_paired_device(&headers) {
        Ok(id) => id,
        Err(e) => return e,
    };

    // Get chunks by IDs
    let chunks = match file_content_embedding_repo::list_chunks_by_ids(&req.ids) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to get chunks: {}", e);
            return CommonResponse::error(CODE_INTERNAL_ERROR, "Failed to get chunks");
        }
    };

    CommonResponse::success(ChunksData {
        chunks,
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
    })
}

/// POST /file/:file_id/cluster_similars - Find similar files across devices with dual loop prevention
/// POST /file/:file_id/cluster_similars - 跨设备查找相似文件（双重防循环机制）
///
/// This endpoint is called by remote devices to find similar files.
/// It searches locally and forwards the request to other devices.
///
/// Dual loop prevention (双重保证防止死循环):
/// 1. exclude_device_ids: Devices to skip when forwarding requests
///    排除列表：转发请求时跳过的设备
/// 2. request_id: Unique ID cached for 10 minutes to prevent reprocessing
///    请求ID：缓存10分钟，防止重复处理同一请求
async fn handle_find_similars(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(file_id): Path<i64>,
    Json(req): Json<RemoteFindSimilarsRequest>,
) -> CommonResponse<FindSimilarsData> {
    log::info!(
        "POST /file/{}/cluster_similars request_id={}, limit={}",
        file_id,
        req.request_id,
        req.limit
    );

    // Verify requester is paired first
    let requester_id: &str = match verify_paired_device(&headers) {
        Ok(id) => id,
        Err(e) => return e,
    };

    // Check if this request has already been processed (prevent loops)
    // 检查此请求是否已被处理过（防止循环）
    if check_and_mark_request(&req.request_id) {
        log::warn!(
            "Request {} already processed, returning empty results to prevent loop",
            req.request_id
        );
        return CommonResponse::success(FindSimilarsData {
            results: vec![],
            total: 0,
            device_id: state.device_id.clone(),
            device_name: state.device_name.clone(),
        });
    }

    // Get file info
    let file_info = match file_info_repo::get_by_id(file_id) {
        Ok(Some(f)) => f,
        Ok(None) => {
            log::warn!("File not found: {}", file_id);
            return CommonResponse::error(
                CODE_NOT_FOUND,
                &t!("message.file-not-found").to_string(),
            );
        }
        Err(e) => {
            log::error!("Failed to get file: {}", e);
            return CommonResponse::error(
                CODE_INTERNAL_ERROR,
                &t!("message.internal-error").to_string(),
            );
        }
    };

    let limit = req.limit;

    // Build exclude list: add ourselves and the requester
    let mut exclude_device_ids = req.exclude_device_ids.clone();
    exclude_device_ids.push(state.device_id.clone());
    if !requester_id.is_empty() && !exclude_device_ids.contains(&requester_id.to_string()) {
        exclude_device_ids.push(requester_id.to_string());
    }

    // 1. Search locally
    let local_results = match crate::similarity::local_similarity_service::find_similars_by_file_id(
        &file_info, limit,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to find local similar files: {}", e);
            return CommonResponse::error(
                CODE_INTERNAL_ERROR,
                &t!("message.internal-error").to_string(),
            );
        }
    };

    // 2. Search on other remote devices (with updated exclude list)
    //    Pass the same request_id to prevent loops
    let remote_results =
        crate::similarity::remote_similarity_service::remote_find_similars_with_exclude(
            &file_info,
            limit,
            &exclude_device_ids,
            &req.request_id,
        )
        .await;

    // 3. Merge results
    let mut all_results = local_results;
    all_results.extend(remote_results);
    all_results.sort_by(|a, b| b.score.cmp(&a.score));
    all_results.truncate(limit);
    let total = all_results.len();

    CommonResponse::success(FindSimilarsData {
        results: all_results,
        total,
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
    })
}

/// GET /file/:file_id - Download file from this device by file ID
/// Only indexed files can be downloaded (security measure)
async fn handle_file_download(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(file_id): Path<i64>,
) -> Result<Response<Body>, StatusCode> {
    log::info!("GET /file/{}", file_id);
    // Verify requester is paired
    let requester_id = verify_paired_device_status(&headers)?;

    // Get file info from database by file_id (only indexed files)
    let file_info = file_info_repo::get_by_id(file_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let file_path = &file_info.path;

    log::info!(
        "Remote file request from {}: {} (id: {})",
        requester_id,
        file_path,
        file_id
    );

    // Read file data (limit to 200 MB to avoid OOM on large files)
    const MAX_DOWNLOAD_SIZE: u64 = 200 * 1024 * 1024;
    let file_meta = std::fs::metadata(file_path).map_err(|e| {
        log::error!("Failed to stat file {}: {}", file_path, e);
        StatusCode::NOT_FOUND
    })?;
    if file_meta.len() > MAX_DOWNLOAD_SIZE {
        log::warn!(
            "File too large for download ({} bytes): {}",
            file_meta.len(),
            file_path
        );
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let file_data = std::fs::read(file_path).map_err(|e| {
        log::error!("Failed to read file {}: {}", file_path, e);
        StatusCode::NOT_FOUND
    })?;

    // Determine content type based on extension
    let content_type = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext.to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "html" | "htm" => "text/html",
            "json" => "application/json",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            _ => "application/octet-stream",
        })
        .unwrap_or("application/octet-stream");

    // Build response with file data
    // Sanitize filename and encode per RFC 5987 to handle special characters and prevent header injection
    let safe_filename = file_info
        .name
        .replace('\\', "")
        .replace('"', "")
        .replace('\r', "")
        .replace('\n', "");
    let encoded_filename: String = safe_filename
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
                vec![b as char]
            } else {
                format!("%{:02X}", b).chars().collect()
            }
        })
        .collect();
    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        safe_filename, encoded_filename
    );

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header("X-Device-Id", &state.device_id)
        .header("X-Device-Name", &state.device_name)
        .body(Body::from(file_data))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

/// GET /file/:file_id/content - Get file content/parsed content by file ID for remote devices
/// GET /file/:file_id/content - 获取文件内容/解析内容（供远程设备调用）
async fn handle_file_content(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(file_id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    log::info!("GET /file/{}/content", file_id);
    // Verify requester is paired
    verify_paired_device_status(&headers)?;

    // Get file info from database
    let mut file_info = file_info_repo::get_by_id(file_id)
        .map_err(|e| {
            log::error!("Failed to get file info: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // In "file" storage mode, content_ref_path points to a .md file — resolve to actual content
    if let Some(ref relative_path) = file_info.content_ref_path {
        if let Some(storage) = STORAGE_PATH.get() {
            let md_path = std::path::Path::new(storage).join(relative_path);
            file_info.content = std::fs::read_to_string(&md_path).unwrap_or_else(|e| {
                log::warn!(
                    "Failed to read parsed document {}: {}",
                    md_path.display(),
                    e
                );
                String::new()
            });
        } else {
            file_info.content = String::new();
        }
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "content": file_info.content,
            "deviceId": state.device_id,
            "deviceName": state.device_name
        }
    })))
}

/// Notify frontend about pairing request handling result
fn notify_pairing_handled(payload: &PairingRequestPayload, status: &str) {
    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit(
            "pairing-request-handled",
            json!({
                "status": status,
                "device_id": payload.device_id,
                "device_name": payload.device_name,
                "ip_address": payload.ip_address,
                "port": payload.port
            }),
        );
    }
}

/// Check if device already exists and return its status
/// Returns Some(response) if device exists and should be handled early
fn check_existing_device(
    payload: &PairingRequestPayload,
) -> Option<Result<Json<Value>, StatusCode>> {
    let device = device_repo::get_by_device_id(&payload.device_id).ok()??;

    match device.pairing_status {
        PairingStatus::Paired => {
            // Log the request as already_paired
            let request = PairingRequest {
                device_id: payload.device_id.clone(),
                device_name: payload.device_name.clone(),
                ip_address: payload.ip_address.clone(),
                port: payload.port,
                direction: "in".to_string(),
                status: PairingRequestStatus::Accepted,
                remark: t!("pairing.remark.already-paired").to_string(),
                ..Default::default()
            };
            let _ = pairing_request_repo::insert(&request);

            notify_pairing_handled(payload, "already_paired");
            Some(Ok(Json(json!({
                "code": 0,
                "data": {
                    "status": PairingResponseStatus::AlreadyPaired,
                    "message": "Device is already paired"
                }
            }))))
        }
        PairingStatus::Rejected => {
            log::info!(
                "Pairing request from rejected device {}, auto-rejecting",
                payload.device_id
            );

            // Log the request as auto_rejected
            let request = PairingRequest {
                device_id: payload.device_id.clone(),
                device_name: payload.device_name.clone(),
                ip_address: payload.ip_address.clone(),
                port: payload.port,
                direction: "in".to_string(),
                status: PairingRequestStatus::AutoRejected,
                remark: t!("pairing.remark.auto-rejected").to_string(),
                ..Default::default()
            };
            let _ = pairing_request_repo::insert(&request);

            notify_pairing_handled(payload, "auto_rejected");
            Some(Ok(Json(json!({
                "code": 0,
                "data": {
                    "status": PairingResponseStatus::Rejected,
                    "message": "Device is rejected"
                }
            }))))
        }
        _ => None,
    }
}

/// Handle auto-pair scenario when both sides sent requests to each other
/// Returns Some(response) if auto-pair is applicable
fn handle_auto_pair(payload: &PairingRequestPayload) -> Option<Result<Json<Value>, StatusCode>> {
    let out_request =
        pairing_request_repo::get_pending_by_device_id(&payload.device_id, "out").ok()??;

    log::info!(
        "Auto-pairing: both sides sent requests to each other, accepting {}",
        payload.device_id
    );

    // Update device to paired status (automatic process)
    if let Ok(Some(device)) = device_repo::get_by_device_id(&payload.device_id) {
        let remark = t!("pairing.remark.auto-paired");
        let _ = device_repo::update_pairing_status_with_remark(
            device.id,
            PairingStatus::Paired,
            &remark,
            false,
        );
    }

    // Update outgoing request to accepted
    let _ = pairing_request_repo::accept(out_request.id);

    // Log incoming request as accepted
    let request = PairingRequest {
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        direction: "in".to_string(),
        status: PairingRequestStatus::Accepted,
        remark: t!("pairing.remark.auto-paired").to_string(),
        ..Default::default()
    };
    let _ = pairing_request_repo::insert(&request);

    notify_pairing_handled(payload, "auto_accepted");
    Some(Ok(Json(json!({
        "code": 0,
        "data": {
            "status": PairingResponseStatus::Approved,
            "message": "Auto-paired: both sides requested pairing"
        }
    }))))
}

/// Handle auto-accept scenario when auto_accept_pairing is enabled
/// Returns Some(response) if auto-accept is applicable
async fn handle_auto_accept(
    payload: &PairingRequestPayload,
) -> Option<Result<Json<Value>, StatusCode>> {
    let config = super::get_cluster_setting().await;
    if !config.auto_accept_pairing {
        return None;
    }

    // Auto-accept
    let device = Device {
        device_id: payload.device_id.clone(),
        name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        online_status: OnlineStatus::Online,
        pairing_status: PairingStatus::Paired,
        discovery_method: "mdns".to_string(),
        ..Default::default()
    };
    let inserted_device = device_repo::upsert(&device).ok()?;

    // Update pairing status with remark (upsert doesn't set pairing_remark)
    let remark = t!("pairing.remark.auto-accepted");
    let _ = device_repo::update_pairing_status_with_remark(
        inserted_device.id,
        PairingStatus::Paired,
        &remark,
        false,
    );

    // Log the request
    let request = PairingRequest {
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        direction: "in".to_string(),
        status: PairingRequestStatus::Accepted,
        remark: t!("pairing.remark.auto-accepted").to_string(),
        ..Default::default()
    };
    let _ = pairing_request_repo::insert(&request);

    notify_pairing_handled(payload, "auto_accepted");
    Some(Ok(Json(json!({
        "code": 0,
        "data": {
            "status": PairingResponseStatus::Approved,
            "message": "Pairing auto-approved"
        }
    }))))
}

/// Handle pending request scenario - create pending request and notify frontend
fn handle_pending_request(payload: &PairingRequestPayload) -> Result<Json<Value>, StatusCode> {
    // Log incoming pairing request
    let request = PairingRequest {
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        direction: "in".to_string(),
        status: PairingRequestStatus::Pending,
        remark: t!("pairing.remark.waiting-approval").to_string(),
        ..Default::default()
    };
    pairing_request_repo::insert(&request).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create/update device record with pending_in status
    let device = Device {
        device_id: payload.device_id.clone(),
        name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        online_status: OnlineStatus::Online,
        pairing_status: PairingStatus::PendingIn,
        discovery_method: "mdns".to_string(),
        ..Default::default()
    };
    let upserted_device =
        device_repo::upsert(&device).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Update pairing status with remark (upsert doesn't set pairing_remark)
    let remark = t!("pairing.remark.waiting-approval").to_string();
    let _ = device_repo::update_pairing_status_with_remark(
        upserted_device.id,
        PairingStatus::PendingIn,
        &remark,
        false,
    );

    // Notify frontend about pending request
    notify_pairing_handled(payload, "pending");

    Ok(Json(json!({
        "code": 0,
        "data": {
            "status": PairingResponseStatus::Pending,
            "message": "Pairing request received, waiting for approval"
        }
    })))
}

/// POST /pairing/request - Handle incoming pairing request
async fn handle_pairing_request(
    State(_state): State<Arc<ServerState>>,
    Json(payload): Json<PairingRequestPayload>,
) -> Result<Json<Value>, StatusCode> {
    log::info!(
        "Received pairing request from {} ({})",
        payload.device_name,
        payload.ip_address
    );

    // 1. Check if device already exists (paired or rejected)
    if let Some(result) = check_existing_device(&payload) {
        return result;
    }

    // 2. Check for auto-pair scenario (both sides sent requests)
    if let Some(result) = handle_auto_pair(&payload) {
        return result;
    }

    // 3. Check if auto_accept_pairing is enabled
    if let Some(result) = handle_auto_accept(&payload).await {
        return result;
    }

    // 4. Default: create pending request
    handle_pending_request(&payload)
}

/// POST /pairing/respond - Handle pairing response (from remote device)
async fn handle_pairing_respond(
    Json(payload): Json<PairingResponsePayload>,
) -> Result<Json<Value>, StatusCode> {
    log::info!(
        "Received pairing response from {}: approved={}",
        payload.responder_id,
        payload.approved
    );

    // Update device pairing status - use responder_id to find the remote device
    if let Ok(Some(device)) = device_repo::get_by_device_id(&payload.responder_id) {
        let (new_status, remark) = if payload.approved {
            (
                PairingStatus::Paired,
                t!("pairing.remark.remote-accepted").to_string(),
            )
        } else {
            // Remote device rejected our request, mark as blocked
            // 对方拒绝了本机的请求，标记为 blocked
            (
                PairingStatus::Blocked,
                t!("pairing.remark.remote-rejected").to_string(),
            )
        };
        log::info!("Updating device pairing status to {:?}", new_status);
        device_repo::update_pairing_status_with_remark(device.id, new_status, &remark, false)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Update the latest request status (any direction) - use responder_id to find the request
    // 更新最新的请求状态（不限方向）- 使用 responder_id 查找请求
    if let Ok(Some(latest_request)) =
        pairing_request_repo::get_latest_by_device_id_any(&payload.responder_id)
    {
        let (status, remark) = if payload.approved {
            (
                PairingRequestStatus::Accepted,
                t!("pairing.remark.remote-accepted").to_string(),
            )
        } else {
            (
                PairingRequestStatus::Rejected,
                t!("pairing.remark.remote-rejected").to_string(),
            )
        };
        let _ = pairing_request_repo::update_status_with_remark(latest_request.id, status, &remark);
    }

    // Notify frontend about pairing response
    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit("pairing-response-received", &payload);
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "status": if payload.approved { "approved" } else { "rejected" }
        }
    })))
}

/// POST /pairing/reset_notify - Handle reset notification from remote device
/// POST /pairing/reset_notify - 处理远程设备的重置通知
///
/// When a remote device resets its pairing status, it notifies this device.
/// This device should also reset its pairing status for that device.
/// 当远程设备重置配对状态时，它会通知本设备。
/// 本设备也应该重置该设备的配对状态。
async fn handle_reset_notify(
    Json(payload): Json<ResetNotifyPayload>,
) -> Result<Json<Value>, StatusCode> {
    log::info!(
        "Received reset notify from {} (previous status: {})",
        payload.from_device_id,
        payload.previous_status
    );

    // Get local device record for this remote device
    // 获取该远程设备的本机记录
    if let Ok(Some(device)) = device_repo::get_by_device_id(&payload.from_device_id) {
        // Reset pairing status to None with remark
        // 重置配对状态为 None 并附带说明
        // Note: is_manual=true because remote reset is triggered by user action on remote device
        // 注意: is_manual=true 因为远程重置是由对方设备上的用户操作触发的
        let remark = t!("pairing.remark.remote-reset").to_string();
        device_repo::update_pairing_status_with_remark(
            device.id,
            PairingStatus::None,
            &remark,
            true,
        )
        .map_err(|e| {
            log::error!(
                "Failed to update pairing status for device {}: {:?}",
                payload.from_device_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        // Update the latest pairing request record for this device
        // 更新该设备最新的配对请求记录
        if let Ok(Some(latest_request)) =
            pairing_request_repo::get_latest_by_device_id_any(&payload.from_device_id)
        {
            let _ = pairing_request_repo::update_status_with_remark(
                latest_request.id,
                PairingRequestStatus::Cancelled,
                &remark,
            );
        }

        // Notify frontend about pairing reset
        if let Some(app_handle) = APP_HANDLE.get() {
            let _ = app_handle.emit("pairing-reset-received", &payload);
        }

        log::info!(
            "Reset pairing status for device {} to None (was notified by remote)",
            payload.from_device_id
        );
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "status": "ok"
        }
    })))
}
