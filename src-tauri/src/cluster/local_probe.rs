//! Shared probe/shutdown client for the loopback local service
//! (the GUI's server or the CLI-spawned daemon).
//! 回环本地服务的共享探测/关闭客户端(GUI 服务或 CLI 拉起的 daemon)。
//!
//! Both the CLI binary (`cli/local_api.rs`) and the GUI's yield logic
//! (`http_server::try_yield_to_cli_daemon`) use this module, so the
//! detection handshake lives in exactly one place.
//! CLI 二进制与 GUI 让位逻辑共用本模块,探测握手只维护一份。

use crate::cluster::api_types::CommonResponse;
use crate::global::CLIENT_ID;
use std::sync::LazyLock;
use std::time::Duration;

/// Probe timeout: also the worst-case cost when nothing is listening
/// 探测超时:无服务监听时的最坏耗时
const PROBE_TIMEOUT_MS: u64 = 600;

static PROBE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        // Loopback requests must bypass the system proxy: local proxies often
        // reject 127.0.0.1 targets, which makes every probe time out.
        // 回环请求不走系统代理:本地代理常拒绝 127.0.0.1 目标,导致探测全部超时
        .no_proxy()
        .timeout(Duration::from_millis(PROBE_TIMEOUT_MS))
        .build()
        .expect("failed to build local probe client")
});

/// Shared request client (60s timeout: the first query may trigger a cold
/// model load on the host side). One client for the whole local API surface
/// so connection pools and policies live in one place.
/// 共享请求客户端(60s 超时:首次查询可能触发宿主侧模型冷加载)。
/// 本地 API 全部请求共用一个客户端,连接池与策略只维护一处。
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
/// The response is parsed by type: PingData is camelCase-serialized, so
/// hand-written field paths would silently break.
/// 探测端口上是否是本机服务。按类型反序列化:PingData 是 camelCase,
/// 手写字段路径会悄悄失效。
pub async fn probe(port: i32) -> LocalProbe {
    let own = client_id();
    let url = format!("http://127.0.0.1:{}/ping", port);
    // 超时视为端口空闲:有的机器上空闲端口也表现为超时(SYN 被丢弃),
    // 若归为 Other 会导致 daemon 永远无法拉起
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
        // Something answered HTTP but not with our /ping shape
        // 有 HTTP 应答但不是我们的 /ping
        return LocalProbe::Other;
    }
    match resp
        .json::<CommonResponse<crate::cluster::api_types::PingData>>()
        .await
    {
        Ok(envelope) => {
            if envelope.data.map(|d| d.device_id) == Some(own) {
                LocalProbe::Mine
            } else {
                LocalProbe::Other
            }
        }
        Err(_) => LocalProbe::Other,
    }
}

/// POST to a /cli endpoint and parse the CommonResponse envelope (shared
/// error ladder; data may be empty).
/// 向 /cli 端点 POST 并解析 CommonResponse 信封(共享错误阶梯;data 可为空)。
async fn post_envelope<T: serde::de::DeserializeOwned>(
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
    let envelope: CommonResponse<T> = resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;
    if envelope.code != 0 {
        return Err(format!(
            "API error: code {} - {}",
            envelope.code, envelope.msg
        ));
    }
    Ok(envelope)
}

/// POST to a /cli endpoint and extract non-empty data as T.
/// 向 /cli 端点 POST 并取非空 data 解析为 T。
pub async fn post_json<T: serde::de::DeserializeOwned>(
    port: i32,
    path: &str,
    body: serde_json::Value,
) -> Result<T, String> {
    post_envelope(port, path, body)
        .await?
        .data
        .ok_or_else(|| "No data in response".to_string())
}

/// Ask the local service host to shut down (the daemon exits; a GUI host
/// ignores it). Auth is the loopback + X-Device-ID middleware on /cli/*.
/// 请求本地宿主退出(daemon 退出;GUI 宿主忽略)。鉴权由 /cli/* 的
/// 回环 + X-Device-ID 中间件承担。
pub async fn request_shutdown(port: i32) -> Result<(), String> {
    post_envelope::<serde_json::Value>(port, "/cli/shutdown", serde_json::Value::Null)
        .await
        .map(|_| ())
}
