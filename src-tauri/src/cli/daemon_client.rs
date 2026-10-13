//! Server client; when no server is running it spawns a daemon (`daemon_server::run`) to carry it.
//! Duties: detect the server → spawn a daemon on demand → call the /cli endpoints.
//!
//! The server is the resident service answering /cli requests — the HTTP entry from `cluster::http_server` plus the hot resources behind it (path cache, embedding model); it is carried by the GUI process when multi-device is enabled, otherwise by the daemon spawned above.
//! The exclusive bind guarantees at most one local server instance at any time, so a running server is reused as-is.
//!
//! The probe/shutdown handshake is the shared `cluster::server`, also used by the GUI (making the daemon yield before the GUI binds) and by `set_data_path` (daemon invalidation on data-dir switch).
//!
//! server 客户端;server 未运行时拉起一个 daemon(`daemon_server::run`)承载。
//! 职责:探测 server → 按需拉起 daemon → 调用 /cli 端点。
//!
//! server 是应答 /cli 请求的常驻服务——入口为 `cluster::http_server` 的 HTTP 层,门后是路径缓存与嵌入模型等热资源;多机互联开启时由 GUI 进程承载,否则由上述拉起的 daemon 进程承载。
//! 端口独占绑定保证任一时刻本地至多一个 server 实例,探测到已在运行就直接复用。
//!
//! 探测/关闭握手是共享的 `cluster::server`,GUI(绑定前请 daemon 让位)与 `set_data_path`(数据目录切换时的失效通知)也在使用。

use mango_finder_lib::cluster::api_types::{CliSearchData, CliSimilarData};
use mango_finder_lib::cluster::http_request_wrap;
use mango_finder_lib::cluster::server::{self, ProbeOutcome};
use mango_finder_lib::structs::search_result::SearchResult;
use serde_json::json;
use std::time::{Duration, Instant};

/// Upper bound for waiting the spawned daemon to become ready
/// 拉起 daemon 后等待就绪的上限
const DAEMON_READY_TIMEOUT_SECS: u64 = 10;

/// Server port: same source as the GUI (DB cluster_setting; the typed default applies when the row is missing).
/// Clamped to the valid range so probing and the daemon's bind can never disagree via `as u16` truncation.
/// server 端口:与 GUI 同源(DB cluster_setting;行缺失时走类型默认值)。
/// 钳制到合法区间,避免超范围值经 `as u16` 截断后探测与绑定落不同端口。
pub async fn server_port() -> i32 {
    let setting = mango_finder_lib::cluster::get_cluster_setting().await;
    // The clamp is shared with the server (http_server::clamp_port)
    mango_finder_lib::cluster::http_server::clamp_port(setting.port)
}

/// Detect the server (GUI-carried or daemon-carried); returns the port when it is ours
/// 探测 server(GUI 承载或 daemon 承载);是本机服务时返回端口
pub async fn detect() -> Option<i32> {
    detect_port(server_port().await).await
}

/// Detect a specific port (for the server re-check after shutdown)
/// 探测指定端口(供 shutdown 后的 server 复查)
pub async fn detect_port(port: i32) -> Option<i32> {
    matches!(server::probe(port).await, ProbeOutcome::Mine).then_some(port)
}

/// Ensure the server is available: probe → spawn the daemon if absent → poll until ready.
/// Any failure returns None and the caller falls back to the in-process path.
/// Behavior per probe outcome:
/// - `Mine` → use it directly
/// - `Other` → the port is held by another program: fail fast without spawning (otherwise every command would pay the full readiness timeout)
/// - `Unreachable` → spawn `mfd serve` and wait for readiness
/// A non-empty `MANGO_FINDER_NO_DAEMON` skips spawning (probe only).
/// 确保 server 可用:探测 → 不在则拉起 daemon → 轮询就绪,任一步失败返回 None 由调用方回落进程内路径。
/// 探测结果决定行为:自家服务直接用;端口被外部程序占用则快速失败(否则每条命令都白等就绪超时);无监听则拉起并等待。
/// `MANGO_FINDER_NO_DAEMON` 非空时跳过拉起(仅探测)。
pub async fn ensure_server() -> Option<i32> {
    let port = server_port().await;
    match server::probe(port).await {
        ProbeOutcome::Mine => return Some(port),
        ProbeOutcome::Other => {
            log::warn!(
                "port {} is held by another process, skipping the server",
                port
            );
            return None;
        }
        ProbeOutcome::Unreachable => {}
    }
    if std::env::var("MANGO_FINDER_NO_DAEMON")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        log::info!("server absent; MANGO_FINDER_NO_DAEMON set, skip spawning daemon");
        return None;
    }
    spawn_daemon().await?;
    wait_ready(port).await
}

/// Spawn `mfd serve` in the background: no window, decoupled from the caller's lifetime.
/// 后台拉起 `mfd serve`:无窗口、脱离当前进程生命周期。
///
/// On Windows the daemon MUST be launched indirectly via PowerShell
/// Start-Process: a direct spawn with `bInheritHandles=TRUE` makes the daemon inherit the caller's pipe write end (e.g. `mfd search | grep` or an agent capturing output), so the pipe never sees EOF.
/// Start-Process creates the process with a fresh handle table; PowerShell itself exits right after.
/// The path is single-quoted with apostrophes doubled, as PowerShell requires.
/// Unix has no such semantics (Rust sets CLOEXEC by default), direct spawn works; the daemon gets its own process group to survive terminal SIGHUP.
/// Windows 下必须经 PowerShell Start-Process 间接拉起:直接 spawn 时 `bInheritHandles=TRUE` 会让 daemon 连带继承调用方管道的写端(如 `mfd search | grep` 或 agent 捕获输出),管道因此永远等不到 EOF。
/// Start-Process 创建全新句柄表的进程;PowerShell 本身拉起后即退出。
/// 路径按 PowerShell 规则单引号包裹、单引号翻倍转义。
/// Unix 无此语义(Rust 默认 CLOEXEC),直接 spawn 并放入独立进程组以免随终端 SIGHUP 退出。
#[cfg(windows)]
async fn spawn_daemon() -> Option<()> {
    let exe = std::env::current_exe().ok()?;
    // In a PowerShell single-quoted string an apostrophe must be doubled
    let exe = exe.display().to_string().replace('\'', "''");
    let script = format!(
        "Start-Process -WindowStyle Hidden -FilePath '{}' -ArgumentList 'serve'",
        exe
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // spawn_blocking: .status() synchronously waits for PowerShell to exit (hundreds of ms to seconds); must not block a tokio worker.
    tokio::task::spawn_blocking(move || cmd.status())
        .await
        .ok()?
        .ok()
        .filter(|s| s.success())
        .map(|_| ())
}

#[cfg(not(windows))]
async fn spawn_daemon() -> Option<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().ok()?;
    std::process::Command::new(exe)
        .arg("serve")
        // Own process group so the daemon is not killed by SIGHUP when the terminal session exits.
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
        .map(|_| ())
}

/// Poll until the daemon is ready (port resolved once by the caller)
/// 轮询等待 daemon 就绪(端口由调用方一次性解析)
async fn wait_ready(port: i32) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(DAEMON_READY_TIMEOUT_SECS);
    loop {
        if matches!(server::probe(port).await, ProbeOutcome::Mine) {
            log::info!("local daemon ready on port {}", port);
            return Some(port);
        }
        if Instant::now() >= deadline {
            log::warn!("daemon not ready within {}s", DAEMON_READY_TIMEOUT_SECS);
            return None;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Search; returns (results, full match count before applying limit).
/// The POST plumbing lives in `cluster::http_request_wrap::post_and_get_data`.
/// 搜索,返回 (结果, 应用 limit 前的完整匹配数)。POST 解析阶梯在
/// `cluster::http_request_wrap::post_and_get_data`。
pub async fn search(
    query: &str,
    search_type: &str,
    limit: usize,
    port: i32,
) -> Result<(Vec<SearchResult>, usize), String> {
    let data: CliSearchData = http_request_wrap::post_and_get_data(
        port,
        "/cli/search",
        json!({ "query": query, "search_type": search_type, "limit": limit }),
    )
    .await?;
    Ok((data.results, data.total))
}

/// Similarity query for an indexed file
/// 已索引文件相似查询
pub async fn similars(file_id: i64, limit: usize, port: i32) -> Result<Vec<SearchResult>, String> {
    let data: CliSimilarData = http_request_wrap::post_and_get_data(
        port,
        &format!("/cli/similar/{}", file_id),
        json!({ "limit": limit }),
    )
    .await?;
    Ok(data.results)
}

/// Ask the server to shut down (the daemon exits; a GUI-carried server ignores it); shared implementation in `cluster::server`.
/// 请求 server 退出(daemon 退出;GUI 承载的 server 忽略);共享实现在 `cluster::server`。
pub async fn request_shutdown(port: i32) -> Result<(), String> {
    server::request_shutdown(port).await
}
