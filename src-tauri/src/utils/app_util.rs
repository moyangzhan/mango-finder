use crate::enums::{FileContentLanguage, TrayMenuItem};
use crate::global::{
    APP_DATA_PATH, AUDIO_DECODER_NAME, AUDIO_DECODER_PATH, AUDIO_ENCODER_NAME, AUDIO_ENCODER_PATH,
    AUDIO_TOKENIZER_NAME, AUDIO_TOKENIZER_PATH, CONTENT_STORAGE_CHANGING, DB_PATH, DOWNLOADING,
    EMBEDDING_MODEL_NAME, EMBEDDING_MODEL_PATH, EMBEDDING_TOKENIZER_NAME, EMBEDDING_TOKENIZER_PATH,
    EXIT_APP_SIGNAL, EXTRACTED_IMAGES_PATH, HOME_PATH, INDEXING, OCR_CLS_MODEL_NAME,
    OCR_CLS_MODEL_PATH, OCR_DET_MODEL_NAME, OCR_DET_MODEL_PATH, OCR_DICT_NAME, OCR_DICT_PATH,
    OCR_REC_MODEL_NAME, OCR_REC_MODEL_PATH, SCANNING, STOP_INDEX_SIGNAL, STORAGE_PATH, TMP_PATH,
    TRAY_ID, VISION_MODEL_PATH, VISION_NAME, VISION_TOKENIZER_NAME, VISION_TOKENIZER_PATH,
    WHISPER_MODEL_NAME, WHISPER_MODEL_PATH,
};
use crate::utils::file_util;
use crate::utils::task_util;
use log::{error, info, warn};
use rust_i18n::t;
use std::env;
use std::fs::create_dir;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use sys_locale::get_locale;
use tauri::menu::{Menu, MenuItem};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

pub fn exit_app(app: &AppHandle) {
    let runing_tasks = running_background_tasks();
    if runing_tasks.len() > 0 {
        let answer = app
            .dialog()
            .message(t!(
                "message.abort-tasks-and-exit",
                tasks = runing_tasks.join(", ")
            ))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                t!("common.yes").to_string(),
                t!("common.no").to_string(),
            ))
            .blocking_show();
        if !answer {
            return;
        }
    }
    STOP_INDEX_SIGNAL.store(true, Ordering::SeqCst);
    EXIT_APP_SIGNAL.store(true, Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let windows = app.webview_windows();
    for (_, window) in windows {
        let _ = window.destroy();
    }
    app.exit(0);
}

pub fn running_background_tasks() -> Vec<&'static str> {
    let mut tasks = Vec::new();
    if DOWNLOADING.load(Ordering::SeqCst) {
        tasks.push("downloading");
    }
    if SCANNING.load(Ordering::SeqCst) {
        tasks.push("scanning");
    }
    if INDEXING.load(Ordering::SeqCst) {
        tasks.push("indexing");
    }
    if CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        tasks.push("content storage change");
    }
    tasks
}

pub fn show_app(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn create_tray_menu(app: &AppHandle) -> Result<Menu<Wry>, String> {
    let show_i = MenuItem::with_id(
        app,
        TrayMenuItem::Show.to_string(),
        t!("common.show"),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let quit_i = MenuItem::with_id(
        app,
        TrayMenuItem::Quit.to_string(),
        t!("common.quit"),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let result = Menu::with_items(app, &[&show_i, &quit_i]).map_err(|e| e.to_string())?;
    Ok(result)
}

pub fn rebuild_tray_menu(app: &AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let menu = create_tray_menu(app)?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn set_data_path(
    new_data_path: &str,
    force: bool,
    app: &AppHandle,
) -> Result<String, String> {
    if new_data_path.is_empty() {
        error!("new data path is empty");
        return Err("new data path is empty".to_string());
    }
    if new_data_path == *crate::read_lock!(APP_DATA_PATH) {
        return Ok("same".to_string());
    }

    // Guard: reject if indexing or content storage change is in progress
    if INDEXING.load(Ordering::SeqCst) || CONTENT_STORAGE_CHANGING.load(Ordering::SeqCst) {
        return Err(
            "Cannot change data path while indexing or content storage change is in progress"
                .to_string(),
        );
    }

    // DB persistent lock
    let old_data_path_val = crate::read_lock!(APP_DATA_PATH).clone();
    task_util::lock_active_task(&task_util::ActiveTask {
        old_path: Some(old_data_path_val.clone()),
        ..task_util::ActiveTask::new("data_copying")
    })
    .map_err(|e| e.to_string())?;

    let ndp = PathBuf::from(new_data_path);

    // Check write permission before attempting to copy files
    {
        let storage_dir = ndp.join("storage");
        if let Err(e) = std::fs::create_dir_all(&storage_dir) {
            let _ = task_util::unlock_active_task();
            return Err(format!(
                "Cannot write to directory '{}': {}. The directory may require administrator privileges.",
                ndp.display(),
                e
            ));
        }
        // Probe write permission by creating and removing a temp file
        let probe = storage_dir.join(".mango_write_probe");
        if let Err(e) = std::fs::write(&probe, "probe") {
            let _ = task_util::unlock_active_task();
            return Err(format!(
                "Cannot write to directory '{}': {}. The directory may require administrator privileges.",
                ndp.display(),
                e
            ));
        }
        let _ = std::fs::remove_file(&probe);
    }

    if !force {
        let mut exist_files = "".to_string();
        if ndp.join("storage").join("mango-finder.db").exists() {
            exist_files.push_str("mango-finder.db, ");
        }
        if !exist_files.is_empty() {
            let _ = task_util::unlock_active_task();
            return Ok("exist:".to_string() + &exist_files);
        }
    }
    let sys_data_path = app
        .path()
        .data_dir()
        .unwrap_or_else(|error| {
            error!("Failed to get user data directory:{}", error);
            PathBuf::from("./")
        })
        .join(env!("CARGO_PKG_NAME"));
    info!("system data path: {}", sys_data_path.display());
    let old_data_path = crate::read_lock!(APP_DATA_PATH).clone();
    let config_path = sys_data_path.join(".config");
    if let Err(e) = std::fs::write(&config_path, new_data_path) {
        error!("Failed to write data path record: {}", e);
    }
    let old_path_buf = PathBuf::from(old_data_path);
    let old_storage = old_path_buf.join("storage");
    let new_storage = ndp.join("storage");
    if old_path_buf.exists() {
        let old_db = old_storage.join("mango-finder.db");
        if old_db.exists() {
            let new_db = new_storage.join("mango-finder.db");
            if let Err(e) = file_util::copy_file(&old_db, &new_db) {
                let _ = task_util::unlock_active_task();
                return Err(format!("Failed to copy db file: {}", e));
            }
        }
        // Copy Markdown parsed documents and extracted images
        for dir_name in &["parsed_documents", "extracted_images"] {
            let old_dir = old_storage.join(dir_name);
            if old_dir.exists() {
                let new_dir = new_storage.join(dir_name);
                if let Err(e) = file_util::copy_dir(&old_dir, &new_dir) {
                    log::warn!("Failed to copy {} directory: {}", dir_name, e);
                }
            }
        }
    }
    *crate::write_lock!(APP_DATA_PATH) = new_data_path.to_string();
    let _ = task_util::unlock_active_task();
    Ok("success".to_string())
}

pub async fn reset_data_path(force: bool, app: &AppHandle) -> Result<String, String> {
    let sys_data_path = app
        .path()
        .data_dir()
        .unwrap_or_else(|error| {
            error!("Failed to get user data directory:{}", error);
            PathBuf::from("./")
        })
        .join(env!("CARGO_PKG_NAME"));
    set_data_path(sys_data_path.to_str().unwrap_or(""), force, app).await
}

/// If the {user data directory}/.config file exists, read the path from it. Otherwise, use {user data directory} as default.
pub async fn init_paths(app: &AppHandle) {
    let data_base = app
        .path()
        .data_dir()
        .unwrap_or_else(|error| {
            error!("Failed to get user data directory:{}", error);
            PathBuf::from("./")
        })
        .join(env!("CARGO_PKG_NAME"));
    info!("system data path: {}", data_base.display());
    init_paths_core(data_base);
    let resource_dir = app.path().resource_dir().unwrap_or_else(|e| {
        error!("Failed to get resource directory: {}", e);
        PathBuf::from(app_home_dir()).join("assets")
    });
    info!("Resource directory: {}", resource_dir.display());
    init_model_assets(&resource_dir);
}

/// App home directory: cwd in debug builds, the executable directory in
/// releases. Single source for HOME_PATH and the resource-dir fallback.
/// 应用主目录:debug 为工作目录,release 为可执行文件目录;HOME_PATH 与
/// resource 目录回落的唯一来源。
fn app_home_dir() -> String {
    #[cfg(debug_assertions)]
    {
        env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|e| {
                error!("Failed to get current directory: {}", e);
                "./".to_string()
            })
    }
    #[cfg(not(debug_assertions))]
    {
        env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                error!("Failed to get executable directory");
                "./".to_string()
            })
    }
}

/// Core data-path init shared by the GUI and the CLI: .config override →
/// APP_DATA_PATH → storage/db/tmp globals. Failures log and continue
/// (GUI contract); callers that need fail-fast check the globals afterwards.
/// GUI/CLI 共用数据路径 core:.config 覆盖 → APP_DATA_PATH → storage/db/tmp
/// 全局。失败记日志继续(GUI 契约);需 fail-fast 的调用方事后检查全局。
pub fn init_paths_core(mut data_path: PathBuf) {
    let config_path = data_path.join(".config");
    info!("MangoFinder config file: {}", config_path.display());
    // Default to use user data directory. Otherwise, use the path from .config file.
    if !config_path.exists() {
        if let Err(e) = std::fs::write(&config_path, data_path.to_str().unwrap_or("")) {
            error!("Failed to write data path record: {}", e);
        }
    } else {
        match std::fs::File::open(&config_path) {
            Ok(file) => {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(file);
                if let Some(Ok(first_line)) = reader.lines().next() {
                    if !first_line.is_empty() {
                        let custom_path = PathBuf::from(&first_line);
                        if custom_path.is_dir() {
                            data_path = custom_path;
                            info!("read data_dir from config file: {}", data_path.display());
                        } else {
                            warn!(
                                "Config path '{}' is not a valid directory, using default",
                                first_line
                            );
                        }
                    } else {
                        warn!("Empty line in data path record, using default directory");
                    }
                } else {
                    error!("Failed to read data path record: empty line");
                }
            }
            Err(e) => {
                error!("Failed to read data path record: {}", e);
            }
        }
    }
    info!("data_path: {}", data_path.display());
    let data_path_str = data_path.to_string_lossy().into_owned();
    info!("MangoFinder data directory: {}", data_path_str);
    *crate::write_lock!(APP_DATA_PATH) = data_path_str;
    if !Path::new(&data_path).exists() {
        create_dir(&data_path).unwrap_or_else(|error| {
            error!("Failed to create app directory: {}", error);
        });
    }
    // Define storage path
    let storage_path = Path::new(&data_path).join("storage");
    if !storage_path.exists() {
        create_dir(&storage_path).unwrap_or_else(|error| {
            error!("Failed to create storage directory: {}", error);
        });
    }
    STORAGE_PATH
        .set(storage_path.to_string_lossy().into_owned())
        .unwrap_or_else(|e| error!("Failed to set STORAGE_PATH: {}", e));
    info!(
        "Storage directory: {}",
        STORAGE_PATH.get().unwrap_or(&String::new())
    );
    let db_path = Path::new(&data_path)
        .join("storage")
        .join("mango-finder.db");
    DB_PATH
        .set(db_path.to_string_lossy().into_owned())
        .unwrap_or_else(|error| error!("Failed to set DB_PATH: {}", error));
    info!(
        "Database path: {}",
        DB_PATH.get().unwrap_or(&String::new()).to_string()
    );
    // Tmp download directory
    let tmp_path = data_path.join("tmp");
    if !tmp_path.exists() {
        create_dir(&tmp_path).unwrap_or_else(|error| {
            error!("Failed to create tmp directory: {}", error);
        });
    }
    TMP_PATH
        .set(tmp_path.to_string_lossy().into_owned())
        .unwrap_or_else(|e| error!("Failed to set TMP_DOWNLOAD_DIRECTORY: {}", e));
    info!(
        "Temp path: {}",
        TMP_PATH.get().unwrap_or(&String::new()).to_string()
    );
}

/// Core model-asset init shared by the GUI and the CLI: HOME_PATH → 12 model
/// paths under {resource_base}/assets/model → extracted_images. Failures log
/// and continue (GUI contract).
/// GUI/CLI 共用模型资产 core:HOME_PATH → {resource_base}/assets/model 下
/// 12 个模型路径 → extracted_images。失败记日志继续(GUI 契约)。
pub fn init_model_assets(resource_base: &Path) {
    let app_dir = app_home_dir();
    if HOME_PATH.set(app_dir.clone()).is_err() {
        error!("Warning: HOME_PATH was already set");
    }
    info!("Home directory: {}", app_dir);
    let build_in_model_path = resource_base.join("assets").join("model");
    if !build_in_model_path.exists() {
        create_dir(&build_in_model_path).unwrap_or_else(|error| {
            error!("Failed to create assets directory: {}", error);
        });
    }

    let multilingual_embedding_path = build_in_model_path
        .join(EMBEDDING_MODEL_NAME)
        .to_string_lossy()
        .into_owned();
    EMBEDDING_MODEL_PATH
        .set(multilingual_embedding_path)
        .unwrap_or_else(|e| error!("Failed to set EMBEDDING_MODEL_PATH: {}", e));
    info!(
        "Multi-language embedding model path: {}",
        EMBEDDING_MODEL_PATH
            .get()
            .unwrap_or(&String::new())
            .to_string()
    );
    let multilingual_tokenizer_path = build_in_model_path
        .join(EMBEDDING_TOKENIZER_NAME)
        .to_string_lossy()
        .into_owned();
    EMBEDDING_TOKENIZER_PATH
        .set(multilingual_tokenizer_path)
        .unwrap_or_else(|e| error!("Failed to set EMBEDDING_TOKENIZER_PATH: {}", e));
    info!(
        "Multi-language tokenizer path: {}",
        EMBEDDING_TOKENIZER_PATH
            .get()
            .unwrap_or(&String::new())
            .to_string()
    );

    // Vision model
    let vision_path = build_in_model_path
        .join(VISION_NAME)
        .to_string_lossy()
        .into_owned();
    VISION_MODEL_PATH
        .set(vision_path)
        .unwrap_or_else(|e| error!("Failed to set VISION_0_PATH: {}", e));
    info!(
        "Vision model path: {}",
        VISION_MODEL_PATH
            .get()
            .unwrap_or(&String::new())
            .to_string()
    );

    let vision_tokenizer_path = build_in_model_path
        .join(VISION_TOKENIZER_NAME)
        .to_string_lossy()
        .into_owned();
    VISION_TOKENIZER_PATH
        .set(vision_tokenizer_path)
        .unwrap_or_else(|e| error!("Failed to set VISION_TOKENIZER_PATH: {}", e));

    // Audio model
    let audio_encoder_path = build_in_model_path
        .join(AUDIO_ENCODER_NAME)
        .to_string_lossy()
        .into_owned();
    AUDIO_ENCODER_PATH
        .set(audio_encoder_path)
        .unwrap_or_else(|e| {
            error!("Failed to set AUDIO_ENCODER_PATH: {}", e);
        });
    let audio_decoder_path = build_in_model_path
        .join(AUDIO_DECODER_NAME)
        .to_string_lossy()
        .into_owned();
    AUDIO_DECODER_PATH
        .set(audio_decoder_path)
        .unwrap_or_else(|e| {
            error!("Failed to set AUDIO_DECODER_PATH: {}", e);
        });
    let audio_tokenizer_path = build_in_model_path
        .join(AUDIO_TOKENIZER_NAME)
        .to_string_lossy()
        .into_owned();
    AUDIO_TOKENIZER_PATH
        .set(audio_tokenizer_path)
        .unwrap_or_else(|e| {
            error!("Failed to set AUDIO_TOKENIZER_PATH: {}", e);
        });

    // Whisper.cpp model
    let whisper_model_path = build_in_model_path
        .join(WHISPER_MODEL_NAME)
        .to_string_lossy()
        .into_owned();
    WHISPER_MODEL_PATH
        .set(whisper_model_path)
        .unwrap_or_else(|e| {
            error!("Failed to set WHISPER_MODEL_PATH: {}", e);
        });

    // OCR models (PaddleOCR)
    let ocr_det_path = build_in_model_path
        .join(OCR_DET_MODEL_NAME)
        .to_string_lossy()
        .into_owned();
    OCR_DET_MODEL_PATH.set(ocr_det_path).unwrap_or_else(|e| {
        error!("Failed to set OCR_DET_MODEL_PATH: {}", e);
    });
    let ocr_cls_path = build_in_model_path
        .join(OCR_CLS_MODEL_NAME)
        .to_string_lossy()
        .into_owned();
    OCR_CLS_MODEL_PATH.set(ocr_cls_path).unwrap_or_else(|e| {
        error!("Failed to set OCR_CLS_MODEL_PATH: {}", e);
    });
    let ocr_rec_path = build_in_model_path
        .join(OCR_REC_MODEL_NAME)
        .to_string_lossy()
        .into_owned();
    OCR_REC_MODEL_PATH.set(ocr_rec_path).unwrap_or_else(|e| {
        error!("Failed to set OCR_REC_MODEL_PATH: {}", e);
    });
    let ocr_dict_path = build_in_model_path
        .join(OCR_DICT_NAME)
        .to_string_lossy()
        .into_owned();
    OCR_DICT_PATH.set(ocr_dict_path).unwrap_or_else(|e| {
        error!("Failed to set OCR_DICT_PATH: {}", e);
    });

    // Extracted images directory
    let extracted_images_path =
        Path::new(STORAGE_PATH.get().unwrap_or(&String::new())).join("extracted_images");
    if !extracted_images_path.exists() {
        create_dir(&extracted_images_path).unwrap_or_else(|error| {
            error!("Failed to create extracted_images directory: {}", error);
        });
    }
    EXTRACTED_IMAGES_PATH
        .set(extracted_images_path.to_string_lossy().into_owned())
        .unwrap_or_else(|e| {
            error!("Failed to set EXTRACTED_IMAGES_PATH: {}", e);
        });
}

pub fn get_db_path() -> String {
    DB_PATH.get().unwrap_or(&String::new()).to_string()
}

pub fn get_assets_tmp_path() -> String {
    TMP_PATH.get().unwrap_or(&String::new()).to_string()
}

pub fn get_multilingual_embedding_path() -> String {
    EMBEDDING_MODEL_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_multilingual_tokenizer_path() -> String {
    EMBEDDING_TOKENIZER_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_vision_0_path() -> String {
    VISION_MODEL_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_vision_tokenizer_path() -> String {
    VISION_TOKENIZER_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_audio_encoder_path() -> String {
    AUDIO_ENCODER_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_audio_decoder_path() -> String {
    AUDIO_DECODER_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_audio_tokenizer_path() -> String {
    AUDIO_TOKENIZER_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_whisper_model_path() -> String {
    WHISPER_MODEL_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_ocr_det_model_path() -> String {
    OCR_DET_MODEL_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_ocr_cls_model_path() -> String {
    OCR_CLS_MODEL_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_ocr_rec_model_path() -> String {
    OCR_REC_MODEL_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_ocr_dict_path() -> String {
    OCR_DICT_PATH.get().unwrap_or(&String::new()).to_string()
}

pub fn get_extracted_images_path() -> String {
    EXTRACTED_IMAGES_PATH
        .get()
        .unwrap_or(&String::new())
        .to_string()
}

pub fn get_default_file_content_language() -> FileContentLanguage {
    let locale = get_locale().unwrap_or_else(|| String::from("en-US"));
    log::debug!("locale: {}", locale);
    match locale.as_str() {
        l if l.starts_with("zh") => FileContentLanguage::Chinese,
        l if l.starts_with("en") => FileContentLanguage::English,
        _ => FileContentLanguage::English, // 默认回退
    }
}

/// CLI 模式路径初始化(不依赖 Tauri):与 GUI 共用 core,仅替换数据目录与
/// 模型目录的解析来源;core 失败只记日志(GUI 契约),由末尾全局检查兜底
/// CLI path init (Tauri-free): shares the GUI cores; only the data-dir and
/// model-dir resolution differ. Core failures just log (GUI contract) — the
/// trailing global check is the safety net.
pub fn init_paths_standalone() -> Result<(), String> {
    // 1. 数据目录:dirs::data_dir() 与 Tauri path().data_dir() 三平台同源
    // Data dir: dirs::data_dir() matches Tauri's data_dir on all platforms
    let data_base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(env!("CARGO_PKG_NAME"));
    init_paths_core(data_base);

    // 2. 模型目录:优先可执行文件目录,回退当前目录
    // (覆盖从源码目录直接运行 mfd 的开发流),承担 GUI resource_dir 的角色
    // Model dir: prefer the executable directory, fall back to the current
    // directory (dev flow running mfd from the source tree); plays the role
    // of the GUI's resource_dir.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let cwd = std::env::current_dir().unwrap_or_default();
    let resource_base = if exe_dir.join("assets").join("model").exists() {
        exe_dir
    } else if cwd.join("assets").join("model").exists() {
        cwd
    } else {
        exe_dir
    };
    init_model_assets(&resource_base);

    // 3. fail-fast:核心未就绪即报错(细节见日志)
    // Fail fast when the cores left globals unset (details are in the logs)
    if STORAGE_PATH.get().is_none() || DB_PATH.get().is_none() {
        return Err("data path globals were not initialized (see logs)".to_string());
    }
    Ok(())
}
