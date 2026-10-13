//! One-time startup jobs; counterpart of `timers.rs` (periodic jobs).
//! 启动时执行一次的任务;与 timers.rs(周期任务)相对应。

use crate::utils::task_util;

/// Run after db init (the task lock lives in the DB): cheap jobs inline, IO-heavy ones spawned off the startup path.
/// 需在 DB 初始化后调用(任务锁存于 DB):轻量任务同步,IO 重的异步不阻塞启动。
pub fn run_startup() {
    // Orphan task-lock cleanup: PID liveness separates dead owners from live GUI/daemon tasks, so standalone runs it too
    task_util::cleanup_orphan_task();

    // Sweep crashed-process temp image dirs async; age-gated, so it cannot race a live query
    let _ = tokio::task::spawn_blocking(crate::document_loaders::cleanup_stale_temp_images_dirs);
}
