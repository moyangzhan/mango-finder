//! Client for the local resident host process (GUI server or CLI daemon),
//! always dialed over loopback; used by both the CLI binary and the GUI's
//! yield logic.
//! 本机常驻宿主进程(GUI 服务或 CLI daemon)的客户端,一律走回环;
//! CLI 二进制与 GUI 让位逻辑共用。

use crate::cluster::api_types::CommonResponse;
use crate::global::CLIENT_ID;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// Probe timeout: also the worst-case cost when nothing is listening
/// 探测超时:无服务监听时的最坏耗时
const PROBE_TIMEOUT_MS: u64 = 600;

static PROBE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        // Loopback requests must bypass the system proxy: local proxies often
        // reject 127.0.0.1 targets, which makes every probe time out.
        .no_proxy()
        .timeout(Duration::from_millis(PROBE_TIMEOUT_MS))
        .build()
        .expect("failed to build local probe client")
});

/// Shared request client (60s timeout: the first query may trigger a cold
/// model load on the host side).
/// 共享请求客户端(60s 超时:首次查询可能触发宿主侧模型冷加载)。
static REQUEST_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build local request client")
});

/// Probe outcome for a local port
/// 本地端口探测结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalProbe {
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

fn client_id() -> String {
    crate::read_lock!(CLIENT_ID).clone()
}

/// Probe a port for our own local service.
/// 探测端口上是否是本机服务。
pub async fn probe(port: i32) -> LocalProbe {
    let own = client_id();
    let url = format!("http://127.0.0.1:{}/ping", port);
    // Timeout means free: on some machines a free port also times out
    // (dropped SYNs); classifying it as Other would break daemon spawning
    let Ok(resp) = PROBE_CLIENT
        .get(url)
        .header("X-Device-ID", &own)
        .send()
        .await
    else {
        return LocalProbe::Unreachable;
    };
    if !resp.status().is_success() {
        return LocalProbe::Other;
    }
    match resp
        .json::<CommonResponse<crate::cluster::api_types::PingData>>()
        .await
    {
        Ok(parsed) => {
            if parsed.data.map(|d| d.device_id) == Some(own) {
                LocalProbe::Mine
            } else {
                LocalProbe::Other
            }
        }
        Err(_) => LocalProbe::Other,
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

/// Poll until the service on `port` stops answering as ours: two consecutive
/// non-Mine probes (a single slow /ping is not misread as an exit) or the
/// timeout, whichever comes first. Sleeps before the first probe so the
/// just-notified host gets a chance to exit first.
/// 轮询等待端口上的服务不再是自家:连续两次非 Mine
/// (单次慢 /ping 不会误判为退出)或超时,先到为准。首次探测前先睡眠,给
/// 刚收到通知的宿主一点退出时间。
pub async fn wait_until_released(port: i32, timeout: Duration) -> ReleaseOutcome {
    const POLL_INTERVAL_MS: u64 = 150;
    let deadline = Instant::now() + timeout;
    let mut consecutive_gone = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        if probe(port).await != LocalProbe::Mine {
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

/// POST to a /cli endpoint and return the parsed CommonResponse (shared
/// error ladder; data may be empty).
/// 向 /cli 端点 POST 并返回解析后的 CommonResponse(共享错误阶梯;data 可为空)。
async fn post_common_response<T: serde::de::DeserializeOwned>(
    port: i32,
    path: &str,
    body: serde_json::Value,
) -> Result<CommonResponse<T>, String> {
    let url = format!("http://127.0.0.1:{}{}", port, path);
    let resp = REQUEST_CLIENT
        .post(url)
        .header("X-Device-ID", client_id())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP error: {}", resp.status()));
    }
    let parsed: CommonResponse<T> = resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;
    if parsed.code != 0 {
        return Err(format!("API error: code {} - {}", parsed.code, parsed.msg));
    }
    Ok(parsed)
}

/// POST to a /cli endpoint and extract non-empty data as T.
/// 向 /cli 端点 POST 并取非空 data 解析为 T。
pub async fn post_and_get_data<T: serde::de::DeserializeOwned>(
    port: i32,
    path: &str,
    body: serde_json::Value,
) -> Result<T, String> {
    post_common_response(port, path, body)
        .await?
        .data
        .ok_or_else(|| "No data in response".to_string())
}

/// Ask the local service host to shut down (the daemon exits; a GUI host
/// ignores it). Auth is the loopback + X-Device-ID middleware on /cli/*.
/// 请求本地宿主退出(daemon 退出;GUI 宿主忽略)。鉴权由 /cli/* 的
/// 回环 + X-Device-ID 中间件承担。
pub async fn request_shutdown(port: i32) -> Result<(), String> {
    post_common_response::<serde_json::Value>(port, "/cli/shutdown", serde_json::Value::Null)
        .await
        .map(|_| ())
}
