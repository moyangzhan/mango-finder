use crate::document_loaders::cleanup_extracted_images;
use crate::entities::{FileInfo, IndexingTask};
use crate::enums::{CommandResultCode, DownloadEvent};
use crate::fs_watcher::watcher;
use crate::global::{
    CONTENT_STORAGE_CHANGING, FS_WATCHER_SETTING, INDEXING, INDEXING_FROM_WATCHER, SCANNING,
    STOP_INDEX_SIGNAL, STORAGE_PATH,
};
use crate::indexer_service;
use crate::repositories::{
    file_content_embedding_repo, file_content_fts_repo, file_info_repo,
    file_metadata_embedding_repo, indexing_task_repo,
};
use crate::searcher;
use crate::structs::command_result::CommandResult;
use crate::utils::download_util;
use rust_i18n::t;
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::{command, ipc::Channel};

#[command]
pub async fn start_indexing(paths: Vec<String>, from: &str) -> Result<CommandResult, String> {
    if paths.is_empty() {
        let result = CommandResult::error(
            CommandResultCode::INDEXING,
            t!("message.indexing-paths-empty").to_string(),
        );
        return Ok(result);
    }
    if SCANNING.load(Ordering::SeqCst) {
        log::info!("Scan process already started.");
        let result = CommandResult::error(
            CommandResultCode::INDEXING,
            t!("message.scanning").to_string(),
        );
        return Ok(result);
    }
    if INDEXING.load(Ordering::SeqCst) {
        log::info!("Indexing process is already running");
        let result = CommandResult::error(
            CommandResultCode::INDEXING,
            t!("message.indexing-processing").to_string(),
        );
        return Ok(result);
    }
    if CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        log::info!("Cannot start indexing while migration is in progress");
        let result = CommandResult::error(
            CommandResultCode::INDEXING,
            t!("message.migration-processing").to_string(),
        );
        return Ok(result);
    }
    let result = indexer_service::start_indexing(paths, from).await?;
    if result {
        return Ok(CommandResult::default());
    }
    let result = CommandResult::error(CommandResultCode::ERROR, "Error".to_string());
    return Ok(result);
}

#[command]
pub async fn stop_indexing() {
    STOP_INDEX_SIGNAL.store(true, Ordering::SeqCst);
    // Also cancel any in-progress migration
    if CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        log::info!("Stop signal received during migration, cancelling migration");
    }
}

#[command]
pub async fn load_indexing_tasks(
    page: i64,
    page_size: i64,
    column_key: &str,
    sort_order: &str,
) -> Result<Vec<IndexingTask>, String> {
    let tasks = indexing_task_repo::list(page, page_size, column_key, sort_order)?;
    Ok(tasks)
}

#[command]
pub async fn count_indexing_tasks() -> Result<i64, String> {
    let count = indexing_task_repo::count()?;
    Ok(count)
}

#[command]
pub async fn load_files(page: i64, page_size: i64) -> Result<Vec<FileInfo>, String> {
    let files = file_info_repo::list(page, page_size)?;
    Ok(files)
}

#[command]
pub async fn count_files() -> Result<i64, String> {
    let count = file_info_repo::count()?;
    Ok(count)
}

#[command]
pub async fn load_file_detail(
    file_id: i64,
    device_id: Option<String>,
) -> Result<Option<FileInfo>, String> {
    // If device_id is provided, fetch from remote device
    if let Some(did) = device_id {
        if !did.is_empty() {
            // Get device info
            let device = crate::repositories::device_repo::get_by_device_id(&did)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Device not found: {}", did))?;

            // Fetch file content from remote device
            let content_data = crate::cluster::http_client::fetch_file_content(
                file_id,
                &device.ip_address,
                device.port,
            )
            .await
            .map_err(|e| format!("Failed to fetch file content from remote device: {}", e))?;

            // Build FileInfo from response
            let file_info = FileInfo {
                id: file_id,
                content: content_data.content,
                ..Default::default()
            };

            return Ok(Some(file_info));
        }
    }

    // Local query
    let mut file = file_info_repo::get_by_id(file_id)?;
    if let Some(ref mut f) = file {
        // In "file" storage mode, content_ref_path points to a .md file — resolve to actual content
        if let Some(ref relative_path) = f.content_ref_path {
            match STORAGE_PATH.get() {
                Some(storage) => {
                    let md_path = std::path::Path::new(storage).join(relative_path);
                    f.content = std::fs::read_to_string(&md_path).unwrap_or_else(|e| {
                        log::warn!(
                            "Failed to read parsed document {}: {}",
                            md_path.display(),
                            e
                        );
                        String::new()
                    });
                }
                None => {
                    log::warn!("Storage path not initialized, cannot read parsed document");
                    f.content = String::new();
                }
            }
        }
    }
    Ok(file)
}

#[command]
pub fn delete_indexing_task(task_id: i64) -> Result<(), String> {
    indexing_task_repo::delete_by_id(task_id)?;
    Ok(())
}

#[command]
pub async fn delete_index_item(file_id: i64) -> Result<(), String> {
    file_content_fts_repo::delete_by_file_id(file_id)?;
    file_content_embedding_repo::delete_by_file_id(file_id)?;
    file_metadata_embedding_repo::delete_by_file_id(file_id)?;
    if let Some(file) = file_info_repo::get_by_id(file_id)? {
        let file_path = file.path;
        searcher::path_search_engine::remove_from_index(file_path.as_str(), true).await;
        // Clean up associated .md file if content was stored on disk
        if let Some(ref relative_path) = file.content_ref_path {
            if let Some(storage) = STORAGE_PATH.get() {
                let md_path = std::path::Path::new(storage).join(relative_path);
                let _ = std::fs::remove_file(&md_path);
            }
        }
        cleanup_extracted_images(file_id);
    }
    file_info_repo::delete_by_id(file_id)?;
    Ok(())
}

#[command]
pub async fn delete_index_items(file_ids: Vec<i64>) -> Result<(), String> {
    for file_id in &file_ids {
        // Best-effort deletion: log failures but continue with remaining files
        if let Err(e) = file_content_fts_repo::delete_by_file_id(*file_id) {
            log::warn!("Failed to delete FTS for file {}: {}", file_id, e);
        }
        if let Err(e) = file_content_embedding_repo::delete_by_file_id(*file_id) {
            log::warn!(
                "Failed to delete content embedding for file {}: {}",
                file_id,
                e
            );
        }
        if let Err(e) = file_metadata_embedding_repo::delete_by_file_id(*file_id) {
            log::warn!(
                "Failed to delete metadata embedding for file {}: {}",
                file_id,
                e
            );
        }
        if let Ok(Some(file)) = file_info_repo::get_by_id(*file_id) {
            searcher::path_search_engine::remove_from_index(&file.path, true).await;
            // Clean up associated .md file if content was stored on disk
            if let Some(ref relative_path) = file.content_ref_path {
                if let Some(storage) = STORAGE_PATH.get() {
                    let md_path = std::path::Path::new(storage).join(relative_path);
                    let _ = std::fs::remove_file(&md_path);
                }
            }
            cleanup_extracted_images(*file_id);
        }
        if let Err(e) = file_info_repo::delete_by_id(*file_id) {
            log::warn!("Failed to delete file info for file {}: {}", file_id, e);
        }
    }
    Ok(())
}

#[command]
pub async fn clear_index() -> Result<(), String> {
    indexer_service::clear_index().await
}

#[command]
pub async fn load_chunks(ids: Vec<u32>, device_id: Option<String>) -> Result<Vec<String>, String> {
    // If device_id is provided, fetch from remote device
    if let Some(did) = device_id {
        if !did.is_empty() {
            // Get device info
            let device = crate::repositories::device_repo::get_by_device_id(&did)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Device not found: {}", did))?;

            // Fetch chunks from remote device
            let chunks =
                crate::cluster::http_client::fetch_chunks(&ids, &device.ip_address, device.port)
                    .await
                    .map_err(|e| format!("Failed to fetch chunks from remote device: {}", e))?;

            return Ok(chunks);
        }
    }

    // Local query
    let segments =
        file_content_embedding_repo::list_chunks_by_ids(&ids).map_err(|e| e.to_string())?;
    Ok(segments)
}

#[command]
pub async fn download_multilingual_model(proxy: bool, on_event: Channel<DownloadEvent>) -> bool {
    log::info!("Downloading multilingual model");
    if let Err(e) = download_util::download_multilingual_model(proxy, &on_event).await {
        log::error!("Download multilingual model error: {}", e);
        return false;
    };
    return true;
}

#[command]
pub async fn check_path_type(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    if path.is_dir() {
        Ok("directory".to_string())
    } else if path.is_file() {
        Ok("file".to_string())
    } else {
        Err("Path does not exist".to_string())
    }
}

// ========== Watch path commands ==========

#[command]
pub async fn add_watch_path(path: &str) -> Result<(), String> {
    watcher::add_path(path).await.unwrap_or_else(|error| {
        log::error!("add watch path error:{}", error);
    });
    Ok(())
}

#[command]
pub async fn remove_watch_path(path: &str) -> Result<(), String> {
    watcher::remove_path(path).await.unwrap_or_else(|error| {
        log::error!("remove watch path error:{}", error);
    });
    Ok(())
}

#[command]
pub async fn indexing_watch_paths() -> Result<(), String> {
    if CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        log::info!("Skipping watch path indexing while content storage change is in progress");
        return Ok(());
    }
    let mut paths = vec![];
    for path in FS_WATCHER_SETTING.read().await.directories.iter() {
        paths.push(path.clone());
    }
    for path in FS_WATCHER_SETTING.read().await.files.iter() {
        paths.push(path.clone());
    }
    start_indexing(paths, INDEXING_FROM_WATCHER).await?;
    Ok(())
}
