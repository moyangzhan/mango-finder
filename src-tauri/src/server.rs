//! Local-server liveness probing and the exit handshake: probe / request exit / wait until released.
//! Used by the CLI binary, the GUI's yield logic (making the daemon yield before the GUI binds) and `set_data_path` (daemon invalidation on data-dir switch).
//!
//! 本机 server 的存活探测与退出握手:探测 / 请求退出 / 等待释放。
//! CLI 二进制、GUI 让位逻辑(GUI 绑定前请 daemon 让位)与 `set_data_path`(数据目录切换时的失效通知)共用。

use crate::cluster::api_types::CommonResponse;
use crate::loopback_request::{client_id, post_common_response};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// Probe timeout: also the worst-case cost when nothing is listening
/// 探测超时:无服务监听时的最坏耗时
const PROBE_TIMEOUT_MS: u64 = 600;

static PROBE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        // Loopback requests must bypass the system proxy: local proxies often reject 127.0.0.1 targets, which makes every probe time out.
        .no_proxy()
        .timeout(Duration::from_millis(PROBE_TIMEOUT_MS))
        .build()
        .expect("failed to build local probe client")
});

/// Probe outcome for a local port
/// 本地端口探测结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Mango Finder's own service answered (device_id matched)
    /// 应答的是 Mango Finder 自身的服务(device_id 匹配)
    Mine,
    /// Something is listening but it is not our service (port collision)
    /// 有进程监听但不是我们的服务(端口撞车)
    Other,
    /// Nothing answered (connection refused / timeout)
    /// 无应答(拒绝连接/超时)
    Unreachable,
}

/// Probe a port for our own local server.
/// 探测端口上是否是本机 server。
pub async fn probe(port: i32) -> ProbeOutcome {
    let own = client_id();
    let url = format!("http://127.0.0.1:{}/ping", port);
    // Timeout means free: on some machines a free port also times out (dropped SYNs); classifying it as Other would break daemon spawning
    let Ok(resp) = PROBE_CLIENT
        .get(url)
        .header("X-Device-ID", &own)
        .send()
        .await
    else {
        return ProbeOutcome::Unreachable;
    };
    if !resp.status().is_success() {
        return ProbeOutcome::Other;
    }
    match resp
        .json::<CommonResponse<crate::cluster::api_types::PingData>>()
        .await
    {
        Ok(parsed) => {
            if parsed.data.map(|d| d.device_id) == Some(own) {
                ProbeOutcome::Mine
            } else {
                ProbeOutcome::Other
            }
        }
        Err(_) => ProbeOutcome::Other,
    }
}

/// Outcome of waiting for the port to stop answering as our own service
/// 等待端口不再应答为自家服务的轮询结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// Two consecutive non-Mine probes within the deadline
    /// 期限内连续两次探测非自家
    Released,
    /// Deadline reached while the port still answers as ours
    /// 超时,端口仍应答为自家服务
    StillHeld,
}

/// Poll until the service on `port` stops answering as ours: two consecutive non-Mine probes (a single slow /ping is not misread as an exit) or the timeout, whichever comes first.
/// Sleeps before the first probe so the just-notified carrier process gets a chance to exit first.
/// 轮询等待端口上的服务不再是自家:连续两次非 Mine(单次慢 /ping 不会误判为退出)或超时,先到为准。
/// 首次探测前先睡眠,给刚收到通知的承载进程一点退出时间。
pub async fn wait_until_released(port: i32, timeout: Duration) -> ReleaseOutcome {
    const POLL_INTERVAL_MS: u64 = 150;
    let deadline = Instant::now() + timeout;
    let mut consecutive_gone = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        if probe(port).await != ProbeOutcome::Mine {
            consecutive_gone += 1;
            if consecutive_gone >= 2 {
                return ReleaseOutcome::Released;
            }
        } else {
            consecutive_gone = 0;
        }
        if Instant::now() >= deadline {
            return ReleaseOutcome::StillHeld;
        }
    }
}

/// Ask the local server to shut down (the daemon exits; a GUI-carried server ignores it). Auth is the loopback + X-Device-ID middleware on /cli/*.
/// 请求本机 server 退出(daemon 退出;GUI 承载的 server 忽略)。鉴权由 /cli/* 的回环 + X-Device-ID 中间件承担。
pub async fn request_shutdown(port: i32) -> Result<(), String> {
    post_common_response::<serde_json::Value>(port, "/cli/shutdown", serde_json::Value::Null)
        .await
        .map(|_| ())
}
