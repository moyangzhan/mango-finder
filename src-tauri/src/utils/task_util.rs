use crate::repositories::config_repo;
use serde::{Deserialize, Serialize};

const CONFIG_NAME_ACTIVE_TASK: &str = "active_task";

/// Tasks older than this threshold (seconds) are considered orphaned.
const ORPHAN_TASK_THRESHOLD_SECS: i64 = 3600; // 1 hour

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActiveTask {
    pub task_type: String, // "indexing" | "content_storage_change" | "data_copying"
    pub category: Option<String>, // only for content_storage_change
    pub new_mode: Option<String>, // target storage mode (e.g. "file"), only for content_storage_change
    pub old_path: Option<String>, // only for data_copying
    pub started_at: i64,          // unix timestamp
}

pub fn lock_active_task(task: &ActiveTask) -> Result<(), String> {
    if let Some(existing) = get_active_task()? {
        return Err(format!("Task already in progress: {}", existing.task_type));
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

/// Check for and remove orphaned task locks left by a previous crash.
/// Should be called once at startup.
pub fn cleanup_orphan_task() {
    let now = chrono::Utc::now().timestamp();
    match get_active_task() {
        Ok(Some(task)) => {
            if now - task.started_at > ORPHAN_TASK_THRESHOLD_SECS {
                log::warn!(
                    "Clearing orphaned task lock: type={}, started_at={}, age={}s",
                    task.task_type,
                    task.started_at,
                    now - task.started_at
                );
                let _ = unlock_active_task();
            } else {
                log::info!(
                    "Active task found (not orphan, age={}s): type={}",
                    now - task.started_at,
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
