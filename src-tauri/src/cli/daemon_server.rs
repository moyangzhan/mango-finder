//! `mfd serve`: the daemon process carrying the server, spawned by the CLI on demand (hidden subcommand)
//! Loopback-only bind, single instance, auto-exits when idle; the model loads lazily and is pre-warmed at startup when an index exists.
//! Prerequisite: main has finished init_paths + initializer(process).
//!
//! `mfd serve`:承载 server 的 daemon 进程,由 CLI 按需拉起(隐藏子命令)
//! 只绑回环、单实例、空闲自动退出;模型懒加载,有索引时启动即预热。
//! 前置条件:main 已完成 init_paths + initializer(process)。

use mango_finder_lib::cluster::http_server;
use mango_finder_lib::repositories::file_content_embedding_repo;
use mango_finder_lib::searcher::path_search_engine;
use mango_finder_lib::searcher::semantic_search_engine::warmup_embedding_service;

use crate::timer;

pub async fn run() -> ! {
    // The exclusive bind is the single-instance arbiter: concurrent daemons race to bind, and the losers exit here before paying any heavy init.
    http_server::mark_daemon();
    if let Err(e) = http_server::start_http_server_loopback().await {
        log::warn!(
            "port is held by an existing server (GUI-carried or a concurrent daemon), mfd serve exits: {}",
            e
        );
        std::process::exit(0);
    }

    // Heavy init after the bind: /ping already answers so the CLI's readiness budget no longer covers the cache build; keyword searches wait on the shared path-cache gate instead of racing a partial build.
    path_search_engine::init().await;

    // Align with the GUI's ui_mounted behavior: pre-warm when an index exists, so even the first CLI query is fast.
    if file_content_embedding_repo::count().unwrap_or(0) > 0 {
        if let Err(e) = warmup_embedding_service().await {
            log::error!("warmup embedding service failed: {}", e);
        }
    }

    // Periodic self-monitoring (idle exit) lives in timer.rs; never returns
    timer::run().await
}
