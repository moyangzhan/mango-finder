use crate::document_loaders::cleanup_extracted_images;
use crate::embedding_service::EmbeddingService;
use crate::entities::IndexingTask;
use crate::enums::{ContentStorageChangeEvent, FileCategory, IndexingEvent};
use crate::errors::AppError;
use crate::global::{
    CONFIG_NAME_INDEXER_SETTING, CONTENT_STORAGE_CHANGING, FS_WATCHER_SETTING, IGNORE_HIDDEN_DIRS,
    INDEXER_SETTING, INDEXING, SCANNING, SCANNING_TOTAL, STOP_INDEX_SIGNAL, STORAGE_PATH,
};
use crate::indexers;
use crate::initializer;
use crate::repositories::{
    config_repo, file_content_embedding_repo, file_content_fts_repo, file_info_repo,
    file_metadata_embedding_repo, indexing_task_repo,
};
use crate::scanner;
use crate::searcher;
use crate::structs::indexer_setting::IndexerSetting;
use crate::traits::indexing_template::IndexingTemplate;
use crate::utils::{frontend_util, indexing_task_util, task_util};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub async fn update_indexer_setting(indexer_setting: IndexerSetting) -> Result<usize, String> {
    let json = serde_json::to_string(&indexer_setting).map_err(|e| AppError::SerializeError(e))?;
    let result = config_repo::update_by_name(CONFIG_NAME_INDEXER_SETTING, &json)?;
    initializer::init_setting(
        CONFIG_NAME_INDEXER_SETTING,
        || serde_json::to_string(&IndexerSetting::default()).unwrap_or_default(),
        &INDEXER_SETTING,
    )
    .await;
    Ok(result)
}

/// 启动内容存储迁移任务（后台执行）
///
/// 切换方向：
///   database → file    : 读取 DB 中的 content → 写入 parsed_documents/{id}_{name}.md → 更新 DB 为相对路径
///   database → none    : 清空 DB 中的 content（仅文档）
///   file → database    : 读取 .md 文件 → 写入 DB content → 删除 .md 文件
///   file → none        : 删除 .md 文件 → 清空 DB content（仅文档）
///   none → database    : 重新解析原始文件（仅文档）→ 保存到 DB
///   none → file        : 重新解析原始文件（仅文档）→ 写入 .md → DB 存相对路径
///
/// 注意：none → 其他方向仅支持文档类型，图片和音频不提供 none 选项
pub async fn start_content_storage_change(category: &str, new_mode: &str) -> Result<(), String> {
    if CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        return Err("Content storage change already in progress".to_string());
    }
    if INDEXING.load(Ordering::SeqCst) {
        return Err("Cannot change storage while indexing is in progress".to_string());
    }

    // DB persistent lock
    task_util::lock_active_task(&task_util::ActiveTask {
        category: Some(category.to_string()),
        new_mode: Some(new_mode.to_string()),
        ..task_util::ActiveTask::new("content_storage_change")
    })
    .map_err(|e| e.to_string())?;

    // Validate mode value
    if new_mode != "database" && new_mode != "file" && new_mode != "none" {
        let _ = task_util::unlock_active_task();
        return Err(format!("Invalid storage mode: {}", new_mode));
    }

    let category_enum = match category {
        "document" => FileCategory::Document,
        "image" => FileCategory::Image,
        "audio" => FileCategory::Audio,
        _ => {
            let _ = task_util::unlock_active_task();
            return Err(format!("Unknown category: {}", category));
        }
    };

    let old_mode = {
        let setting = INDEXER_SETTING.read().await;
        setting
            .content_storage
            .get_for_category(&category_enum)
            .to_string()
    };

    if old_mode == new_mode {
        let _ = task_util::unlock_active_task();
        return Ok(());
    }

    // Persist new setting to DB first so a crash won't leave data inconsistent.
    // If the change fails later, revert_and_persist will restore the old value.
    {
        let mut setting = INDEXER_SETTING.write().await;
        setting
            .content_storage
            .set_for_category(category, new_mode.to_string());
    }
    persist_indexer_setting().await;

    let category_str = category.to_string();
    let new_mode = new_mode.to_string();
    let old_mode = old_mode;

    tokio::spawn(async move {
        CONTENT_STORAGE_CHANGING.store(true, Ordering::SeqCst);

        let category_str_outer = category_str.clone();
        let old_mode_outer = old_mode.clone();
        let result = tokio::spawn(async move {
            let total = match file_info_repo::count_by_category(category_enum.value()) {
                Ok(t) => t,
                Err(e) => {
                    log::error!("Failed to count files for migration: {}", e);
                    return Err(format!("Failed to count files: {}", e));
                }
            };

            frontend_util::send_event(
                "content-storage-change-event",
                &ContentStorageChangeEvent::Start {
                    category: category_str.clone(),
                    total,
                },
            );

            let inner = change_content_storage_inner(
                &category_str,
                &category_enum,
                &old_mode,
                &new_mode,
                total,
            )
            .await;
            match inner {
                Ok((changed, failed)) => {
                    frontend_util::send_event(
                        "content-storage-change-event",
                        &ContentStorageChangeEvent::Complete {
                            category: category_str.clone(),
                            changed,
                            failed,
                        },
                    );
                    Ok(())
                }
                Err(e) => Err(e),
            }
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("Content storage migration panicked: {}", e);
            Err("Storage change task panicked".to_string())
        });

        CONTENT_STORAGE_CHANGING.store(false, Ordering::SeqCst);
        let _ = task_util::unlock_active_task();

        match result {
            Ok(()) => {
                persist_indexer_setting().await;
            }
            Err(e) => {
                frontend_util::send_event(
                    "content-storage-change-event",
                    &ContentStorageChangeEvent::Error {
                        category: category_str_outer.clone(),
                        message: e,
                    },
                );
                revert_and_persist(&category_str_outer, &old_mode_outer).await;
            }
        }
    });

    Ok(())
}

async fn revert_and_persist(category_str: &str, old_mode: &str) {
    let mut setting = INDEXER_SETTING.write().await;
    setting
        .content_storage
        .set_for_category(category_str, old_mode.to_string());
    drop(setting);
    persist_indexer_setting().await;
}

async fn persist_indexer_setting() {
    let setting = INDEXER_SETTING.read().await;
    if let Ok(json) = serde_json::to_string(&*setting) {
        if let Err(e) = config_repo::update_by_name(CONFIG_NAME_INDEXER_SETTING, &json) {
            log::error!("Failed to persist indexer setting after migration: {}", e);
        }
    }
}

async fn change_content_storage_inner(
    category_str: &str,
    category: &FileCategory,
    old_mode: &str,
    new_mode: &str,
    total: i64,
) -> Result<(i64, i64), String> {
    let storage_path = STORAGE_PATH
        .get()
        .cloned()
        .ok_or_else(|| "Storage path not initialized".to_string())?;
    if storage_path.is_empty() {
        return Err("Storage path is empty".to_string());
    }
    let mut changed = 0i64;
    let mut failed = 0i64;
    let batch_size = 500i64;
    let mut offset = 0i64;
    let mut last_progress = 0i64;

    while offset < total {
        let files = file_info_repo::list_by_category_paged(category.value(), batch_size, offset)
            .map_err(|e| e.to_string())?;

        if files.is_empty() {
            break;
        }

        for file in &files {
            if STOP_INDEX_SIGNAL.load(Ordering::SeqCst) {
                log::info!("Storage change cancelled by stop signal");
                frontend_util::send_event(
                    "content-storage-change-event",
                    &ContentStorageChangeEvent::Cancelled {
                        category: category_str.to_string(),
                        changed,
                        failed,
                    },
                );
                return Err("Storage change cancelled".to_string());
            }

            match change_content_storage_one(file, old_mode, new_mode, &storage_path, category, 0)
                .await
            {
                Ok(true) => changed += 1,
                Ok(false) => {} // no change needed
                Err(e) => {
                    log::warn!("Storage change failed for file {}: {}", file.path, e);
                    failed += 1;
                }
            }

            // Throttle progress events: emit at most every 50 files
            let processed = changed + failed;
            if processed - last_progress >= 50 || processed == total {
                last_progress = processed;
                frontend_util::send_event(
                    "content-storage-change-event",
                    &ContentStorageChangeEvent::Progress {
                        category: category_str.to_string(),
                        current: processed,
                        total,
                    },
                );
            }
        }

        offset += files.len() as i64;
    }

    log::info!(
        "Content storage change complete: category={}, {}→{}, changed={}, failed={}",
        category_str,
        old_mode,
        new_mode,
        changed,
        failed
    );
    Ok((changed, failed))
}

/// Change a single file's content storage mode. Returns Ok(true) if changed, Ok(false) if skipped.
/// `depth` limits recursive fallback calls to prevent infinite recursion.
fn change_content_storage_one<'a>(
    file: &'a crate::entities::FileInfo,
    old_mode: &'a str,
    new_mode: &'a str,
    storage_path: &'a str,
    category: &'a FileCategory,
    depth: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, String>> + Send + 'a>> {
    Box::pin(async move {
        let is_file_mode = file.content_ref_path.is_some();

        match (old_mode, new_mode) {
            // database → file: DB has content, write to .md
            ("database", "file") => {
                if is_file_mode || file.content.is_empty() {
                    return Ok(false); // already in file mode or no content
                }
                // Skip overly large content to avoid memory spike
                const MAX_MIGRATE_CONTENT_SIZE: usize = 50 * 1024 * 1024; // 50 MB
                if file.content.len() > MAX_MIGRATE_CONTENT_SIZE {
                    log::warn!(
                        "Skipping migration for file {}: content too large ({} bytes)",
                        file.id,
                        file.content.len()
                    );
                    return Ok(false);
                }
                let relative_path = crate::document_loaders::md_relative_path(file.id, &file.name);
                let md_path = std::path::Path::new(storage_path).join(&relative_path);
                if let Some(parent) = md_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&md_path, &file.content)
                    .map_err(|e| format!("write {}: {}", md_path.display(), e))?;
                file_info_repo::update_content_only(file.id, "", Some(&relative_path))
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            // database → none: clear content (document only)
            ("database", "none") => {
                if file.content.is_empty() || is_file_mode {
                    return Ok(false);
                }
                file_info_repo::update_content_only(file.id, "", None)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            // file → database: read .md → write to DB → delete .md
            ("file", "database") => {
                if !is_file_mode {
                    return Ok(false); // not in file mode
                }
                let relative_path = file
                    .content_ref_path
                    .as_deref()
                    .ok_or_else(|| format!("Missing content_ref_path for file {}", file.id))?;
                let md_path = std::path::Path::new(storage_path).join(relative_path);
                let content = std::fs::read_to_string(&md_path)
                    .map_err(|e| format!("read {}: {}", md_path.display(), e))?;
                // Write to DB first, then delete file to avoid data loss on crash
                file_info_repo::update_content_only(file.id, &content, None)
                    .map_err(|e| e.to_string())?;
                if let Err(e) = std::fs::remove_file(&md_path) {
                    log::warn!(
                        "Failed to delete .md file after storage change {}: {}",
                        relative_path,
                        e
                    );
                }
                Ok(true)
            }

            // file → none: delete .md + clear DB (document only)
            ("file", "none") => {
                if let Some(ref relative_path) = file.content_ref_path {
                    let md_path = std::path::Path::new(storage_path).join(relative_path);
                    let _ = std::fs::remove_file(&md_path);
                }
                file_info_repo::update_content_only(file.id, "", None)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            // none → database: re-parse file → save to DB
            ("none", "database") => {
                let content = reparse_file(file, category).await?;
                if content.is_empty() {
                    return Ok(false);
                }
                file_info_repo::update_content_only(file.id, &content, None)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            // none → file: re-parse file → write .md → save relative path
            ("none", "file") => {
                let content = reparse_file(file, category).await?;
                if content.is_empty() {
                    return Ok(false);
                }
                let relative_path = crate::document_loaders::md_relative_path(file.id, &file.name);
                let md_path = std::path::Path::new(storage_path).join(&relative_path);
                if let Some(parent) = md_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&md_path, &content)
                    .map_err(|e| format!("write {}: {}", md_path.display(), e))?;
                file_info_repo::update_content_only(file.id, "", Some(&relative_path))
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            // Fallback: detect actual state from content field and change accordingly
            _ => {
                if depth >= 1 {
                    return Err(format!(
                        "Storage change recursion limit reached for file {} (old={}, new={})",
                        file.id, old_mode, new_mode
                    ));
                }
                // Handle cross-direction cases (e.g., old was "none" but content exists from prior migration)
                let actual_old = if is_file_mode {
                    "file"
                } else if file.content.is_empty() {
                    "none"
                } else {
                    "database"
                };
                if actual_old == new_mode {
                    return Ok(false);
                }
                change_content_storage_one(
                    file,
                    actual_old,
                    new_mode,
                    storage_path,
                    category,
                    depth + 1,
                )
                .await
            }
        }
    })
}

/// Re-parse a file to recover its content (used for none → database/file migration).
/// Only supports documents. Images and audio would require expensive model inference.
async fn reparse_file(
    file: &crate::entities::FileInfo,
    category: &FileCategory,
) -> Result<String, String> {
    let path = std::path::Path::new(&file.path);
    if !path.exists() {
        return Err(format!("File no longer exists: {}", file.path));
    }

    match category {
        FileCategory::Document => {
            use crate::global::{EXT_TO_DOC_LOADER, MAX_DOCUMENT_LOAD_CHARS};
            let loader_map = EXT_TO_DOC_LOADER.read().await;
            let loader = loader_map.get(&file.file_ext).cloned();
            drop(loader_map);

            match loader {
                Some(doc_loader) => {
                    let file_path = file.path.clone();
                    let file_id = file.id;
                    let file_name = file.name.clone();
                    tokio::task::spawn_blocking(move || {
                        doc_loader.load_max_with_id(
                            std::path::Path::new(&file_path),
                            file_id,
                            &file_name,
                            MAX_DOCUMENT_LOAD_CHARS,
                        )
                    })
                    .await
                    .map_err(|e| format!("Re-parse task panicked: {}", e))?
                    .map_err(|e| format!("Re-parse failed: {}", e))
                }
                None => Ok(String::new()),
            }
        }
        _ => {
            // Image/audio re-parsing requires model inference, not supported in migration
            log::warn!(
                "Re-parsing not supported for {:?} files in migration",
                category
            );
            Ok(String::new())
        }
    }
}

pub async fn is_embedding_model_changed() -> Result<bool, String> {
    let tasks = indexing_task_repo::list(1, 1, "id", "desc");
    if tasks.is_err() {
        return Ok(false);
    }
    match tasks {
        Ok(tasks) => {
            if tasks.is_empty() {
                return Ok(false);
            }
            let latest = tasks
                .first()
                .map(|item| item.embedding_model.clone())
                .unwrap_or_default();
            let embedding_name: &'static str = EmbeddingService::model_name().await;
            if latest != embedding_name {
                return Ok(true);
            } else {
                return Ok(false);
            }
        }
        Err(_) => {
            return Ok(false);
        }
    }
}

/// Indexing workflow consists of four main phases:
///
/// 1. File scanning:
///    Scan specified paths and store file metadata in database
///
/// 2. Document processing:
///    Extract text content and generate document embeddings
///
/// 3. Image processing:
///    Extract visual features and generate image embeddings
///    Prerequisites: Private mode disabled, Model platform configured
///
/// 4. Audio processing:
///    Extract audio features and generate audio embeddings
///    Prerequisites: Private mode disabled, Model platform configured
pub async fn start_indexing(paths: Vec<String>, from: &str) -> Result<bool, String> {
    if paths.is_empty() {
        return Ok(false);
    }
    if SCANNING.load(Ordering::SeqCst) {
        return Ok(false);
    }
    if INDEXING.load(Ordering::SeqCst) {
        return Ok(false);
    }

    // DB persistent lock
    task_util::lock_active_task(&task_util::ActiveTask::new("indexing"))
        .map_err(|e| e.to_string())?;

    STOP_INDEX_SIGNAL.store(false, Ordering::SeqCst);
    let embedding_model = EmbeddingService::model_name().await;
    let task = match indexing_task_util::task_new(&paths.to_vec(), embedding_model).await {
        Ok(t) => t,
        Err(e) => {
            let _ = task_util::unlock_active_task();
            return Err(e.to_string());
        }
    };

    let task = Arc::new(task);

    frontend_util::send_event(
        get_event_from(from),
        &IndexingEvent::Start {
            task_id: task.id,
            msg: "Start".to_string(),
        },
    );

    let result = run_indexing_pipeline(&paths, &task, from).await;

    match &result {
        Ok(()) => {
            if let Err(e) = indexing_finish(task.id, "done", from).await {
                log::error!("Failed to finish indexing task: {}", e);
            }
        }
        Err(e) => {
            log::error!("Indexing pipeline failed: {}", e);
            if let Err(e2) = indexing_finish(task.id, &format!("error: {}", e), from).await {
                log::error!("Failed to finish indexing task after error: {}", e2);
            }
        }
    }

    result.map(|_| true)
}

async fn run_indexing_pipeline(
    paths: &[String],
    task: &Arc<IndexingTask>,
    from: &str,
) -> Result<(), String> {
    // Scan specified paths and store file metadata in database
    scanner::start(&paths.to_vec(), task.clone(), from).await;

    // Scanned files
    indexing_task_util::set_total(SCANNING_TOTAL.load(Ordering::SeqCst) as i64).await;
    indexing_task_util::summary_to_db().await;

    // Embedding processing
    INDEXING.store(true, Ordering::SeqCst);

    log::info!("Starting document indexing...");
    let mut document_indexer = indexers::document_indexer::DocumentIndexer::new();
    let _ = document_indexer
        .process(task.clone(), from)
        .await
        .unwrap_or_else(|e| log::error!("start_indexing => Document indexing error,{}", e));
    log::info!(
        "Document indexing done, status: {}",
        serde_json::json!(document_indexer.status)
    );
    indexing_task_util::summary_to_db().await;

    let unindex_images_cnt = file_info_repo::count_unindexed_files(FileCategory::Image.value())
        .map_err(|e| e.to_string())?;
    log::info!("Total images to index: {}", unindex_images_cnt);
    if unindex_images_cnt > 0 {
        if let Ok(mut image_indexer) = indexers::image_indexer::ImageIndexer::new().await {
            log::info!("Starting image indexing...");
            let _ = image_indexer
                .process(task.clone(), from)
                .await
                .unwrap_or_else(|e| log::error!("Image indexing error: {}", e));
            indexing_task_util::summary_to_db().await;
        }
    }

    let unindex_audio_cnt = file_info_repo::count_unindexed_files(FileCategory::Audio.value())
        .map_err(|e| e.to_string())?;
    log::info!("Total audio files to index: {}", unindex_audio_cnt);
    if unindex_audio_cnt > 0 {
        if let Ok(mut audio_indexer) = indexers::audio_indexer::AudioIndexer::new().await {
            log::info!("Starting audio indexing...");
            let _ = audio_indexer
                .process(task.clone(), from)
                .await
                .unwrap_or_else(|e| log::error!("Audio indexing error: {}", e));
            indexing_task_util::summary_to_db().await;
        }
    }

    Ok(())
}

/// Sync offline changes in watched directories.
/// Detects files deleted while the app was offline, then runs the full indexing pipeline.
pub async fn sync_offline_changes() -> Result<(), String> {
    let watcher_setting = FS_WATCHER_SETTING.read().await;
    let dir_paths: Vec<String> = watcher_setting.directories.clone();
    let file_paths: Vec<String> = watcher_setting.files.clone();
    drop(watcher_setting);

    if dir_paths.is_empty() && file_paths.is_empty() {
        return Ok(());
    }

    log::info!(
        "Starting offline sync for {} directories, {} files",
        dir_paths.len(),
        file_paths.len()
    );

    frontend_util::send_event("offline-sync-status", "started");

    // Collect existing file paths from watched directories
    let existing_files = collect_existing_file_paths(&dir_paths).await;

    // Detect deleted files: DB records whose paths no longer exist on disk
    let mut deleted_count = 0u32;
    for dir in &dir_paths {
        let db_paths = file_info_repo::list_paths_by_prefix_path(dir).unwrap_or_default();
        for db_path in db_paths {
            if !existing_files.contains(&db_path) {
                remove_file_index(&db_path)?;
                deleted_count += 1;
            }
        }
    }

    // Handle watched individual files
    for file_path in &file_paths {
        if !Path::new(file_path).exists() {
            remove_file_index(file_path)?;
            deleted_count += 1;
        }
    }

    if deleted_count > 0 {
        searcher::path_search_engine::notify_paths_removed();
    }
    log::info!(
        "Offline sync: removed {} deleted file records",
        deleted_count
    );

    // Scan + embed via existing pipeline
    let mut all_paths = dir_paths;
    all_paths.extend(file_paths);
    if let Err(e) = start_indexing(all_paths, "offline_sync").await {
        log::error!("Offline sync indexing failed: {}", e);
        frontend_util::send_event("offline-sync-status", "error");
        return Err(e);
    }

    frontend_util::send_event("offline-sync-status", "completed");

    Ok(())
}

async fn collect_existing_file_paths(dirs: &[String]) -> HashSet<String> {
    let mut existing = HashSet::new();
    for dir in dirs {
        collect_files_recursive(dir, &mut existing).await;
    }
    existing
}

async fn collect_files_recursive(dir: &str, result: &mut HashSet<String>) {
    let indexer_setting = INDEXER_SETTING.read().await.clone();
    let shared_result = Arc::new(std::sync::Mutex::new(std::mem::take(result)));
    collect_files_recursive_inner(
        dir.to_string(),
        shared_result.clone(),
        Arc::new(indexer_setting),
        0,
    )
    .await;
    let extracted = match Arc::try_unwrap(shared_result) {
        Ok(mutex) => mutex.into_inner().unwrap_or_else(|e| e.into_inner()),
        Err(arc) => {
            log::warn!(
                "collect_files_recursive: Arc still had multiple references, falling back to lock"
            );
            arc.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    };
    *result = extracted;
}

const MAX_DIR_DEPTH: u32 = 50;

fn collect_files_recursive_inner(
    dir: String,
    result: Arc<std::sync::Mutex<HashSet<String>>>,
    indexer_setting: Arc<IndexerSetting>,
    depth: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        if depth > MAX_DIR_DEPTH {
            log::warn!("Directory traversal depth limit reached: {}", dir);
            return;
        }
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to read directory {}: {}", dir, e);
                return;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path_buf = entry.path();
            let path_str = match path_buf.to_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            if path_buf.is_file() {
                if scanner::is_valid_file_with(&path_buf).await {
                    result
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(path_str);
                }
            } else if path_buf.is_dir() {
                let dir_name = path_buf.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if indexer_setting.ignore_dirs.contains(&dir_name.to_string()) {
                    continue;
                }
                if IGNORE_HIDDEN_DIRS && dir_name.starts_with('.') {
                    continue;
                }
                let mut skip = false;
                for ignore_path in &indexer_setting.ignore_path_prefixes {
                    if path_str.starts_with(ignore_path) {
                        skip = true;
                        break;
                    }
                }
                if skip {
                    continue;
                }
                // Skip symlinks to avoid infinite loops from circular links
                if path_buf.is_symlink() {
                    continue;
                }
                collect_files_recursive_inner(
                    path_str,
                    result.clone(),
                    indexer_setting.clone(),
                    depth + 1,
                )
                .await;
            }
        }
    })
}

pub async fn index_file(path: &str) -> Result<(), String> {
    // Skip if a long-running task is in progress
    if INDEXING.load(Ordering::SeqCst) || CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        return Ok(());
    }
    let path_buf = PathBuf::from(path);
    let is_valid = scanner::is_valid_file_with(&path_buf).await;
    if !is_valid {
        return Ok(());
    }
    if let Err(add_info_result) = scanner::add_or_update_file_info(path.to_string()).await {
        log::error!("add_or_update_file_info error: {:?}", add_info_result);
        return Ok(());
    }

    // Double-check before heavy embedding work
    if INDEXING.load(Ordering::SeqCst) || CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        return Ok(());
    }

    let ext = path_buf
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(file_info) = file_info_repo::get_by_path(path)? else {
        return Ok(());
    };
    let category = FileCategory::from_ext(&ext);

    let storage_mode = {
        INDEXER_SETTING
            .read()
            .await
            .content_storage
            .get_for_category(&category)
            .to_string()
    };

    match category {
        FileCategory::Document => {
            let document_indexer = indexers::document_indexer::DocumentIndexer::new();
            document_indexer
                .embedding_one_file(&file_info, &storage_mode)
                .await?;
        }
        FileCategory::Image => {
            let image_indexer = indexers::image_indexer::ImageIndexer::new().await?;
            image_indexer
                .embedding_one_file(&file_info, &storage_mode)
                .await?;
        }
        FileCategory::Audio => {
            let audio_indexer = indexers::audio_indexer::AudioIndexer::new().await?;
            audio_indexer
                .embedding_one_file(&file_info, &storage_mode)
                .await?;
        }
        _ => {}
    }
    Ok(())
}

pub fn remove_file_index(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    let file_info = file_info_repo::get_by_path(path)?;
    if let Some(file_info) = file_info {
        // Clean up associated .md file if content was stored on disk
        if let Some(ref relative_path) = file_info.content_ref_path {
            if let Some(storage) = STORAGE_PATH.get() {
                let md_path = std::path::Path::new(storage).join(relative_path);
                let _ = std::fs::remove_file(&md_path);
            }
        }
        cleanup_extracted_images(file_info.id);
        file_info_repo::delete_by_id(file_info.id)?;
        file_content_fts_repo::delete_by_file_id(file_info.id)?;
        file_content_embedding_repo::delete_by_file_id(file_info.id)?;
        file_metadata_embedding_repo::delete_by_file_id(file_info.id)?;
    }
    Ok(())
}

pub fn remove_directory_index(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    file_content_fts_repo::delete_by_prefix_path(path)?;
    file_content_embedding_repo::delete_by_file_prefix_path(path)?;
    file_metadata_embedding_repo::delete_by_file_prefix_path(path)?;
    file_info_repo::delete_by_prefix_path(path)?;
    Ok(())
}

async fn indexing_finish(task_id: i64, msg: &str, from: &str) -> Result<(), String> {
    SCANNING.store(false, Ordering::SeqCst);
    INDEXING.store(false, Ordering::SeqCst);
    SCANNING_TOTAL.store(0, Ordering::SeqCst);
    STOP_INDEX_SIGNAL.store(false, Ordering::SeqCst);
    let _ = task_util::unlock_active_task();

    // Notify frontend first so UI always unblocks
    frontend_util::send_event(
        get_event_from(from),
        &IndexingEvent::Finish {
            task_id,
            msg: msg.to_string(),
        },
    );

    let _ = indexing_task_util::task_done().await;
    Ok(())
}

pub fn get_event_from(from: &str) -> &'static str {
    if from.is_empty() {
        return crate::global::EVENT_SELECTOR_INDEXING;
    }
    if from == crate::global::INDEXING_FROM_SELECTOR {
        return crate::global::EVENT_SELECTOR_INDEXING;
    }
    return crate::global::EVENT_WATCHER_INDEXING;
}
