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
use std::sync::OnceLock;
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
    let mut data_path = app
        .path()
        .data_dir()
        .unwrap_or_else(|error| {
            error!("Failed to get user data directory:{}", error);
            PathBuf::from("./")
        })
        .join(env!("CARGO_PKG_NAME"));
    info!("system data path: {}", data_path.display());
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

    init_embedding_model_path(app);
}

fn init_embedding_model_path(app_handle: &AppHandle) {
    let app_dir = {
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
    };
    if HOME_PATH.set(app_dir.clone()).is_err() {
        error!("Warning: HOME_PATH was already set");
    }
    info!("Home directory: {}", app_dir);
    let resource_dir = app_handle.path().resource_dir().unwrap_or_else(|e| {
        error!("Failed to get resource directory: {}", e);
        PathBuf::from(app_dir).join("assets")
    });
    info!("Resource directory: {}", resource_dir.display());
    let build_in_model_path = resource_dir.join("assets").join("model");
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

/// CLI 模式路径初始化（不依赖 Tauri）
pub fn init_paths_standalone() -> Result<(), String> {
    // 1. HOME_PATH：可执行文件所在目录
    let exe_path =
        std::env::current_exe().map_err(|e| format!("Failed to get executable path: {}", e))?;
    let home_path = exe_path
        .parent()
        .ok_or("Failed to get executable directory")?
        .to_string_lossy()
        .into_owned();
    HOME_PATH
        .set(home_path.clone())
        .map_err(|_| "HOME_PATH already set".to_string())?;
    info!("Home directory: {}", home_path);

    // 2. APP_DATA_PATH：系统 AppData 目录（支持自定义路径）
    let mut data_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(env!("CARGO_PKG_NAME"));

    // 读取 .config 文件获取自定义数据路径（与 Tauri 版本保持一致）
    let config_path = data_path.join(".config");
    if config_path.exists() {
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
                    }
                }
            }
            Err(e) => {
                error!("Failed to read data path record: {}", e);
            }
        }
    }

    let data_path_str = data_path.to_string_lossy().into_owned();
    *APP_DATA_PATH.write().unwrap() = data_path_str.clone();
    info!("Data directory: {}", data_path_str);

    // 创建数据目录
    if !data_path.exists() {
        std::fs::create_dir_all(&data_path)
            .map_err(|e| format!("Failed to create data dir: {}", e))?;
    }

    // 3. STORAGE_PATH
    let storage_path = data_path.join("storage");
    std::fs::create_dir_all(&storage_path)
        .map_err(|e| format!("Failed to create storage dir: {}", e))?;
    STORAGE_PATH
        .set(storage_path.to_string_lossy().into_owned())
        .map_err(|_| "STORAGE_PATH already set".to_string())?;
    info!("Storage directory: {}", storage_path.display());

    // 4. DB_PATH
    let db_path = storage_path.join("mango-finder.db");
    DB_PATH
        .set(db_path.to_string_lossy().into_owned())
        .map_err(|_| "DB_PATH already set".to_string())?;
    info!("Database path: {}", db_path.display());

    // 5. 模型路径:优先可执行文件目录,不存在时回退当前目录
    // (与 GUI debug 模式的解析一致,覆盖从源码目录直接运行 mfd 的开发流)
    // 5. Model path: prefer the executable directory; fall back to the
    // current directory when absent (mirrors the GUI debug-mode resolution,
    // covering the dev flow of running mfd straight from the source tree).
    let exe_model = PathBuf::from(&home_path).join("assets").join("model");
    let model_path = if exe_model.exists() {
        exe_model
    } else {
        let cwd_model = std::env::current_dir()
            .unwrap_or_default()
            .join("assets")
            .join("model");
        if cwd_model.exists() {
            cwd_model
        } else {
            exe_model
        }
    };
    init_model_paths(&model_path);

    // 6. TMP_PATH
    let tmp_path = data_path.join("tmp");
    std::fs::create_dir_all(&tmp_path).map_err(|e| format!("Failed to create tmp dir: {}", e))?;
    TMP_PATH
        .set(tmp_path.to_string_lossy().into_owned())
        .map_err(|_| "TMP_PATH already set".to_string())?;
    info!("Temp directory: {}", tmp_path.display());

    // 7. EXTRACTED_IMAGES_PATH
    let extracted_images_path = storage_path.join("extracted_images");
    std::fs::create_dir_all(&extracted_images_path)
        .map_err(|e| format!("Failed to create extracted_images dir: {}", e))?;
    EXTRACTED_IMAGES_PATH
        .set(extracted_images_path.to_string_lossy().into_owned())
        .map_err(|_| "EXTRACTED_IMAGES_PATH already set".to_string())?;

    Ok(())
}

/// 模型路径初始化（复用）
fn init_model_paths(model_path: &Path) {
    let set_path = |once: &OnceLock<String>, name: &str| {
        let p = model_path.join(name).to_string_lossy().into_owned();
        once.set(p)
            .unwrap_or_else(|_| log::warn!("{} already set", name));
    };

    set_path(&EMBEDDING_MODEL_PATH, EMBEDDING_MODEL_NAME);
    set_path(&EMBEDDING_TOKENIZER_PATH, EMBEDDING_TOKENIZER_NAME);
    set_path(&VISION_MODEL_PATH, VISION_NAME);
    set_path(&VISION_TOKENIZER_PATH, VISION_TOKENIZER_NAME);
    set_path(&AUDIO_ENCODER_PATH, AUDIO_ENCODER_NAME);
    set_path(&AUDIO_DECODER_PATH, AUDIO_DECODER_NAME);
    set_path(&AUDIO_TOKENIZER_PATH, AUDIO_TOKENIZER_NAME);
    set_path(&WHISPER_MODEL_PATH, WHISPER_MODEL_NAME);
    set_path(&OCR_DET_MODEL_PATH, OCR_DET_MODEL_NAME);
    set_path(&OCR_CLS_MODEL_PATH, OCR_CLS_MODEL_NAME);
    set_path(&OCR_REC_MODEL_PATH, OCR_REC_MODEL_NAME);
    set_path(&OCR_DICT_PATH, OCR_DICT_NAME);
}
