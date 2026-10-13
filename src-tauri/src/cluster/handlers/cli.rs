//! /cli endpoints for local on-machine clients (the CLI binary)
//! /cli 端点:本机客户端(CLI 二进制)

use crate::cluster::api_types::{
    CODE_INTERNAL_ERROR, CliSearchData, CliSearchRequest, CliSimilarData, CliSimilarRequest,
    CommonResponse,
};
use crate::cluster::http_server::{
    IS_DAEMON, stop_http_server, touch_cli_request, wait_server_stopped,
};
use crate::global::CLIENT_ID;
use crate::searcher;
use crate::similarity::similarity_service;
use axum::{
    Json,
    extract::{ConnectInfo, Path, Request},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;

/// /cli auth: loopback peer, X-Device-ID must be the local CLIENT_ID, and
/// Host must be a loopback literal.
/// The Host check blocks DNS-rebinding pages, which do connect from 127.0.0.1 and can read device_id from the open /ping.
/// Local processes share the DB's trust domain, so this suffices.
/// /cli 鉴权:回环连接、X-Device-ID 须为本机 CLIENT_ID、Host 须为回环字面量。
/// Host 校验阻断 DNS rebinding(此类页面确实来自 127.0.0.1 且能从无鉴权
/// /ping 读到 device_id);本机进程与数据库同信任域,该校验已足够。
pub async fn verify_loopback(req: Request, next: Next) -> Response {
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

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    // IPv6 literal with port: "[::1]:port"
    if let Some(rest) = host.strip_prefix("[::1]") {
        return rest.is_empty() || rest.starts_with(':');
    }
    let name = host.split(':').next().unwrap_or("");
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1"
}

/// Same searcher dispatch as the CLI's in-process path; output identical field by field.
/// 与 CLI 进程内路径同一分发,输出逐字段一致
pub async fn cli_search(Json(req): Json<CliSearchRequest>) -> CommonResponse<CliSearchData> {
    let results = searcher::search_by_type(&req.query, req.search_type.as_deref()).await;
    let total = results.len();
    let results: Vec<_> = results.into_iter().take(req.limit.unwrap_or(10)).collect();
    CommonResponse::success(CliSearchData { results, total })
}

/// Same similarity_service call as the CLI's in-process path (cross-device aggregation included when the cluster is enabled).
/// 与 CLI 进程内路径同一调用(集群开启时含跨设备聚合)
pub async fn cli_similar(
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

/// Ask the daemon host to exit; a GUI host ignores it.
/// Callers distinguish the host by pinging again after shutdown.
/// 请求 daemon 宿主退出;GUI 宿主忽略。调用方靠 shutdown 后的二次 ping 判别宿主
pub async fn cli_shutdown() -> CommonResponse<()> {
    if IS_DAEMON.load(Ordering::Relaxed) {
        tokio::spawn(async {
            // Graceful shutdown instead of a fixed sleep: stop sends the signal, then we wait until the serve task actually finished (it clears
            // SHUTDOWN_SENDER once all in-flight responses are written).
            // This both flushes our own response and avoids killing other CLIs' in-flight queries.
            // Bounded at 10s for safety.
            stop_http_server().await;
            wait_server_stopped().await;
            log::info!("CLI daemon exiting on /cli/shutdown");
            std::process::exit(0);
        });
    }
    CommonResponse::success(())
}
