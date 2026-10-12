use crate::enums::FsEvent;
use crate::fs_watcher::fs_event_normalizer::FsEventNormalizer;
use crate::global::{
    APP_DATA_PATH, CONFIG_NAME_WATCHER_SETTING, EXIT_APP_SIGNAL, FS_WATCHER_SETTING,
    INCREMENT_WATCH_PATHS, INDEXING_FROM_WATCHER,
};
use crate::indexer_service;
use crate::repositories::{config_repo, file_info_repo};
use crate::searcher::path_search_engine;
use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::{collections::HashMap, path::PathBuf, time::Duration};
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;

pub static WATCHER: OnceLock<AsyncRwLock<RecommendedWatcher>> = OnceLock::new();

pub async fn init_after_ui_mounted() -> Result<()> {
    log::info!("init file watcher");
    let (tx, rx) = mpsc::channel::<Event>(5000);
    let watcher = RecommendedWatcher::new(
        move |res: std::result::Result<Event, notify::Error>| match res {
            Ok(event) => {
                if let Err(error) = tx.try_send(event) {
                    log::error!("file watcher send msg to channel error:{}", error);
                }
            }
            Err(err) => log::error!("file watch error: {:?}", err),
        },
        Config::default(),
    )?;
    WATCHER.set(AsyncRwLock::new(watcher)).map_err(|_| {
        anyhow::anyhow!("Failed to initialize watcher - it may already be initialized")
    })?;
    for path in FS_WATCHER_SETTING.read().await.directories.iter() {
        watch_path(path).await?;
    }
    for path in FS_WATCHER_SETTING.read().await.files.iter() {
        watch_path(path).await?;
    }
    let mut fs_event_normalizer = FsEventNormalizer::new();
    // debounce duration, adjust as needed
    tokio::spawn(async move {
        fs_event_loop(rx, &mut fs_event_normalizer).await;
    });
    Ok(())
}

pub async fn add_path(path: &str) -> Result<()> {
    let mut guard = FS_WATCHER_SETTING.write().await;
    if Path::new(path).is_dir() {
        if guard.directories.contains(&path.to_string()) {
            return Ok(());
        }
        guard.directories.push(path.to_string().clone());
    } else {
        if guard.files.contains(&path.to_string()) {
            return Ok(());
        }
        guard.files.push(path.to_string().clone());
    }
    let json = serde_json::to_string(&*guard)?;
    drop(guard);
    config_repo::update_by_name(CONFIG_NAME_WATCHER_SETTING, &json)?;
    watch_path(path).await?;
    INCREMENT_WATCH_PATHS.write().await.insert(path.to_string());
    Ok(())
}

pub async fn remove_path(path: &str) -> Result<()> {
    let mut guard = FS_WATCHER_SETTING.write().await;
    if Path::new(path).is_dir() {
        guard.directories.retain(|directory| directory != &path);
    } else {
        guard.files.retain(|file_path| file_path != &path);
    }
    let json = serde_json::to_string(&*guard)?;
    drop(guard);
    config_repo::update_by_name(CONFIG_NAME_WATCHER_SETTING, &json)?;
    unwatch_path(&path).await?;
    INCREMENT_WATCH_PATHS.write().await.remove(path);
    Ok(())
}

async fn watch_path(path: &str) -> Result<()> {
    log::info!("Watching {}", path.to_string());
    if let Some(watcher) = WATCHER.get() {
        let mut guard = watcher.write().await;
        if let Err(e) = guard.watch(Path::new(path), RecursiveMode::Recursive) {
            log::error!("Watch {} failed: {}", path.to_string(), e);
        } else {
            log::info!("Watching success {}", path.to_string());
        }
    } else {
        log::error!("Watcher not initialized");
    }
    Ok(())
}

async fn unwatch_path(path: &str) -> Result<()> {
    log::info!("Unwatching {}", path.to_string());
    if let Some(watcher) = WATCHER.get() {
        let mut guard = watcher.write().await;
        if let Err(e) = guard.unwatch(Path::new(path)) {
            log::error!("Unwatching {} failed: {}", path.to_string(), e);
        } else {
            log::info!("Unwatching success {}", path.to_string());
        }
    } else {
        log::error!("Watcher not initialized");
    }
    Ok(())
}

/// Receive events, aggregate by path, process aggregated events after debounce window expires
async fn fs_event_loop(mut rx: Receiver<Event>, fs_event_normalizer: &mut FsEventNormalizer) {
    let debounce_duration = Duration::from_millis(1000);
    // pending: path -> aggregated EventKind
    let mut pending: HashMap<PathBuf, FsEvent> = HashMap::new();
    loop {
        if EXIT_APP_SIGNAL.load(Ordering::SeqCst) {
            log::info!("Exiting file system wather loop");
            break;
        }
        tokio::select! {
            maybe_ev = rx.recv() => {
                match maybe_ev {
                    Some(ev) => {
                        // Skip events inside the app's own data directory to prevent
                        // infinite loops (DB write → fs event → re-index → DB write → …)
                        if is_app_data_event(&ev).await {
                            continue;
                        }
                        let fs_event = fs_event_normalizer.handle(ev);
                        aggregate_event(&mut pending, fs_event);
                    }
                    None => {
                        // The queue is closed — flush pending and exit
                        flush_pending(&mut pending);
                        log::warn!("File system watcher channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(debounce_duration) => {
                flush_pending(&mut pending);
            }
        }
    }
}

/// Merge a single Event into pending (aggregate FsEvent by path)
fn aggregate_event(pending: &mut HashMap<PathBuf, FsEvent>, fs_events: Vec<FsEvent>) {
    if fs_events.is_empty() {
        return;
    }
    for fs_event in fs_events {
        let event_clone = (&fs_event).clone();
        match fs_event {
            FsEvent::Rename { from, to: _ } => {
                pending
                    .entry(from.clone())
                    .and_modify(|existing_fs_event| {
                        *existing_fs_event = merge_kinds(&existing_fs_event, &event_clone).clone();
                    })
                    .or_insert(event_clone);
            }
            _ => {
                for path in fs_event.paths() {
                    let event_clone = fs_event.clone();
                    pending
                        .entry(path.clone())
                        .and_modify(|existing_fs_event| {
                            *existing_fs_event =
                                merge_kinds(&existing_fs_event, &event_clone).clone();
                        })
                        .or_insert(event_clone);
                }
            }
        }
    }
}

/// Merge logic: Remove > Rename > Create > Modify > Other
fn merge_kinds<'a>(a_event: &'a FsEvent, b_event: &'a FsEvent) -> &'a FsEvent {
    match (&a_event, &b_event) {
        // If one is Remove, the result is Remove(Any)
        (FsEvent::Remove { .. }, _) | (_, FsEvent::Remove { .. }) => {
            if let FsEvent::Remove { .. } = a_event {
                a_event
            } else {
                b_event
            }
        }
        (FsEvent::Rename { .. }, _) | (_, FsEvent::Rename { .. }) => {
            if let FsEvent::Rename { .. } = a_event {
                a_event
            } else {
                b_event
            }
        }
        // If one is Create, the result is Create(Any)
        (FsEvent::Create(_), _) | (_, FsEvent::Create(_)) => {
            if let FsEvent::Create(_) = a_event {
                a_event
            } else {
                b_event
            }
        }
        // If one is Modify, the result is Modify(Any)
        (FsEvent::Modify(_), _) | (_, FsEvent::Modify(_)) => {
            if let FsEvent::Modify(_) = a_event {
                a_event
            } else {
                b_event
            }
        }
        // Otherwise return b (default to the latest)
        _ => b_event,
    }
}

fn flush_pending(pending: &mut HashMap<PathBuf, FsEvent>) {
    if pending.is_empty() {
        return;
    }
    for (path_buf, fs_event) in pending.drain() {
        log::info!("Processing aggregated event: {:?}", fs_event);
        let path = path_buf.to_string_lossy().to_string();
        match fs_event {
            FsEvent::Remove { path: _, is_file } => {
                if is_file {
                    indexer_service::remove_file_index(&path).unwrap_or_else(|error| {
                        log::error!("Failed to remove file index: {}", error);
                    })
                } else {
                    log::info!("Remove directory: {}", path);
                    indexer_service::remove_directory_index(&path).unwrap_or_else(|error| {
                        log::error!("Failed to remove directory index: {}", error);
                    })
                }
                tokio::task::spawn(async move {
                    path_search_engine::remove_from_index(&path, is_file).await;
                });
            }
            FsEvent::Rename { from, to } => {
                let from_path = from.to_string_lossy().to_string();
                let is_file = to.is_file();
                tokio::task::spawn({
                    let delete_path = from_path.clone();
                    async move {
                        path_search_engine::remove_from_index(&delete_path, is_file).await;
                    }
                });
                let target_path = to.to_string_lossy().to_string();
                if is_file {
                    log::debug!("Rename file: {} -> {}", from_path, target_path);
                    let file_info = file_info_repo::get_by_path(&from_path).unwrap_or(None);
                    match file_info {
                        Some(_) => {
                            let new_name = to
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            file_info_repo::rename(&from_path, &target_path, &new_name)
                                .unwrap_or_else(|error| {
                                    log::error!("Failed to rename file: {}", error);
                                    0
                                });
                        }
                        None => {
                            tokio::task::spawn(async move {
                                indexer_service::index_file(&target_path)
                                    .await
                                    .unwrap_or_else(|error| {
                                        log::error!("Failed to index new file: {}", error);
                                    });
                            });
                        }
                    }
                } else {
                    log::info!("Rename directory: {} -> {}", from_path, target_path);
                    let count = file_info_repo::count_by_prefix_path(&from_path).unwrap_or(0);
                    if count == 0 {
                        tokio::task::spawn(async move {
                            indexer_service::start_indexing(
                                vec![target_path],
                                INDEXING_FROM_WATCHER,
                            )
                            .await
                            .unwrap_or_else(|error: String| {
                                log::error!("Failed to index new directory: {}", error);
                                false
                            });
                        });
                    } else {
                        file_info_repo::replace_directory_prefix_path(&from_path, &target_path)
                            .unwrap_or_else(|error| {
                                log::error!("Failed to repalce path: {}", error);
                                0
                            });
                    }
                }
            }
            FsEvent::Create(create_path) => {
                if create_path.is_file() {
                    tokio::task::spawn(async move {
                        indexer_service::index_file(&path)
                            .await
                            .unwrap_or_else(|error| {
                                log::error!("Failed to index new file: {}", error);
                            });
                    });
                } else {
                    log::info!("Create directory: {}", path);
                }
            }
            FsEvent::Modify(modify_path) => {
                if modify_path.is_file() {
                    tokio::task::spawn(async move {
                        indexer_service::index_file(&path)
                            .await
                            .unwrap_or_else(|error| {
                                log::error!("Failed to index modified file: {}", error);
                            })
                    });
                }
            }
            _ => {
                log::debug!("event: Other")
            }
        }
    }
    log::debug!("--------------------------");
}

/// Check whether a file system event originated from inside the app's own data directory.
/// Events from the data directory (database, parsed_documents, model files, etc.) must be
/// ignored to prevent infinite feedback loops.
async fn is_app_data_event(event: &Event) -> bool {
    let app_data = crate::read_lock!(APP_DATA_PATH).clone();
    if app_data.is_empty() {
        return false;
    }
    event.paths.iter().any(|p| {
        let path_str = p.to_string_lossy();
        path_str.starts_with(&app_data)
    })
}
