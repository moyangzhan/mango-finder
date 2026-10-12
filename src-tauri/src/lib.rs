#![allow(dead_code)]

// Public modules (CLI accessible)
pub mod db_initializer;
pub mod entities;
pub mod enums;
pub mod global;
pub mod indexer_service;
pub mod initializer;
pub mod repositories;
pub mod searcher;
pub mod similarity;
pub mod structs;
pub mod utils;

// Private modules
mod audio_parser;
pub mod cluster;
mod commands;
mod db_init_manager;
mod db_migrations;
mod document_loaders;
mod embedding_service;
mod embedding_service_manager;
mod errors;
mod fs_watcher;
mod image_parser;
mod indexers;
mod model_platform_services;
mod one_time_job;
mod ocr_service;
mod scanner;
mod self_hosted_services;
mod timers;
mod traits;
mod types;

use crate::commands::{
    add_device_manually, add_watch_path, change_content_storage, check_devices,
    check_devices_status, check_model_platform, check_path_type, check_self_hosted_platform,
    clear_active_task, clear_index, clear_pairing_requests, count_files, count_indexed_files,
    count_indexing_tasks, count_pending_pairing_requests, delete_index_item, delete_index_items,
    delete_indexing_task, delete_pairing_request, delete_pairing_requests,
    download_multilingual_model, fetch_remote_file, find_similars_by_file_id, get_active_task,
    get_client_id, get_data_path, get_local_ip, get_test_mode, indexing_watch_paths,
    is_embedding_model_changed, list_online_devices, load_active_locale, load_active_platform,
    load_active_self_hosted_platform, load_chunks, load_cluster_setting, load_config_value,
    load_devices, load_file_detail, load_files, load_indexer_setting, load_indexing_tasks,
    load_model_by_type, load_model_platforms, load_pairing_requests, load_pending_pairing_requests,
    load_proxy_info, load_self_hosted_platforms, local_device_search, open_directory,
    read_file_data, reject_device, remote_device_search, remove_watch_path, reset_data_path,
    reset_pairing_status, respond_pairing_request, retry_data_copy, revert_data_path,
    send_pairing_request, set_active_locale, set_active_platform, set_active_self_hosted_platform,
    set_data_path, start_indexing, stop_indexing, toggle_cluster, toggle_test_mode, ui_mounted,
    unreject_device, update_ai_model, update_cluster_setting, update_config,
    update_indexer_setting, update_model_platform, update_proxy_info, update_self_hosted_platform,
};
use crate::global::{APP_HANDLE, UI_MOUNTED};
use crate::repositories::file_content_embedding_repo;
use crate::utils::app_util;
use global::TRAY_ID;
use log::{error, info};
use rusqlite::ffi::sqlite3_auto_extension;
use sqlite_vec::sqlite3_vec_init;
use std::env;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri::WindowEvent;
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en-US");

static WARMUP_DONE: AtomicBool = AtomicBool::new(false);

/// Register the sqlite-vec extension so every SQLite connection opened in this
/// process has the `vec0` virtual table module available. GUI 和 CLI 都必须在
/// 任何数据库访问之前调用一次。
pub fn init_sqlite_extensions() {
    // SAFETY: sqlite3_vec_init 是 sqlite-vec 扩展的入口点。
    // 虽然 sqlite3_auto_extension 期望的回调签名是 fn(*mut sqlite3) -> c_int，
    // 但 SQLite 允许入口点函数忽略参数，这是 sqlite-vec 官方推荐的初始化方式。
    // 参考: https://github.com/asg017/sqlite-vec
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_sqlite_extensions();
    panic::set_hook(Box::new(|panic_info| {
        let message = format!("Application crashed: {:?}", panic_info);
        error!("{}", message);
        //TODO add file logger
    }));
    let mut builder = tauri::Builder::default().plugin(tauri_plugin_process::init());
    let log_path = {
        #[cfg(debug_assertions)]
        {
            let current_dir = env::current_dir()
                .map(|p| p.join("src-tauri"))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|e| {
                    error!("Failed to get current directory: {}, using app data dir", e);
                    String::new()
                });
            Some(current_dir)
        }
        #[cfg(not(debug_assertions))]
        {
            None
        }
    };
    #[cfg(not(target_os = "android"))]
    #[cfg(not(target_os = "ios"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app
                .get_webview_window("main")
                .expect("no main window")
                .set_focus();
        }));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .timezone_strategy(TimezoneStrategy::UseLocal)
                .targets([
                    Target::new(TargetKind::LogDir {
                        file_name: log_path,
                    }),
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::Webview),
                ])
                .rotation_strategy(RotationStrategy::KeepSome(5))
                .max_file_size(1024 * 1024 * 5)
                .level(log::LevelFilter::Debug)
                .filter(|metadata| {
                    // Suppress noisy mDNS logs about invalid addresses
                    // These are normal for interfaces without IPv4 addresses (WSL, virtual adapters, etc.)
                    if metadata.target().starts_with("mdns_sd") {
                        // return metadata.level() < log::Level::Warn;
                        return false;
                    }
                    // Suppress noisy polling/iocp TRACE logs (Tokio async I/O polling)
                    // These are normal async I/O events, not errors
                    if metadata.target().starts_with("polling") {
                        return metadata.level() <= log::Level::Debug;
                    }
                    true
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            local_device_search,
            load_active_locale,
            load_model_platforms,
            load_proxy_info,
            load_active_platform,
            load_indexer_setting,
            load_model_by_type,
            update_ai_model,
            update_ai_model,
            load_indexing_tasks,
            load_files,
            load_file_detail,
            load_config_value,
            load_chunks,
            count_indexing_tasks,
            count_files,
            set_active_platform,
            set_active_locale,
            update_model_platform,
            update_proxy_info,
            update_indexer_setting,
            start_indexing,
            stop_indexing,
            indexing_watch_paths,
            download_multilingual_model,
            check_path_type,
            delete_indexing_task,
            delete_index_item,
            delete_index_items,
            clear_index,
            read_file_data,
            ui_mounted,
            is_embedding_model_changed,
            change_content_storage,
            count_indexed_files,
            get_data_path,
            set_data_path,
            reset_data_path,
            add_watch_path,
            remove_watch_path,
            find_similars_by_file_id,
            get_client_id,
            open_directory,
            // Self-hosted platform commands
            load_self_hosted_platforms,
            load_active_self_hosted_platform,
            set_active_self_hosted_platform,
            update_self_hosted_platform,
            update_config,
            update_cluster_setting,
            load_cluster_setting,
            toggle_cluster,
            check_devices,
            check_devices_status,
            get_local_ip,
            check_model_platform,
            check_self_hosted_platform,
            load_devices,
            load_pairing_requests,
            load_pending_pairing_requests,
            count_pending_pairing_requests,
            delete_pairing_request,
            delete_pairing_requests,
            clear_pairing_requests,
            respond_pairing_request,
            send_pairing_request,
            reject_device,
            unreject_device,
            reset_pairing_status,
            add_device_manually,
            // Remote device search commands
            remote_device_search,
            list_online_devices,
            fetch_remote_file,
            // Test mode commands
            toggle_test_mode,
            get_test_mode,
            // Active task commands
            get_active_task,
            clear_active_task,
            retry_data_copy,
            revert_data_path,
        ])
        .setup(|app| {
            APP_HANDLE.set(app.handle().clone()).unwrap_or_else(|_| {
                error!("Failed to set APP_HANDLE");
            });
            let app_handle = app.handle();
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                app_util::init_paths(app_handle).await;
                initializer::process(false).await;
            });
            let menu = app_util::create_tray_menu(app_handle)?;
            let tray_builder = TrayIconBuilder::with_id(TRAY_ID);
            let tray_builder = if let Some(icon) = app.default_window_icon() {
                tray_builder.icon(icon.clone())
            } else {
                tray_builder
            };
            let _ = tray_builder
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } => {
                        let app = tray.app_handle();
                        app_util::show_app(app);
                    }
                    _ => {}
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        app_util::show_app(app);
                    }
                    "quit" => {
                        info!("Exit menu item was clicked");
                        app_util::exit_app(&app);
                    }
                    _ => {}
                })
                .build(app)?;

            let window = app
                .get_webview_window("main")
                .expect("main window not found");
            window.on_window_event(|event| match event {
                WindowEvent::Focused(focused) => {
                    if *focused
                        && UI_MOUNTED.load(Ordering::SeqCst)
                        && file_content_embedding_repo::count().unwrap_or(0) > 0
                        && WARMUP_DONE.load(Ordering::SeqCst)
                    {
                        WARMUP_DONE.store(false, Ordering::SeqCst);
                        tauri::async_runtime::spawn(async move {
                            searcher::semantic_search_engine::warmup_embedding_service()
                                .await
                                .unwrap_or_else(|error| {
                                    log::error!(
                                        "first warming up embedding service error: {}",
                                        error
                                    );
                                });
                            WARMUP_DONE.store(true, Ordering::SeqCst);
                        });
                    }
                }
                _ => {}
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                if let Err(error) = window.hide() {
                    error!("error hiding window: {}", error);
                } else {
                    api.prevent_close();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
