//! Daemon periodic self-monitoring tasks (idle exit)
//! daemon 周期自监控任务(空闲退出)

use mango_finder_lib::server::http_server;
use std::time::Duration;

/// Idle exit threshold (default 30 minutes, mirroring the model idle expiry; overridable via env var for testing)
/// 空闲退出上限(默认 30 分钟,镜像模型空闲过期时间;可用环境变量覆盖,便于测试)
fn idle_limit_secs() -> u64 {
    std::env::var("MANGO_FINDER_DAEMON_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30 * 60)
}

/// Self-monitoring loop; never returns — the daemon exits here once idle
/// 自监控循环;永不返回——daemon 空闲后从此处退出
pub async fn run() -> ! {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let idle = http_server::cli_idle_secs().unwrap_or(0);
        if idle >= idle_limit_secs() {
            log::info!("mfd serve idle for {}s, exiting", idle);
            http_server::stop_http_server().await;
            // Graceful shutdown (same as /cli/shutdown): wait until in-flight responses are written before exiting.
            http_server::wait_server_stopped().await;
            std::process::exit(0);
        }
    }
}
