use crate::document_loaders::anytomd_loader::AnyToMdLoader;
use crate::document_loaders::odp::OdpLoader;
use crate::document_loaders::ods::OdsLoader;
use crate::document_loaders::odt::OdtLoader;
use crate::document_loaders::pdfplumber_loader::PdfPlumberLoader;
use crate::entities::{ModelPlatform, SelfHostedPlatform};
use crate::structs::fs_watcher_setting::FsWatcherSetting;
use crate::structs::indexer_setting::IndexerSetting;
use crate::structs::indexing_summary::IndexingSummary;
use crate::structs::proxy_setting::ProxyInfo;
use crate::traits::document_loader::DocumentLoader;
use chrono::{DateTime, Local};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, LazyLock, OnceLock, RwLock};
use tauri::AppHandle;
use tokio::sync::RwLock as AsyncRwLock;

pub const DB_VERSION: i32 = 5;
pub const DEFAULT_PAGE_SIZE: usize = 20;
pub const HUGGINFACE_WEBSITE: &str = "https://huggingface.co";
pub const HUGGINFACE_MIRROR: &str = "https://hf-mirror.com";
// url: https://huggingface.co/moyangzhan/multilingual-e5-base-onnx (768 dimensions)
pub const MULTI_LANG_MODEL_URL: &str = "https://huggingface.co/moyangzhan/multilingual-e5-base-onnx/resolve/main/model_opt2_QInt8.onnx";
pub const MULTI_LANG_TOKENIZER_URL: &str =
    "https://huggingface.co/moyangzhan/multilingual-e5-base-onnx/resolve/main/tokenizer.json";
pub const EMBEDDING_MODEL_NAME: &str = "embedding.onnx";
pub const EMBEDDING_TOKENIZER_NAME: &str = "embedding_tokenizer.json";

// vision model
pub const VISION_NAME: &str = "vision.onnx";
pub const VISION_TOKENIZER_NAME: &str = "vision_tokenizer.json";

// audio model
pub const AUDIO_ENCODER_NAME: &str = "audio_encoder.onnx";
pub const AUDIO_DECODER_NAME: &str = "audio_decoder.onnx";
pub const AUDIO_TOKENIZER_NAME: &str = "audio_tokenizer.json";

// whisper.cpp model
pub const WHISPER_MODEL_NAME: &str = "whisper-small-q8_0.bin";

// OCR models (PaddleOCR PP-OCRv4)
pub const OCR_DET_MODEL_NAME: &str = "ch_PP-OCRv4_det_infer.onnx";
pub const OCR_CLS_MODEL_NAME: &str = "ch_ppocr_mobile_v2.0_cls_infer.onnx";
pub const OCR_REC_MODEL_NAME: &str = "ch_PP-OCRv4_rec_infer.onnx";
pub const OCR_DICT_NAME: &str = "ppocr_keys_v1.txt";

// assets/model/model.onnx
pub static EMBEDDING_MODEL_PATH: OnceLock<String> = OnceLock::new();
// assets/model/tokenizer.json
pub static EMBEDDING_TOKENIZER_PATH: OnceLock<String> = OnceLock::new();
pub static VISION_MODEL_PATH: OnceLock<String> = OnceLock::new();
pub static VISION_TOKENIZER_PATH: OnceLock<String> = OnceLock::new();

// Audio model
pub static AUDIO_ENCODER_PATH: OnceLock<String> = OnceLock::new();
pub static AUDIO_DECODER_PATH: OnceLock<String> = OnceLock::new();
pub static AUDIO_TOKENIZER_PATH: OnceLock<String> = OnceLock::new();

// Whisper.cpp model path
pub static WHISPER_MODEL_PATH: OnceLock<String> = OnceLock::new();

// OCR model paths (PaddleOCR)
pub static OCR_DET_MODEL_PATH: OnceLock<String> = OnceLock::new();
pub static OCR_CLS_MODEL_PATH: OnceLock<String> = OnceLock::new();
pub static OCR_REC_MODEL_PATH: OnceLock<String> = OnceLock::new();
pub static OCR_DICT_PATH: OnceLock<String> = OnceLock::new();

// Extracted images storage path
pub static EXTRACTED_IMAGES_PATH: OnceLock<String> = OnceLock::new();

pub static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub const CONFIG_NAME_CLIENT_ID: &'static str = "client_id";
pub const CONFIG_NAME_PROXY: &'static str = "proxy";
pub const CONFIG_NAME_INDEXER_SETTING: &'static str = "indexer_setting";
pub const CONFIG_NAME_WATCHER_SETTING: &'static str = "fs_watcher_setting";
pub const CONFIG_NAME_ACTIVE_LOCALE: &'static str = "active_locale";
pub const CONFIG_NAME_DB_VERSION: &'static str = "db_version";
pub const CONFIG_NAME_PATHS_CACHE_VERSION: &'static str = "paths_cache_version";
pub const CONFIG_NAME_ACTIVE_SELF_HOSTED_PLATFORM: &'static str = "active_self_hosted_platform";

pub static APP_DATA_PATH: RwLock<String> = RwLock::new(String::new());
pub static ONNX_EXEC_PROVIDERS_INITIALIZED: OnceLock<bool> = OnceLock::new();
pub static HOME_PATH: OnceLock<String> = OnceLock::new();
pub static STORAGE_PATH: OnceLock<String> = OnceLock::new();
pub static DB_PATH: OnceLock<String> = OnceLock::new();
pub static TMP_PATH: OnceLock<String> = OnceLock::new();
pub static EXIT_APP_SIGNAL: AtomicBool = AtomicBool::new(false);
pub static ACTIVE_MODEL_PLATFORM: LazyLock<RwLock<ModelPlatform>> =
    LazyLock::new(|| RwLock::new(ModelPlatform::default()));
pub static ACTIVE_SELF_HOSTED_PLATFORM: LazyLock<RwLock<SelfHostedPlatform>> =
    LazyLock::new(|| RwLock::new(SelfHostedPlatform::default()));

pub static PROXY: LazyLock<AsyncRwLock<ProxyInfo>> = LazyLock::new(|| {
    AsyncRwLock::new(ProxyInfo {
        protocol: "".to_string(),
        host: "".to_string(),
        port: 0,
    })
});
pub const DEFAULT_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub const DEFAULT_DATETIME_MICRO_FORMAT: &str = "%Y-%m-%d %H:%M:%S.%f";

pub static CLIENT_ID: RwLock<String> = RwLock::new(String::new());
// Current locale, default is en-US
pub static ACTIVE_LOCALE: LazyLock<RwLock<String>> =
    LazyLock::new(|| RwLock::new("en-US".to_string()));

pub static DOWNLOADING: AtomicBool = AtomicBool::new(false);

// Scanning related
pub static SCANNING: AtomicBool = AtomicBool::new(false);
pub static SCANNING_TOTAL: AtomicUsize = AtomicUsize::new(0);

macro_rules! define_document_exts {
    ($($name:ident: [$($ext:literal),*];)*) => {
        $(
            pub const $name: &[&str] = &[$($ext),*];
        )*
        pub const SUPPORTED_DOCS_EXTS: &[&str] = &[$($($ext),*),*];
    }
}
define_document_exts! {
    DOCX_EXTS: ["docx"];
    EXCEL_EXTS: ["xlsx", "xls", "xlsm", "xlsb", "xla", "xlam"];
    ODS_EXTS: ["ods"];
    ODP_EXTS: ["odp"];
    ODT_EXTS: ["odt"];
    PDF_EXTS: ["pdf"];
    PPTX_EXTS: ["pptx"];
    PLAIN_TEXT_EXTS: ["txt", "log", "md", "mdx", "ini", "toml", "yaml", "yml"];
    ANYTOMD_EXTRA_EXTS: ["html", "htm", "ipynb", "json", "xml", "csv"];
}
pub const SUPPORTED_IMAGE_EXTS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "webp"];
pub const SUPPORTED_AUDIO_EXTS: [&str; 8] =
    ["mp3", "wav", "aac", "flac", "ogg", "m4a", "wma", "amr"];
pub const SUPPORTED_VIDEO_EXTS: [&str; 4] = ["mp4", "avi", "mov", "mkv"];

// Document related
type DocHandler = Arc<dyn DocumentLoader + Send + Sync>;
pub static EXT_TO_DOC_LOADER: LazyLock<AsyncRwLock<HashMap<String, DocHandler>>> =
    LazyLock::new(|| {
        let loaders: Vec<Arc<dyn DocumentLoader + Send + Sync>> = vec![
            Arc::new(AnyToMdLoader::default()) as DocHandler,
            Arc::new(OdpLoader::default()) as DocHandler,
            Arc::new(OdsLoader::default()) as DocHandler,
            Arc::new(OdtLoader::default()) as DocHandler,
            Arc::new(PdfPlumberLoader::default()) as DocHandler,
        ];
        // key: extension, value: document loader
        let mut ext_to_loader = HashMap::new();
        for loader in loaders {
            for ext in loader.get_exts() {
                ext_to_loader.insert(ext.clone(), Arc::clone(&loader));
            }
        }
        AsyncRwLock::new(ext_to_loader)
    });

// Chunking related
pub const DOCUMENT_CHUNK_SIZE: usize = 768;
pub const DOCUMENT_CHUNK_OVERLAP: usize = 20;
pub const MAX_DOCUMENT_LOAD_CHARS: usize = 200_000;

// Indexing related
pub static INDEXER_SETTING: LazyLock<AsyncRwLock<IndexerSetting>> =
    LazyLock::new(|| AsyncRwLock::new(IndexerSetting::default()));
pub static INDEXING: AtomicBool = AtomicBool::new(false);
pub static CONTENT_STORAGE_CHANGING: AtomicBool = AtomicBool::new(false);
pub static STOP_INDEX_SIGNAL: AtomicBool = AtomicBool::new(false);
pub static FS_WATCHER_SETTING: LazyLock<AsyncRwLock<FsWatcherSetting>> =
    LazyLock::new(|| AsyncRwLock::new(FsWatcherSetting::default()));

// Running indexing task summary
pub static INDEXING_SUMMARY: LazyLock<AsyncRwLock<IndexingSummary>> =
    LazyLock::new(|| AsyncRwLock::new(IndexingSummary::default()));

pub static INCREMENT_WATCH_PATHS: LazyLock<AsyncRwLock<HashSet<String>>> =
    LazyLock::new(|| AsyncRwLock::new(HashSet::new()));

// Ignore dot-prefixed directories, such as .git, .vscode, etc.
pub const IGNORE_HIDDEN_DIRS: bool = true;
// Ignore dot-prefixed files, such as .gitignore, .env, etc.
pub const IGNORE_HIDDEN_FILES: bool = true;

// For user defined document types pub async fn supported_doc_exts() -> Vec<String> {
//     let guard = EXT_TO_DOC_LOADER.read().await;
//     guard.keys().cloned().collect()
// }

pub const TRAY_ID: &'static str = "main";
pub static UI_MOUNTED: AtomicBool = AtomicBool::new(false);

/// Test mode for simulating remote devices
/// When enabled, remote device search returns mock data instead of making real network requests
///
/// Usage:
/// - `true`: Mock devices are added to device list, returns simulated search/similar results
/// - `false` (default): Real network requests to paired devices only
///
/// To toggle from frontend console:
/// - Enable:  `await window.__TAURI__.invoke('toggle_test_mode', { enabled: true })`
/// - Disable: `await window.__TAURI__.invoke('toggle_test_mode', { enabled: false })`
/// - Check:   `await window.__TAURI__.invoke('get_test_mode')`
///
pub static TEST_MODE_REMOTE_DEVICE: AtomicBool = AtomicBool::new(false);

/// Path-search cache entries as (file_info.id, path): the real DB id travels with the path so search results never fabricate ids from line numbers
/// 路径搜索缓存元素为 (file_info.id, path):真实 id 随路径入缓存,搜索结果不再用行号伪造 id
pub static PATHS_CACHE: LazyLock<AsyncRwLock<Vec<(i64, String)>>> =
    LazyLock::new(|| AsyncRwLock::new(vec![]));
pub static PATHS_CACHE_BUILD_TIME: LazyLock<AsyncRwLock<DateTime<Local>>> =
    LazyLock::new(|| AsyncRwLock::new(Local::now()));

pub const EVENT_SELECTOR_INDEXING: &'static str = "selector-indexing";
pub const EVENT_WATCHER_INDEXING: &'static str = "watcher-indexing";

pub const INDEXING_FROM_SELECTOR: &'static str = "selector";
pub const INDEXING_FROM_WATCHER: &'static str = "watcher";

pub const SHORT_QUERY_LEN: usize = 8;
pub const QUERY_MIN_SCORE: usize = 50;
