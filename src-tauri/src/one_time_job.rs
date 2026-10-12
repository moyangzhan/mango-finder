//! One-time startup jobs; counterpart of `timers.rs` (periodic jobs).
//! 启动时执行一次的任务;与 timers.rs(周期任务)相对应。

use crate::utils::task_util;

/// Run after db init (the task lock lives in the DB): cheap jobs inline,
/// IO-heavy ones spawned off the startup path.
/// 需在 DB 初始化后调用(任务锁存于 DB):轻量任务同步,IO 重的异步不阻塞启动。
pub fn run_startup(standalone: bool) {
    // Orphan task-lock cleanup; standalone (CLI/daemon) skips: the age
    // heuristic cannot tell a live GUI long task from a crashed one
    // 孤儿任务锁清理;standalone(CLI/daemon)跳过:时长启发式无法区分
    // GUI 活任务与崩溃残留
    if !standalone {
        task_util::cleanup_orphan_task();
    }

    // Sweep crashed-process temp image dirs async; age-gated, so it cannot
    // race a live query
    // 异步清扫崩溃遗留的临时图片目录;按时间阈值,不会误删进行中查询的目录
    let _ = tokio::task::spawn_blocking(crate::document_loaders::cleanup_stale_temp_images_dirs);
}
