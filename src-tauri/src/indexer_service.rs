use crate::document_loaders::cleanup_extracted_images;
use crate::entities::IndexingTask;
use crate::enums::{ContentStorageChangeEvent, FileCategory, IndexingEvent};
use crate::errors::AppError;
use crate::global::{
    CONFIG_NAME_INDEXER_SETTING, CONTENT_STORAGE_CHANGING, FS_WATCHER_SETTING, IGNORE_HIDDEN_DIRS,
    INDEXER_SETTING, INDEXING, INDEXING_FROM_SELECTOR, SCANNING, SCANNING_TOTAL,
    STOP_INDEX_SIGNAL, STORAGE_PATH,
};
use crate::indexers;
use crate::initializer;
use crate::repositories::{
    config_repo, embedding_collection_repo, file_content_embedding_repo, file_content_fts_repo,
    file_info_repo,
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
    task_util::lock_storage_task(&task_util::ActiveTask {
        category: Some(category.to_string()),
        new_mode: Some(new_mode.to_string()),
        ..task_util::ActiveTask::new("content_storage_change")
    })
    .map_err(|e| e.to_string())?;

    // Validate mode value
    if new_mode != "database" && new_mode != "file" && new_mode != "none" {
        let _ = task_util::unlock_storage_task();
        return Err(format!("Invalid storage mode: {}", new_mode));
    }

    let category_enum = match category {
        "document" => FileCategory::Document,
        "image" => FileCategory::Image,
        "audio" => FileCategory::Audio,
        _ => {
            let _ = task_util::unlock_storage_task();
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
        let _ = task_util::unlock_storage_task();
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
        let _ = task_util::unlock_storage_task();

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

/// Whether the active embedding identity differs from the identity whose
/// collection currently holds a complete index (platform+model pair equality).
/// Generalizes the old local-model-name check to any backend.
///
/// 激活嵌入身份与「已完成索引」身份是否不同(平台+模型判等),
/// 将旧的本地模型名检测泛化到任意后端。
pub fn is_embedding_model_changed() -> Result<bool, String> {
    let active = embedding_collection_repo::active_identity();
    match embedding_collection_repo::indexed_identity() {
        Some(indexed) => Ok(!active.same_backend(&indexed)),
        None => Ok(true),
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
    task_util::lock_storage_task(&task_util::ActiveTask::new("indexing"))
        .map_err(|e| e.to_string())?;

    STOP_INDEX_SIGNAL.store(false, Ordering::SeqCst);
    // Record the active identity ("model · platform") on the task
    let identity = embedding_collection_repo::active_identity();
    let embedding_model = format!("{} · {}", identity.model, identity.platform);
    let task = match indexing_task_util::task_new(&paths.to_vec(), &embedding_model).await {
        Ok(t) => t,
        Err(e) => {
            let _ = task_util::unlock_storage_task();
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
            // A clean (not stopped) pipeline completion makes the active
            // collection the searchable index again (covers rebuild="none"
            // switches followed by a manual full index, and normal runs);
            // a user stop leaves the collection incomplete
            if !STOP_INDEX_SIGNAL.load(Ordering::SeqCst) {
                let active = embedding_collection_repo::active_identity();
                let _ = embedding_collection_repo::set_indexed_identity(Some(&active));
            }
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
        // Single transaction across file_info + FTS + every vector collection
        embedding_collection_repo::delete_file_with_indexes(file_info.id)?;
    }
    Ok(())
}

pub fn remove_directory_index(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    embedding_collection_repo::delete_files_with_prefix(path)?;
    Ok(())
}

async fn indexing_finish(task_id: i64, msg: &str, from: &str) -> Result<(), String> {
    SCANNING.store(false, Ordering::SeqCst);
    INDEXING.store(false, Ordering::SeqCst);
    SCANNING_TOTAL.store(0, Ordering::SeqCst);
    STOP_INDEX_SIGNAL.store(false, Ordering::SeqCst);
    let _ = task_util::unlock_storage_task();

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

/// 清空全部索引:单事务(全部向量库 + FTS + file_info)→ 路径缓存 → 磁盘 parsed_documents/extracted_images
/// Clear the whole index: one transaction (every collection + FTS + file_info) → path cache → on-disk leftovers
pub async fn clear_index() -> Result<(), String> {
    embedding_collection_repo::clear_all_collections()?;
    searcher::path_search_engine::clear().await;
    if let Some(storage) = STORAGE_PATH.get() {
        for dir in ["parsed_documents", "extracted_images"] {
            let dir_path = Path::new(storage).join(dir);
            if dir_path.exists() {
                let _ = std::fs::remove_dir_all(&dir_path);
            }
        }
    }
    Ok(())
}

/// Background metadata catch-up: embed metadata for files whose content index
/// is done but the metadata embedding is still pending (v6 upgrade leftovers,
/// switch-back restores, earlier failures). Runs only while no indexing /
/// storage-change job is active; a backend failure pauses the round and the
/// next cycle retries - transient outages self-heal, nothing blocks.
///
/// 后台 metadata 补嵌:为内容索引已完成但元数据嵌入仍待处理的文件补嵌
/// (v6 升级遗留、切回恢复、早前失败)。仅在无索引/存储变更任务运行时执行;
/// 后端失败暂停本轮、下轮重试 —— 小概率故障自愈,不阻断任何功能。
/// Cross-process lease so the GUI and an `mfd serve` daemon sharing the same
/// database never run the metadata catch-up concurrently (doubled API cost and
/// racing delete+insert pairs). Owner token = client_id + pid; a 10-minute TTL
/// makes a crashed holder self-release. The verify-after-write read resolves
/// the read-check-write race: only the process whose token is present after
/// both writes proceeds.
///
/// 跨进程租约:GUI 与共享同一数据库的 mfd serve daemon 不会并发跑补嵌
/// (双倍 API 开销 + delete/insert 竞争)。持有者令牌 = client_id + pid,
/// 10 分钟 TTL 让崩溃持有者自释放;写后回读消除读-判-写竞争,
/// 只有最终写入令牌的进程继续执行。
const METADATA_CATCHUP_LEASE: &str = "metadata_catchup_owner";
const METADATA_CATCHUP_LEASE_TTL_SECS: i64 = 600;

fn try_acquire_catchup_lease() -> bool {
    let now = chrono::Utc::now().timestamp();
    let token = format!(
        "{}-{}",
        *crate::read_lock!(crate::global::CLIENT_ID),
        std::process::id()
    );
    let current = config_repo::get_val(METADATA_CATCHUP_LEASE);
    if let Some((holder, expiry_str)) = current.split_once('|') {
        if let Ok(expiry) = expiry_str.parse::<i64>() {
            if expiry > now && holder != token {
                return false; // another live holder
            }
        }
    }
    let _ = config_repo::upsert(
        METADATA_CATCHUP_LEASE,
        &format!("{}|{}", token, now + METADATA_CATCHUP_LEASE_TTL_SECS),
    );
    // Verify after write: the loser's token gets overwritten and it stops here
    config_repo::get_val(METADATA_CATCHUP_LEASE).starts_with(&token)
}

pub async fn run_metadata_catchup() {
    loop {
        if crate::global::EXIT_APP_SIGNAL.load(Ordering::SeqCst) {
            break;
        }
        if SCANNING.load(Ordering::SeqCst)
            || INDEXING.load(Ordering::SeqCst)
            || CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst)
        {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            continue;
        }
        if !try_acquire_catchup_lease() {
            // Another process (GUI or daemon) holds the catch-up lease
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            continue;
        }
        let mut min_id = 0i64;
        let mut processed_any = false;
        let mut backend_failed = false;
        'round: loop {
            let files = match file_info_repo::list_meta_pending(min_id, 64) {
                Ok(files) => files,
                Err(e) => {
                    log::warn!("metadata catch-up list failed: {}", e);
                    break;
                }
            };
            if files.is_empty() {
                break;
            }
            for file in &files {
                if crate::global::EXIT_APP_SIGNAL.load(Ordering::SeqCst) {
                    return;
                }
                if SCANNING.load(Ordering::SeqCst)
                    || INDEXING.load(Ordering::SeqCst)
                    || CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst)
                {
                    break 'round;
                }
                min_id = file.id;
                let meta_text = file.metadata.to_text();
                let embed_result =
                    match crate::embedding_service_manager::embed(meta_text.as_str()).await {
                        Ok(result) => result,
                        Err(e) => {
                            // Backend unavailable: pause this round, retry next cycle
                            log::warn!("metadata catch-up embed failed: {}", e);
                            backend_failed = true;
                            break 'round;
                        }
                    };
                processed_any = true;
                if embed_result.dense.is_empty() {
                    let _ = file_info_repo::update_meta_index_status(
                        file.id,
                        crate::enums::FileIndexStatus::IndexFailed.value(),
                        "empty embedding response",
                    );
                    continue;
                }
                // Idempotent: drop a previous partial row for this file first
                let _ = crate::repositories::file_metadata_embedding_repo::delete_by_file_id(file.id);
                let inserted = crate::repositories::file_metadata_embedding_repo::insert(
                    &(crate::entities::FileMetaEmbedding {
                        id: 0,
                        file_id: file.id,
                        embedding: embed_result.dense,
                        sparse_vec: embed_result.sparse,
                        distance: -0.1,
                        sparse_score: 0.0,
                        score: 0,
                    }),
                );
                match inserted {
                    Ok(_) => {
                        let _ = file_info_repo::update_meta_index_status(
                            file.id,
                            crate::enums::FileIndexStatus::Indexed.value(),
                            "success",
                        );
                    }
                    Err(e) => {
                        log::warn!("metadata catch-up insert failed for {}: {}", file.id, e);
                        let _ = file_info_repo::update_meta_index_status(
                            file.id,
                            crate::enums::FileIndexStatus::IndexFailed.value(),
                            e.to_string().as_str(),
                        );
                    }
                }
            }
        }
        if backend_failed || !processed_any {
            // Nothing to do (or transient failure): idle wait before re-checking
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        } else {
            // Finished a batch; brief pause before looking for more
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}



// ---------------------------------------------------------------------------
// Embedding backend switching & rebuilds
// 嵌入后端切换与重建
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct SwitchOutcome {
    /// The probed identity actually in effect after the switch
    pub identity: crate::structs::embedding_identity::EmbeddingIdentity,
    /// Target collection already existed (instant switch, no rebuild needed)
    pub existing_collection: bool,
    /// A background rebuild was started
    pub rebuild_started: bool,
}

/// Watched paths (all indexed locations), used as the rebuild scope.
async fn all_watch_paths() -> Vec<String> {
    let watcher_setting = FS_WATCHER_SETTING.read().await;
    let mut paths = watcher_setting.directories.clone();
    paths.extend(watcher_setting.files.clone());
    paths
}

/// Persist the embedding category mode. Called only AFTER the identity flip
/// succeeded so a failure never leaves mode≠identity (the dispatcher would
/// then refuse every embed).
async fn persist_embedding_mode(mode: crate::enums::FileParserMode) -> Result<(), String> {
    let mut setting = INDEXER_SETTING.write().await;
    setting.embedding_parser_mode = mode;
    let json = serde_json::to_string(&*setting).map_err(|e| e.to_string())?;
    config_repo::update_by_name(CONFIG_NAME_INDEXER_SETTING, &json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Drop every collection except the given identity (explicit user choice).
/// Never called while a rebuild may still need a source collection.
fn clear_other_collections(
    identity: &crate::structs::embedding_identity::EmbeddingIdentity,
) -> Result<(), String> {
    let collections = embedding_collection_repo::list_collections().map_err(|e| e.to_string())?;
    for collection in collections {
        if collection.platform == identity.platform && collection.model == identity.model {
            continue;
        }
        let dropping = crate::structs::embedding_identity::EmbeddingIdentity {
            platform: collection.platform.clone(),
            model: collection.model.clone(),
            dim: collection.dim,
            has_sparse: collection.has_sparse,
        };
        embedding_collection_repo::drop_collection(&dropping).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// State flip of one switch, isolated so the caller can unlock the storage
/// lock on EVERY error path (a leaked persistent lock blocks all storage tasks
/// until orphan cleanup, an hour later).
/// 前置:存储任务锁已由调用方持有;本函数不触碰锁,错误统一由调用方解锁。
async fn perform_switch_flip(
    mode: crate::enums::FileParserMode,
    identity: &crate::structs::embedding_identity::EmbeddingIdentity,
    previous_identity: &crate::structs::embedding_identity::EmbeddingIdentity,
    same_target: bool,
) -> Result<bool, String> {
    // existing = registered AND holds data
    let existing_collection = embedding_collection_repo::get_collection(identity)
        .map_err(|e| e.to_string())?
        .map(|info| info.content_rows > 0 || info.meta_rows > 0)
        .unwrap_or(false);

    if same_target {
        // Identity unchanged; only the mode may differ. A built collection
        // needs nothing else; an unbuilt one (failed previous rebuild) falls
        // through to the rebuild dispatch in the caller.
        persist_embedding_mode(mode).await?;
        return Ok(existing_collection);
    }

    let tables = embedding_collection_repo::ensure_collection(identity).map_err(|e| e.to_string())?;

    if existing_collection {
        // Instant switch: recompute statuses from the target collection's rows
        embedding_collection_repo::restore_status_for(&tables).map_err(|e| e.to_string())?;
        embedding_collection_repo::set_active_identity(identity).map_err(|e| e.to_string())?;
        embedding_collection_repo::set_indexed_identity(Some(identity)).map_err(|e| e.to_string())?;
        if let Err(e) = persist_embedding_mode(mode).await {
            // Roll back to the previous identity so mode/identity stay in sync
            let prev_tables = embedding_collection_repo::collection_tables(previous_identity);
            let _ = embedding_collection_repo::restore_status_for(&prev_tables);
            let _ = embedding_collection_repo::set_active_identity(previous_identity);
            return Err(e);
        }
        return Ok(true);
    }

    // Fresh collection: reset statuses, semantic search stays refused
    // (indexed_identity cleared) until the rebuild finishes
    embedding_collection_repo::set_active_identity(identity).map_err(|e| e.to_string())?;
    embedding_collection_repo::set_indexed_identity(None).map_err(|e| e.to_string())?;
    embedding_collection_repo::reset_all_status_waiting().map_err(|e| e.to_string())?;
    if let Err(e) = persist_embedding_mode(mode).await {
        // Roll back: previous identity + statuses recomputed for it
        let _ = embedding_collection_repo::set_indexed_identity(Some(previous_identity));
        let _ = embedding_collection_repo::set_active_identity(previous_identity);
        let prev_tables = embedding_collection_repo::collection_tables(previous_identity);
        let _ = embedding_collection_repo::restore_status_for(&prev_tables);
        return Err(e);
    }
    Ok(false)
}

/// Switch the embedding backend (identity + category mode) atomically.
///
/// - existing collection → instant switch: statuses recomputed from the target
///   collection's rows, semantic search available immediately, optional rebuild;
/// - new collection (or the active-but-unbuilt one after a failed rebuild) →
///   statuses reset to Waiting and a background rebuild is started
///   ("fast" = re-embed stored chunk_text from the old collection, "full" =
///   standard pipeline over the watch paths, "none" = leave Waiting for the
///   next indexing run).
///
/// The storage-task lock is taken up front: the watcher indexing job and any
/// concurrent switch fail fast instead of interleaving with the flip, and the
/// lock is released on every path (handed to the spawned rebuild job for
/// fast/full - the job releases it in indexing_finish). With rebuild
/// fast/full, "clear others" is deferred until the rebuild SUCCEEDS so the old
/// collection stays as the switch-back fallback when a rebuild fails or is
/// stopped.
///
/// 原子切换嵌入后端(身份 + 类目模式)。已有向量库 → 即时切换;未建库
/// (含上次重建失败后的激活身份) → status 全量置 Waiting 并转后台重建
/// (fast=抄旧库仅重嵌入,full=标准管线,none=留给下次索引)。
/// 存储任务锁前置:watcher 索引与并发切换快速失败;锁在所有路径释放
/// (fast/full 移交给后台任务,由 indexing_finish 释放)。fast/full 时
/// 「清除其他库」推迟到重建成功后执行 —— 重建失败/中止,旧库保留作切回兜底。
pub async fn switch_embedding_backend(
    mode: crate::enums::FileParserMode,
    platform: String,
    model: String,
    rebuild: String,
    clear_others: bool,
) -> Result<SwitchOutcome, String> {
    if SCANNING.load(Ordering::SeqCst) || INDEXING.load(Ordering::SeqCst) {
        return Err(t!("message.indexing-processing").to_string());
    }
    // Single barrier up front: everything below runs under the storage lock
    task_util::lock_storage_task(&task_util::ActiveTask::new("indexing"))
        .map_err(|e| e.to_string())?;

    // 1. Probe the candidate identity (dim discovered from the backend itself)
    let identity = match crate::embedding_service_manager::probe_identity(&mode, &platform, &model)
        .await
    {
        Ok(identity) => identity,
        Err(e) => {
            let _ = task_util::unlock_storage_task();
            return Err(e.to_string());
        }
    };

    let previous_identity = embedding_collection_repo::active_identity();
    let same_target = previous_identity.same_backend(&identity);

    // 2. Flip identity/collection state (mode persisted only after the flip;
    //    every error unlocks here - no leaked persistent lock)
    let existing_collection = match perform_switch_flip(mode, &identity, &previous_identity, same_target).await {
        Ok(existing) => existing,
        Err(e) => {
            let _ = task_util::unlock_storage_task();
            return Err(e);
        }
    };

    // 3. Rebuild dispatch. Same target with a built collection = nothing left
    //    to do. Otherwise the requested rebuild ("fast"/"full") runs in the
    //    background: a fresh collection, an instant-switch-with-optional-rebuild,
    //    or the active-but-unbuilt retry. The lock is handed over to the job
    //    (released in its indexing_finish) and flags go up here so the watcher
    //    timer cannot squeeze a job in between.
    let start_rebuild = matches!(rebuild.as_str(), "fast" | "full")
        && !matches!((same_target, existing_collection), (true, true));

    if start_rebuild {
        if same_target {
            // Active identity, unbuilt collection (failed rebuild retry):
            // queue everything back up and refuse semantic search until done
            embedding_collection_repo::reset_all_status_waiting().map_err(|e| {
                let _ = task_util::unlock_storage_task();
                e.to_string()
            })?;
            embedding_collection_repo::set_indexed_identity(None).map_err(|e| {
                let _ = task_util::unlock_storage_task();
                e.to_string()
            })?;
        }
        INDEXING.store(true, Ordering::SeqCst);
        STOP_INDEX_SIGNAL.store(false, Ordering::SeqCst);
        let source_identity = previous_identity.clone();
        let target_identity = identity.clone();
        let use_full = rebuild == "full";
        let paths = if use_full { all_watch_paths().await } else { Vec::new() };
        if use_full && paths.is_empty() {
            // Nothing to rebuild: finish inline, statuses stay Waiting
            INDEXING.store(false, Ordering::SeqCst);
            if clear_others {
                if let Err(e) = clear_other_collections(&identity) {
                    let _ = task_util::unlock_storage_task();
                    return Err(e);
                }
            }
            let _ = task_util::unlock_storage_task();
            return Ok(SwitchOutcome {
                identity,
                existing_collection,
                rebuild_started: false,
            });
        }
        let _ = tokio::spawn(async move {
            if use_full {
                run_full_rebuild_job(&paths, &target_identity, clear_others).await;
            } else {
                run_fast_reembed_job(&source_identity, &target_identity, clear_others).await;
            }
        });
        return Ok(SwitchOutcome {
            identity,
            existing_collection,
            rebuild_started: true,
        });
    }

    // No rebuild: instant switch, rebuild="none", or re-confirming a built
    // collection. Clearing others is the user's explicit choice and no rebuild
    // needs a source fallback.
    if clear_others {
        if let Err(e) = clear_other_collections(&identity) {
            let _ = task_util::unlock_storage_task();
            return Err(e);
        }
    }
    let _ = task_util::unlock_storage_task();
    Ok(SwitchOutcome {
        identity,
        existing_collection,
        rebuild_started: false,
    })
}

/// Common rebuild-job tail: flags-off + unlock + Finish event. `success`
/// decides whether the collection becomes searchable (indexed_identity) and
/// whether the deferred "clear others" runs.
async fn finish_rebuild_job(
    task: &Arc<IndexingTask>,
    target_identity: &crate::structs::embedding_identity::EmbeddingIdentity,
    clear_others: bool,
    success: bool,
) {
    if success {
        // The target collection is the searchable index from now on; keyword
        // fallback ends. Clearing "other" collections is only safe once the
        // rebuild no longer needs the old one as raw material / fallback.
        let _ = embedding_collection_repo::set_indexed_identity(Some(target_identity));
        if clear_others {
            if let Err(e) = clear_other_collections(target_identity) {
                log::warn!("Clear other collections failed: {}", e);
            }
        }
    }
    if let Err(e) = indexing_finish(
        task.id,
        if success { "done" } else { "stopped" },
        INDEXING_FROM_SELECTOR,
    )
    .await
    {
        log::error!("Failed to finish rebuild task: {}", e);
    }
}

/// Whether a completed pipeline counts as a success: a user stop leaves the
/// collection incomplete, so it must NOT unlock semantic search or drop
/// fallback collections.
fn pipeline_finished_cleanly() -> bool {
    !STOP_INDEX_SIGNAL.load(Ordering::SeqCst)
}

/// "full" rebuild: the standard pipeline over the watch paths (scanner + all
/// category indexers). Assumes the storage lock is held and INDEXING is set.
async fn run_full_rebuild_job(
    paths: &[String],
    target_identity: &crate::structs::embedding_identity::EmbeddingIdentity,
    clear_others: bool,
) {
    let task = match indexing_task_util::task_new(
        &paths.to_vec(),
        &format!(
            "rebuild · {} · {}",
            target_identity.model, target_identity.platform
        ),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("Rebuild task creation failed: {}", e);
            INDEXING.store(false, Ordering::SeqCst);
            let _ = task_util::unlock_storage_task();
            return;
        }
    };
    let task = Arc::new(task);
    frontend_util::send_event(
        get_event_from(INDEXING_FROM_SELECTOR),
        &IndexingEvent::Start {
            task_id: task.id,
            msg: "Start".to_string(),
        },
    );
    let result = run_indexing_pipeline(paths, &task, INDEXING_FROM_SELECTOR).await;
    let success = result.is_ok() && pipeline_finished_cleanly();
    if let Err(e) = result {
        log::error!("Full rebuild pipeline failed: {}", e);
    }
    indexing_task_util::summary_to_db().await;
    finish_rebuild_job(&task, target_identity, clear_others, success).await;
}

/// "fast" re-embed rebuild: copy chunk_text from the source collection into
/// the (already active) target collection, re-embedding every chunk with the
/// new backend. Parsing is skipped entirely - the stored text is the raw
/// material. The source collection and the FTS rows are left alone until the
/// whole file is re-embedded; FTS rows are then rewritten to point at the new
/// chunk ids. Per-file failures mark that file IndexFailed (retryable) without
/// failing the whole job - only a stop signal or a database-level error keeps
/// the collection unsearchable. Assumes the storage lock is held and INDEXING
/// is set.
///
/// 「仅重嵌入」快速通道:从源向量库抄 chunk_text 进已激活的目标库,全部用
/// 新后端重新嵌入,完全跳过解析。源库与 FTS 行在整文件重嵌完成前不动,
/// 随后 FTS 行改写指向新 chunk id。单文件失败只标记该文件(可重试),
/// 不阻断整个库 —— 仅停止信号或数据库级错误才保持语义检索拒绝。
async fn run_fast_reembed_job(
    source_identity: &crate::structs::embedding_identity::EmbeddingIdentity,
    target_identity: &crate::structs::embedding_identity::EmbeddingIdentity,
    clear_others: bool,
) {
    let task = match indexing_task_util::task_new(
        &Vec::new(),
        &format!(
            "re-embed · {} · {}",
            target_identity.model, target_identity.platform
        ),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("Re-embed task creation failed: {}", e);
            INDEXING.store(false, Ordering::SeqCst);
            let _ = task_util::unlock_storage_task();
            return;
        }
    };
    let task = Arc::new(task);
    frontend_util::send_event(
        get_event_from(INDEXING_FROM_SELECTOR),
        &IndexingEvent::Start {
            task_id: task.id,
            msg: "Re-embedding".to_string(),
        },
    );

    let source_tables = embedding_collection_repo::collection_tables(source_identity);
    let mut min_file_id = 0i64;
    let mut failed_files = 0usize;
    let mut aborted = false;
    'files: loop {
        if STOP_INDEX_SIGNAL.load(Ordering::SeqCst)
            || crate::global::EXIT_APP_SIGNAL.load(Ordering::SeqCst)
        {
            log::info!("Fast re-embed stopped by signal");
            aborted = true;
            break;
        }
        let file_ids =
            match embedding_collection_repo::list_file_ids_in_collection(&source_tables, min_file_id, 100)
            {
                Ok(ids) => ids,
                Err(e) => {
                    log::error!("Fast re-embed list files failed: {}", e);
                    aborted = true;
                    break;
                }
            };
        if file_ids.is_empty() {
            break;
        }
        min_file_id = *file_ids.last().unwrap_or(&min_file_id);
        for file_id in &file_ids {
            if STOP_INDEX_SIGNAL.load(Ordering::SeqCst) {
                aborted = true;
                break 'files;
            }
            let chunks = match file_content_embedding_repo::list_chunk_texts_by_file(
                &source_tables,
                *file_id,
            ) {
                Ok(chunks) => chunks,
                Err(e) => {
                    // Database-level read failure: the job cannot trust its
                    // iteration anymore
                    log::error!("Fast re-embed read chunks of {} failed: {}", file_id, e);
                    aborted = true;
                    break 'files;
                }
            };
            if !fast_reembed_one_file(*file_id, &chunks).await {
                failed_files += 1;
            }
        }
    }
    if failed_files > 0 {
        log::warn!("Fast re-embed finished with {} failed files (marked IndexFailed)", failed_files);
    }

    finish_rebuild_job(&task, target_identity, clear_others, !aborted).await;
}

/// Re-embed one file's stored chunks into the active (target) collection and
/// rewrite its FTS rows. Target-scoped deletes only - the source collection is
/// never touched here (it is the resume material and the switch-back fallback).
/// Returns false when the file ended up IndexFailed (retryable, non-fatal).
async fn fast_reembed_one_file(file_id: i64, chunks: &[(i64, String)]) -> bool {
    // Drop partial target rows from a previous interrupted attempt (idempotent
    // resume); FTS is rewritten below once the new chunks exist
    let _ = file_content_embedding_repo::delete_active_rows_by_file(file_id);
    let _ = crate::repositories::file_metadata_embedding_repo::delete_by_file_id(file_id);

    // Chunk id per position: a failed re-select (Ok(None)) skips only its own
    // FTS row instead of shifting the id/text alignment
    let mut new_chunk_ids: Vec<Option<i64>> = Vec::with_capacity(chunks.len());
    let mark_failed = |file_id: i64, msg: String| {
        let _ = file_info_repo::update_content_index_status(
            file_id,
            crate::enums::FileIndexStatus::IndexFailed.value(),
            msg.as_str(),
        );
    };
    const BATCH_SIZE: usize = 16;
    for batch in chunks.chunks(BATCH_SIZE) {
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        let embeds = match crate::embedding_service_manager::embed_batch(&texts).await {
            Ok(embeds) => embeds,
            Err(e) => {
                log::warn!("Fast re-embed embed failed (file {}): {}", file_id, e);
                mark_failed(file_id, e.to_string());
                return false;
            }
        };
        for ((chunk_index, chunk_text), embed) in batch.iter().zip(embeds.into_iter()) {
            match file_content_embedding_repo::insert(&crate::entities::FileContentEmbedding {
                id: 0,
                file_id,
                embedding: embed.dense,
                chunk_index: *chunk_index,
                chunk_text: chunk_text.clone(),
                sparse_vec: embed.sparse,
                distance: -0.1,
                sparse_score: 0.0,
                score: 0,
            }) {
                Ok(row) => new_chunk_ids.push(row.map(|r| r.id)),
                Err(e) => {
                    log::warn!("Fast re-embed insert failed (file {}): {}", file_id, e);
                    mark_failed(file_id, e.to_string());
                    return false;
                }
            }
        }
    }
    // Rewrite FTS rows for this file to point at the new chunk ids (keyword
    // search shares the active collection's chunk ids)
    let _ = file_content_fts_repo::delete_by_file_id(file_id);
    for (chunk_id, (_, chunk_text)) in new_chunk_ids.iter().zip(chunks.iter()) {
        if let Some(chunk_id) = chunk_id {
            let _ = file_content_fts_repo::insert(file_id, *chunk_id, chunk_text).await;
        }
    }

    // Metadata re-embed from the stored metadata JSON. A metadata failure does
    // NOT fail the content index: chunks are valid, and the metadata catch-up
    // job re-tries meta rows independently.
    let meta_ok = match file_info_repo::get_by_id(file_id) {
        Ok(Some(file)) => {
            match crate::embedding_service_manager::embed(file.metadata.to_text().as_str()).await {
                Ok(embed) => {
                    let inserted = crate::repositories::file_metadata_embedding_repo::insert(
                        &(crate::entities::FileMetaEmbedding {
                            id: 0,
                            file_id,
                            embedding: embed.dense,
                            sparse_vec: embed.sparse,
                            distance: -0.1,
                            sparse_score: 0.0,
                            score: 0,
                        }),
                    );
                    if inserted.is_err() {
                        let _ = file_info_repo::update_meta_index_status(
                            file_id,
                            crate::enums::FileIndexStatus::IndexFailed.value(),
                            "re-embed metadata insert failed",
                        );
                        false
                    } else {
                        let _ = file_info_repo::update_meta_index_status(
                            file_id,
                            crate::enums::FileIndexStatus::Indexed.value(),
                            "success",
                        );
                        true
                    }
                }
                Err(e) => {
                    let _ = file_info_repo::update_meta_index_status(
                        file_id,
                        crate::enums::FileIndexStatus::IndexFailed.value(),
                        e.to_string().as_str(),
                    );
                    false
                }
            }
        }
        _ => false,
    };
    let _ = file_info_repo::update_content_index_status(
        file_id,
        crate::enums::FileIndexStatus::Indexed.value(),
        "success",
    );
    let _ = meta_ok;
    true
}

/// Category-level re-index: reset one category's files to Waiting and re-run
/// the standard pipeline for that category only (parser model changed →
/// optional re-transcription/re-caption; same identity, no rebuild prompt).
///
/// 按类目重索引:该类目文件置 Waiting,仅重跑该类目的标准管线
/// (解析模型变更后的可选重转写/重描述;身份不变,不弹重建确认)。
pub async fn start_category_reindex(category: &str) -> Result<bool, String> {
    let category = match category {
        "document" => FileCategory::Document,
        "image" => FileCategory::Image,
        "audio" => FileCategory::Audio,
        _ => return Err(format!("Unknown category: {}", category)),
    };
    if SCANNING.load(Ordering::SeqCst) || INDEXING.load(Ordering::SeqCst) {
        return Err(t!("message.indexing-processing").to_string());
    }
    // Lock BEFORE resetting statuses: a lock failure must not leave the
    // category Waiting with no job scheduled
    task_util::lock_storage_task(&task_util::ActiveTask::new("indexing"))
        .map_err(|e| e.to_string())?;
    let affected =
        match file_info_repo::reset_category_status_waiting(category.value()) {
            Ok(affected) => affected,
            Err(e) => {
                let _ = task_util::unlock_storage_task();
                return Err(e.to_string());
            }
        };
    if affected == 0 {
        let _ = task_util::unlock_storage_task();
        return Ok(false);
    }

    STOP_INDEX_SIGNAL.store(false, Ordering::SeqCst);
    INDEXING.store(true, Ordering::SeqCst);
    let identity = embedding_collection_repo::active_identity();
    let task = match indexing_task_util::task_new(
        &Vec::new(),
        &format!(
            "re-index · {} · {} · {}",
            category.to_text(),
            identity.model,
            identity.platform
        ),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            INDEXING.store(false, Ordering::SeqCst);
            let _ = task_util::unlock_storage_task();
            return Err(e.to_string());
        }
    };
    let task = Arc::new(task);
    let from = INDEXING_FROM_SELECTOR;
    frontend_util::send_event(
        get_event_from(from),
        &IndexingEvent::Start {
            task_id: task.id,
            msg: "Start".to_string(),
        },
    );

    match category {
        FileCategory::Document => {
            let mut indexer = indexers::document_indexer::DocumentIndexer::new();
            let _ = indexer.process(task.clone(), from).await;
        }
        FileCategory::Image => {
            if let Ok(mut indexer) = indexers::image_indexer::ImageIndexer::new().await {
                let _ = indexer.process(task.clone(), from).await;
            }
        }
        FileCategory::Audio => {
            if let Ok(mut indexer) = indexers::audio_indexer::AudioIndexer::new().await {
                let _ = indexer.process(task.clone(), from).await;
            }
        }
        _ => {}
    }
    indexing_task_util::summary_to_db().await;
    if let Err(e) = indexing_finish(task.id, "done", from).await {
        log::error!("Failed to finish category re-index task: {}", e);
    }
    Ok(true)
}
