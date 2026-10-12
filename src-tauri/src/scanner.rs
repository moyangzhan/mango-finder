use crate::entities::{FileInfo, IndexingTask};
use crate::enums::{FileCategory, FileIndexStatus, IndexingEvent};
use crate::errors::{AppError, IndexingError};
use crate::global::{
    IGNORE_HIDDEN_DIRS, IGNORE_HIDDEN_FILES, INDEXER_SETTING, SCANNING, SCANNING_TOTAL,
    STOP_INDEX_SIGNAL,
};
use crate::indexer_service;
use crate::repositories::file_info_repo;
use crate::structs::indexer_setting::IndexerSetting;
use crate::utils::file_util::calculate_md5;
use crate::utils::{datetime_util, file_util, frontend_util};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;
use tokio::time::{Duration, sleep};

// Interval for logging scan progress (in loop iterations)
static PROGRESS_LOG_COUNTER: AtomicUsize = AtomicUsize::new(0);
const PROGRESS_LOG_INTERVAL: usize = 100;

const MAX_RETRIES: usize = 3;
static UNSCANNED_DIR_COUNT: AtomicUsize = AtomicUsize::new(0);

pub async fn start(paths: &Vec<String>, indexing_task: Arc<IndexingTask>, from: &str) {
    if paths.is_empty() {
        return;
    }
    if SCANNING.load(Ordering::SeqCst) {
        log::info!("Scan process already started.");
        return;
    }
    log::info!("Start scan process with {} paths", paths.len());
    SCANNING.store(true, Ordering::SeqCst);
    SCANNING_TOTAL.store(0, Ordering::SeqCst);
    UNSCANNED_DIR_COUNT.store(0, Ordering::SeqCst);
    PROGRESS_LOG_COUNTER.store(0, Ordering::SeqCst);
    let mut tasks = JoinSet::new();
    let (sender, mut rx) = mpsc::channel::<String>(5000);
    for path in paths {
        if path.is_empty() {
            continue;
        }
        let is_file = Path::new(path).is_file();
        if is_file {
            SCANNING_TOTAL.fetch_add(1, Ordering::SeqCst);
            let path_str = path.to_string();
            let task_id = indexing_task.id;
            let event_name = indexer_service::get_event_from(from);
            let task = tokio::spawn(async move {
                frontend_util::send_event(
                    event_name,
                    &IndexingEvent::Scan {
                        task_id: task_id,
                        msg: format!("Scanning path: {}", path_str),
                    },
                );
                let is_valid = is_valid_file_with(&PathBuf::from(&path_str)).await;
                if !is_valid {
                    log::debug!("File is not valid: {}", path_str);
                    return;
                }
                if let Err(op) = add_or_update_file_info(path_str.to_string()).await {
                    log::error!("add_or_update_file_info error: {}", op);
                }
            });
            tasks.spawn(task);
        } else {
            UNSCANNED_DIR_COUNT.fetch_add(1, Ordering::SeqCst);
            let _ = sender.send(path.to_string()).await.map_err(|op| {
                log::error!("queue send message error: {}", op);
                return;
            });
        }
    }
    loop {
        tokio::select! {
            maybe_dir = rx.recv() => {
                let Some(dir) = maybe_dir else { break; };
                let tx_clone = sender.clone();
                let task = indexing_task.clone();
                let from = from.to_string();
                let task = tokio::spawn(async move {
                    let _ = scan_and_store(dir, tx_clone, task, &from).await;
                });
                tasks.spawn(task);
            }

            _ = async {
                // Wait until stop signal or all directories are processed
                // Do not replace [ UNFINISH_DIR_COUNT.load(Ordering::SeqCst) ] with [ rx.len() ].
                while !STOP_INDEX_SIGNAL.load(Ordering::SeqCst)
                    && UNSCANNED_DIR_COUNT.load(Ordering::SeqCst) > 0
                {
                    sleep(Duration::from_millis(500)).await;
                    // Log progress periodically to avoid huge log files
                    let counter = PROGRESS_LOG_COUNTER.fetch_add(1, Ordering::SeqCst);
                    if counter % PROGRESS_LOG_INTERVAL == 0 {
                        log::debug!(
                            "Scan progress: unscanned dirs={}",
                            UNSCANNED_DIR_COUNT.load(Ordering::SeqCst)
                        );
                    }
                }
            } => {
                log::info!("Stop signal received or all directories processed.");
                break;
            }
        }
    }
    while let Some(task) = tasks.join_next().await {
        if let Err(e) = task {
            log::error!("Task failed: {}", e);
        }
    }
    log::info!(
        "Scan process was finished. Total files: {}",
        SCANNING_TOTAL.load(Ordering::SeqCst)
    );
    SCANNING.store(false, Ordering::SeqCst);
}

pub async fn scan_and_store(
    dir: String,
    sender: Sender<String>,
    task: Arc<IndexingTask>,
    from: &str,
) -> Result<(), IndexingError> {
    if dir.is_empty() {
        return Ok(());
    }
    log::debug!("Scan directory: {}", dir);
    let event_name = indexer_service::get_event_from(from);
    let indexer_setting = INDEXER_SETTING.read().await.clone();
    let mut entries = tokio::fs::read_dir(dir).await?;
    'outer: while let Some(entry) = entries.next_entry().await? {
        if STOP_INDEX_SIGNAL.load(Ordering::SeqCst) {
            log::info!("Scanning process was stopped.");
            frontend_util::send_event(
                &event_name,
                &IndexingEvent::Stop {
                    task_id: task.id,
                    msg: "Scanning interrupted by stop signal.".to_string(),
                },
            );
            SCANNING.store(false, Ordering::SeqCst);
            break;
        }
        let path_buf = entry.path();
        let path_str = path_buf.to_str().unwrap_or("");
        if path_str.is_empty() {
            continue;
        }
        frontend_util::send_event(
            &event_name,
            &IndexingEvent::Scan {
                task_id: task.id,
                msg: format!("Scanning path: {}", path_str),
            },
        );
        if path_buf.is_file() {
            SCANNING_TOTAL.fetch_add(1, Ordering::SeqCst);
            let is_valid = is_valid_file(&path_buf, &indexer_setting).await;
            if !is_valid {
                continue;
            }
            if let Err(op) = add_or_update_file_info(path_str.to_string()).await {
                log::error!("add_or_update_file_info error: {}", op);
                continue;
            }
        } else if path_buf.is_dir() {
            let dir_name: &str = path_buf.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if indexer_setting.ignore_dirs.contains(&dir_name.to_string()) {
                log::debug!("Ignore directory (in ignore list): {}", dir_name);
                continue;
            }
            // Check ignore_path_prefixes (full path prefixes)
            for ignore_path in &indexer_setting.ignore_path_prefixes {
                if path_str.starts_with(ignore_path) {
                    log::debug!("Ignore path prefix: {}", ignore_path);
                    continue 'outer;
                }
            }
            if IGNORE_HIDDEN_DIRS && dir_name.starts_with(".") {
                continue;
            }
            let path_owned = path_str.to_owned();

            // Try to send, handle full queue case
            let mut retries = 0;
            loop {
                match sender.try_send(path_owned.clone()) {
                    Ok(_) => {
                        UNSCANNED_DIR_COUNT.fetch_add(1, Ordering::SeqCst);
                        break;
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        if retries >= MAX_RETRIES {
                            log::warn!("Max retries exceeded for path: {}", path_owned);
                            break;
                        }
                        retries += 1;
                        // Queue is full, wait briefly and retry
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        log::error!("Error sending to channel: {}", e);
                        break;
                    }
                }
            }
        }
    }
    UNSCANNED_DIR_COUNT.fetch_sub(1, Ordering::SeqCst);
    Ok(())
}

pub async fn is_valid_file_with(path_buf: &PathBuf) -> bool {
    let indexer_setting = INDEXER_SETTING.read().await.clone();
    return is_valid_file(path_buf, &indexer_setting).await;
}

async fn is_valid_file(path_buf: &PathBuf, indexer_setting: &IndexerSetting) -> bool {
    let ext_str = path_buf
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext_str.is_empty() {
        // File has no extension - skip silently (common in development)
        return false;
    }
    let ext = ext_str.as_str();
    if indexer_setting.ignore_exts.contains(&ext.to_string()) {
        log::debug!("File extension is ignored: {}", path_buf.display());
        return false;
    }
    if !indexer_setting.ignore_files.is_empty()
        && indexer_setting
            .ignore_files
            .contains(&path_buf.to_str().unwrap_or("").to_string())
    {
        log::debug!("File is in ignore list: {}", path_buf.display());
        return false;
    }
    let file_name = path_buf.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if IGNORE_HIDDEN_FILES && file_name.starts_with(".") {
        // Skip hidden files silently
        return false;
    }
    true
}

pub async fn add_or_update_file_info(input_path: String) -> Result<(), IndexingError> {
    let path_str = input_path.as_str();
    let path = PathBuf::from(path_str);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut file_handle = tokio::fs::File::open(path.as_path()).await?;
    let md5_hash = calculate_md5(&mut file_handle)
        .await
        .map_err(|op| AppError::CalculateMd5Error(op.to_string()))?;

    // Check the file by path
    if let Some(mut file_record) = file_info_repo::get_by_path(path_str)? {
        let modified_time = datetime_util::systemtime_to_datetime(
            (&file_handle).metadata().await?.modified()?.into(),
        );
        if file_record.content_index_status == FileIndexStatus::Indexed.value()
            && file_record.file_update_time.ge(&modified_time)
        {
            log::debug!("File is already indexed: {}", path.display());
            return Ok(());
        }
        let mut meta = file_util::get_meta_by_record(path.as_path(), &file_record).await?;
        let file_category = FileCategory::from_ext(&ext);
        meta.extension = ext.clone();
        meta.category = file_category.to_text().to_string();
        let new_meta = meta.clone();
        file_record.name = new_meta.name.clone();
        file_record.md5 = md5_hash;
        file_record.is_invalid = false;
        file_record.invalid_reason = "".to_string();
        file_record.category = file_category.value();
        file_record.file_ext = ext;
        file_record.file_size = new_meta.size;
        file_record.file_create_time = new_meta.created;
        file_record.file_update_time = new_meta.modified;
        file_record.metadata = meta.clone();
        file_record.content_index_status = FileIndexStatus::Waiting.value();
        file_record.content_index_status_msg = "".to_string();
        file_record.meta_index_status = FileIndexStatus::Waiting.value();
        file_record.meta_index_status_msg = "".to_string();
        file_info_repo::update(&file_record)?;
    }
    // New file
    else {
        log::debug!("New file: {}, creating record for indexing", path.display());
        let mut meta = file_util::get_meta_by_local(path.as_path(), &file_handle).await?;
        let file_category = FileCategory::from_ext(&ext);
        meta.extension = ext.clone();
        meta.category = file_category.to_text().to_string();
        let new_meta = meta.clone();
        let mut new_file_record = FileInfo::default();
        new_file_record.name = new_meta.name.clone();
        new_file_record.category = file_category.value();
        new_file_record.path = path_str.to_string();
        new_file_record.md5 = md5_hash;
        new_file_record.path = path_str.to_string();
        new_file_record.file_ext = ext;
        new_file_record.file_size = new_meta.size;
        new_file_record.file_create_time = new_meta.created;
        new_file_record.file_update_time = new_meta.modified;
        new_file_record.metadata = meta.clone();
        match file_info_repo::insert(&new_file_record) {
            Ok(Some(new_file_record)) => {
                log::debug!("New file record created: {}", new_file_record.id);
            }
            Ok(None) => {
                log::warn!("Failed to create file record: {}", path.display());
            }
            Err(op) => {
                log::error!(
                    "Failed to create file record: {}, error: {}",
                    path.display(),
                    op
                );
            }
        }
    }
    Ok(())
}
