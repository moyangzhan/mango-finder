use crate::fs_watcher::watcher;
use crate::global::{ACTIVE_LOCALE, APP_DATA_PATH, CLIENT_ID, UI_MOUNTED};
use crate::repositories::file_content_embedding_repo;
use crate::searcher;
use crate::utils::{app_util, task_util};
use chrono::Utc;
use std::fs::read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, command};

#[command]
pub async fn ui_mounted(app: AppHandle) -> Result<(), String> {
    log::info!("UI mounted");
    UI_MOUNTED.store(true, Ordering::SeqCst);
    let locale = crate::read_lock!(ACTIVE_LOCALE).clone();
    rust_i18n::set_locale(locale.as_str());
    let _ = app_util::rebuild_tray_menu(&app);
    tokio::spawn(async move {
        crate::timers::start_after_ui_mounted(); // Timer start

        // Offline sync must complete before watcher starts to avoid duplicate processing
        if let Err(e) = crate::indexer_service::sync_offline_changes().await {
            log::error!("Offline sync failed: {}", e);
        }

        watcher::init_after_ui_mounted()
            .await
            .unwrap_or_else(|error| {
                log::error!("init file watch error:{}", error);
            }); // File watcher start
        if file_content_embedding_repo::count().unwrap_or(0) > 0 {
            searcher::semantic_search_engine::warmup_embedding_service()
                .await
                .unwrap_or_else(|error| {
                    log::error!("first warming up embedding service error: {}", error);
                });
        }
    });
    tokio::spawn(async {
        searcher::path_search_engine::init().await;
    });

    // Start cluster service if enabled
    let cluster_setting = crate::cluster::get_cluster_setting().await;
    if cluster_setting.enabled {
        tokio::spawn(async move {
            if let Err(e) = crate::cluster::start_cluster_service().await {
                log::error!("Failed to start cluster service: {}", e);
            }
        });
    }

    Ok(())
}

#[command]
pub async fn get_client_id() -> String {
    crate::read_lock!(CLIENT_ID).clone()
}

#[command]
pub async fn get_data_path() -> Result<String, String> {
    Ok(crate::read_lock!(APP_DATA_PATH).to_string())
}

#[command]
pub async fn set_data_path(path: String, force: bool, app: AppHandle) -> Result<String, String> {
    app_util::set_data_path(&path, force, &app).await
}

#[command]
pub async fn reset_data_path(force: bool, app: AppHandle) -> Result<String, String> {
    app_util::reset_data_path(force, &app).await
}

#[command]
pub async fn read_file_data(path: String) -> Result<Vec<u8>, String> {
    // Limit file size to 50 MB to prevent OOM
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > 50 * 1024 * 1024 {
        return Err("File too large to read".to_string());
    }
    read(path).map_err(|e| e.to_string())
}

#[command]
pub async fn open_directory(path: &str) -> Result<(), String> {
    let path = Path::new(&path);
    if !path.exists() {
        return Err("Path does not exist".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[command]
pub async fn get_active_task() -> Result<Option<task_util::ActiveTask>, String> {
    task_util::get_active_task()
}

#[command]
pub async fn clear_active_task() -> Result<(), String> {
    task_util::clear_active_task()
}

/// Retry data copy from old path after crash recovery
#[command]
pub async fn retry_data_copy(old_path: String, _app: AppHandle) -> Result<String, String> {
    let new_data_path = crate::read_lock!(APP_DATA_PATH).clone();

    // Lock active task
    task_util::lock_active_task(&task_util::ActiveTask {
        task_type: "data_copying".to_string(),
        category: None,
        new_mode: None,
        old_path: Some(old_path.clone()),
        started_at: Utc::now().timestamp(),
    })
    .map_err(|e| e.to_string())?;

    let old_path_buf = Path::new(&old_path);
    if !old_path_buf.exists() {
        let _ = task_util::unlock_active_task();
        return Err(format!("Old data path no longer exists: {}", old_path));
    }
    let old_storage = old_path_buf.join("storage");
    let new_storage = Path::new(&new_data_path).join("storage");

    let mut copy_errors = Vec::new();

    // Re-copy database
    if old_storage.exists() {
        let old_db = old_storage.join("mango-finder.db");
        if old_db.exists() {
            let new_db = new_storage.join("mango-finder.db");
            if let Err(e) = crate::utils::file_util::copy_file(&old_db, &new_db) {
                let _ = task_util::unlock_active_task();
                return Err(format!("Failed to copy db file: {}", e));
            }
        }
        for dir_name in &["parsed_documents", "extracted_images"] {
            let old_dir = old_storage.join(dir_name);
            if old_dir.exists() {
                let new_dir = new_storage.join(dir_name);
                if let Err(e) = crate::utils::file_util::copy_dir(&old_dir, &new_dir) {
                    log::warn!("Failed to copy {} directory: {}", dir_name, e);
                    copy_errors.push(format!("{}: {}", dir_name, e));
                }
            }
        }
    }

    let _ = task_util::unlock_active_task();
    if copy_errors.is_empty() {
        Ok("success".to_string())
    } else {
        Err(format!("Partial copy failure: {}", copy_errors.join(", ")))
    }
}

/// Revert data path to old path after crash recovery
#[command]
pub async fn revert_data_path(old_path: String, app: AppHandle) -> Result<(), String> {
    let sys_data_path = app
        .path()
        .data_dir()
        .unwrap_or_else(|error| {
            log::error!("Failed to get user data directory: {}", error);
            PathBuf::from("./")
        })
        .join(env!("CARGO_PKG_NAME"));
    let config_path = sys_data_path.join(".config");
    if let Err(e) = std::fs::write(&config_path, &old_path) {
        log::error!("Failed to revert data path: {}", e);
    }

    *crate::write_lock!(APP_DATA_PATH) = old_path;

    let _ = task_util::clear_active_task();
    Ok(())
}
