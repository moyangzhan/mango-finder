//! `mfd serve`: local service host spawned by the CLI on demand (hidden subcommand)
//! `mfd serve`:CLI 按需拉起的本地服务宿主(隐藏子命令)
//!
//! Loopback-only bind, single instance, auto-exits when idle; the model loads
//! lazily and is pre-warmed at startup when an index exists.
//! Prerequisite: main has finished init_paths + initializer(process).
//! 只绑回环、单实例、空闲自动退出;模型懒加载,有索引时启动即预热。
//! 前置条件:main 已完成 init_paths + initializer(process)。

use mango_finder_lib::cluster::http_server;
use mango_finder_lib::repositories::file_content_embedding_repo;
use mango_finder_lib::searcher::path_search_engine;
use mango_finder_lib::searcher::semantic_search_engine::warmup_embedding_service;
use std::time::Duration;

/// Idle exit threshold (default 30 minutes, mirroring the model idle expiry;
/// overridable via env var for testing)
/// 空闲退出上限(默认 30 分钟,镜像模型空闲过期时间;可用环境变量覆盖,便于测试)
fn idle_limit_secs() -> u64 {
    std::env::var("MANGO_FINDER_DAEMON_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30 * 60)
}

pub async fn run() -> ! {
    // The exclusive bind is the single-instance arbiter: concurrent daemons
    // race to bind, and the losers exit here before paying any heavy init.
    // 独占 bind 即单实例仲裁:并发 daemon 竞争绑定,输家在付出重初始化前退出
    http_server::mark_daemon();
    if let Err(e) = http_server::start_http_server_loopback().await {
        log::warn!(
            "port is held by an existing host (GUI or a concurrent daemon), mfd serve exits: {}",
            e
        );
        std::process::exit(0);
    }

    // Heavy init after the bind: /ping already answers so the CLI's readiness
    // budget no longer covers the cache build; keyword searches wait on the
    // shared path-cache gate instead of racing a partial build.
    // 绑定后再做重初始化:/ping 已可应答,就绪预算不再涵盖缓存构建;
    // 关键词查询阻塞在路径缓存共享闸门上,不会撞上半成品缓存
    path_search_engine::init().await;

    // Align with the GUI's ui_mounted behavior: pre-warm when an index exists,
    // so even the first CLI query is fast.
    // 与 GUI ui_mounted 行为对齐:有索引时启动即预热,首条 CLI 查询也快
    if file_content_embedding_repo::count().unwrap_or(0) > 0 {
        if let Err(e) = warmup_embedding_service().await {
            log::error!("warmup embedding service failed: {}", e);
        }
    }

    // Idle loop: exit once no /cli request arrives within the threshold,
    // leaving no resident process behind.
    // 空闲计时循环:无 /cli 请求达到上限即退出,不留常驻进程
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let idle = http_server::cli_idle_secs().unwrap_or(0);
        if idle >= idle_limit_secs() {
            log::info!("mfd serve idle for {}s, exiting", idle);
            http_server::stop_http_server().await;
            // Graceful shutdown (same as /cli/shutdown): wait until in-flight
            // responses are written before exiting.
            // 优雅停机(与 /cli/shutdown 相同):等在途响应写回后再退出
            http_server::wait_server_stopped().await;
            std::process::exit(0);
        }
    }
}
