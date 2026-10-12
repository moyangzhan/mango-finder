use crate::repositories::{config_repo, indexing_task_repo};
use serde::{Deserialize, Serialize};

const CONFIG_NAME_ACTIVE_TASK: &str = "active_task";

/// Age fallback of the orphan rule; also bounds PID-reuse misjudgment (a
/// dead owner's pid reassigned to a live process looks alive for at most
/// this long). Same value as the pre-PID heuristic.
/// 孤儿判定的年龄兜底,同时限定 PID 复用误判的上界(死进程 pid 被复用后
/// 最多被误认存活此时长);与引入 PID 前的阈值一致。
const ORPHAN_TASK_THRESHOLD_SECS: i64 = 3600; // 1 hour

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActiveTask {
    pub task_type: String, // "indexing" | "content_storage_change" | "data_copying"
    pub category: Option<String>, // only for content_storage_change
    pub new_mode: Option<String>, // target storage mode (e.g. "file"), only for content_storage_change
    pub old_path: Option<String>, // only for data_copying
    pub started_at: i64,          // unix timestamp
    /// Owner process id; 0 = legacy lock JSON written before this field
    /// 持有进程 id;0 = 加该字段之前写入的旧锁
    #[serde(default)]
    pub pid: u32,
}

impl ActiveTask {
    /// Owner identity (pid + started_at) pre-filled
    /// 预填持有者信息(pid + started_at)
    pub fn new(task_type: &str) -> Self {
        Self {
            task_type: task_type.to_string(),
            category: None,
            new_mode: None,
            old_path: None,
            started_at: chrono::Utc::now().timestamp(),
            pid: std::process::id(),
        }
    }
}

pub fn lock_active_task(task: &ActiveTask) -> Result<(), String> {
    if let Some(existing) = get_active_task()? {
        if !is_orphan(&existing) {
            return Err(format!(
                "Task already in progress: {} (pid {}, {}s)",
                existing.task_type,
                existing.pid,
                task_age_secs(&existing)
            ));
        }
        clear_orphan(&existing);
    }
    let json = serde_json::to_string(task).map_err(|e| e.to_string())?;
    config_repo::upsert(CONFIG_NAME_ACTIVE_TASK, &json).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn unlock_active_task() -> Result<(), String> {
    config_repo::update_by_name(CONFIG_NAME_ACTIVE_TASK, "").map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_active_task() -> Result<Option<ActiveTask>, String> {
    let val = config_repo::get_val(CONFIG_NAME_ACTIVE_TASK);
    if val.is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<ActiveTask>(&val) {
        Ok(task) => Ok(Some(task)),
        Err(e) => {
            log::warn!("Corrupt active_task value, clearing: {}", e);
            let _ = unlock_active_task();
            Ok(None)
        }
    }
}

pub fn clear_active_task() -> Result<(), String> {
    unlock_active_task()
}

/// Orphan = owner process dead, or age over threshold; pid=0 (legacy) is
/// age-only.
/// 孤儿 = 持有进程已死或超龄;pid=0(旧锁)仅按年龄判定。
fn is_orphan(task: &ActiveTask) -> bool {
    !is_pid_alive(task.pid) || task_age_secs(task) > ORPHAN_TASK_THRESHOLD_SECS
}

fn task_age_secs(task: &ActiveTask) -> i64 {
    chrono::Utc::now().timestamp() - task.started_at
}

/// Clear an orphaned lock; rows still "running" belong to dead owners (a
/// live indexer holds the lock), so mark them cancelled.
/// 清除孤儿锁;仍处 running 的任务行必属死进程(活索引持有锁),标记 cancelled。
fn clear_orphan(task: &ActiveTask) {
    log::warn!(
        "Clearing orphaned task lock: type={}, pid={}, age={}s",
        task.task_type,
        task.pid,
        task_age_secs(task)
    );
    let _ = unlock_active_task();
    let _ = indexing_task_repo::interrupt_running();
}

/// pid=0 (unknown) counts as alive so the age rule decides
/// pid=0(未知)视为存活,交由年龄规则判定
fn is_pid_alive(pid: u32) -> bool {
    pid == 0 || pid_is_alive_impl(pid)
}

#[cfg(windows)]
fn pid_is_alive_impl(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Access denied still proves existence; other errors: gone
            // 拒绝访问仍能证明进程存在,其余错误视为已退出
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut exit_code: u32 = 0;
        let alive = GetExitCodeProcess(handle, &mut exit_code) != 0
            && exit_code == STILL_ACTIVE as u32;
        CloseHandle(handle);
        alive
    }
}

#[cfg(not(windows))]
fn pid_is_alive_impl(pid: u32) -> bool {
    // Signal 0 probes existence; EPERM means alive but owned by another user
    // 信号 0 仅探测存在性;EPERM 表示存活但属其他用户
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Check for and remove orphaned task locks left by a previous crash.
/// Should be called once at startup.
pub fn cleanup_orphan_task() {
    match get_active_task() {
        Ok(Some(task)) => {
            if is_orphan(&task) {
                clear_orphan(&task);
            } else {
                log::info!(
                    "Active task found (not orphan, age={}s): type={}",
                    task_age_secs(&task),
                    task.task_type
                );
            }
        }
        Ok(None) => {}
        Err(e) => {
            log::error!("Failed to check active_task on startup: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_with(pid: u32, age_secs: i64) -> ActiveTask {
        ActiveTask {
            started_at: chrono::Utc::now().timestamp() - age_secs,
            pid,
            ..ActiveTask::new("indexing")
        }
    }

    #[test]
    fn pid_zero_and_own_pid_count_as_alive() {
        assert!(is_pid_alive(0));
        assert!(is_pid_alive(std::process::id()));
    }

    #[test]
    fn dead_owner_pid_makes_orphan() {
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!is_pid_alive(pid));
        assert!(is_orphan(&task_with(pid, 0)));
    }

    #[test]
    fn orphan_rule_combines_pid_and_age() {
        assert!(!is_orphan(&task_with(std::process::id(), 60)));
        assert!(!is_orphan(&task_with(0, 60)));
        assert!(is_orphan(&task_with(
            std::process::id(),
            ORPHAN_TASK_THRESHOLD_SECS + 1
        )));
        assert!(is_orphan(&task_with(0, ORPHAN_TASK_THRESHOLD_SECS + 1)));
    }

    #[test]
    fn legacy_lock_json_without_pid_deserializes_to_zero() {
        let t: ActiveTask = serde_json::from_str(
            r#"{"task_type":"indexing","category":null,"new_mode":null,"old_path":null,"started_at":100}"#,
        )
        .unwrap();
        assert_eq!(t.pid, 0);
    }
}
