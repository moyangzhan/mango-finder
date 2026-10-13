//! Generic HTTP request wrapper for the local server: shared 60s client, X-Device-ID header, CommonResponse error ladder.
//! 本机 server 的通用 HTTP 请求封装:共享 60s 客户端、X-Device-ID 头、CommonResponse 错误阶梯。

use crate::cluster::api_types::CommonResponse;
use crate::global::CLIENT_ID;
use std::sync::LazyLock;
use std::time::Duration;

/// Shared request client (60s timeout: the first query may trigger a cold model load on the server side).
/// 共享请求客户端(60s 超时:首次查询可能触发 server 侧模型冷加载)。
static REQUEST_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build local request client")
});

/// X-Device-ID header source: the local client_id global
/// X-Device-ID 头来源:本机 client_id 全局
pub(crate) fn client_id() -> String {
    crate::read_lock!(CLIENT_ID).clone()
}

/// POST to a /cli endpoint and return the parsed CommonResponse (shared error ladder; data may be empty).
/// 向 /cli 端点 POST 并返回解析后的 CommonResponse(共享错误阶梯;data 可为空)。
pub(crate) async fn post_common_response<T: serde::de::DeserializeOwned>(
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
