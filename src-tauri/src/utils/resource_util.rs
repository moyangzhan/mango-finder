use crate::global::CONFIG_NAME_RESOURCE_SNAPSHOT;
use crate::utils::app_util;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Availability of the three local model backends. The full file set of a
/// backend must exist for its option to be selectable (a half-present backend
/// would only explode mid-index) - one detection function, shared by the
/// settings-page greying, the preset fallback and the first-run default, so
/// the three can never drift apart.
///
/// 三个本地模型后端的可用性。后端所需模型文件全集存在才可选
/// (半残后端会索引到一半才炸)—— 单一检测函数,设置页灰亮、预设回落、
/// 首次默认三处共用,口径不漂移。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LocalBackendStatus {
    pub vector_missing: Vec<String>,
    pub vision_missing: Vec<String>,
    pub audio_missing: Vec<String>,
}

impl LocalBackendStatus {
    pub fn vector_available(&self) -> bool {
        self.vector_missing.is_empty()
    }
    pub fn vision_available(&self) -> bool {
        self.vision_missing.is_empty()
    }
    pub fn audio_available(&self) -> bool {
        self.audio_missing.is_empty()
    }
    pub fn all_available(&self) -> bool {
        self.vector_available() && self.vision_available() && self.audio_available()
    }
}

fn missing(files: &[(&str, String)]) -> Vec<String> {
    files
        .iter()
        .filter(|(_, path)| !std::path::Path::new(path).exists())
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Single detection source for all three categories.
pub fn check_local_backends() -> LocalBackendStatus {
    LocalBackendStatus {
        vector_missing: missing(&[
            (
                crate::global::EMBEDDING_MODEL_NAME,
                app_util::get_multilingual_embedding_path(),
            ),
            (
                crate::global::EMBEDDING_TOKENIZER_NAME,
                app_util::get_multilingual_tokenizer_path(),
            ),
        ]),
        vision_missing: missing(&[
            ("vision.onnx", app_util::get_vision_0_path()),
            ("vision_tokenizer.json", app_util::get_vision_tokenizer_path()),
        ]),
        audio_missing: missing(&[(
            crate::global::WHISPER_MODEL_NAME,
            app_util::get_whisper_model_path(),
        )]),
    }
}

/// All bundled model resource files (existence snapshot keys). Files live in
/// one directory, so they usually vanish as a group - install damage /
/// antivirus quarantine / manual deletion.
const RESOURCE_FILES: [(&str, fn() -> String); 9] = [
    (
        crate::global::EMBEDDING_MODEL_NAME,
        app_util::get_multilingual_embedding_path,
    ),
    (
        crate::global::EMBEDDING_TOKENIZER_NAME,
        app_util::get_multilingual_tokenizer_path,
    ),
    ("vision.onnx", app_util::get_vision_0_path),
    ("vision_tokenizer.json", app_util::get_vision_tokenizer_path),
    (crate::global::WHISPER_MODEL_NAME, app_util::get_whisper_model_path),
    ("ch_PP-OCRv4_det_infer.onnx", app_util::get_ocr_det_model_path),
    (
        "ch_ppocr_mobile_v2.0_cls_infer.onnx",
        app_util::get_ocr_cls_model_path,
    ),
    ("ch_PP-OCRv4_rec_infer.onnx", app_util::get_ocr_rec_model_path),
    ("ppocr_keys_v1.txt", app_util::get_ocr_dict_path),
];

/// Current existence snapshot: file name → exists.
pub fn resource_snapshot() -> BTreeMap<String, bool> {
    RESOURCE_FILES
        .iter()
        .map(|(name, path_fn)| (name.to_string(), std::path::Path::new(&path_fn()).exists()))
        .collect()
}

/// Resource files that existed in the old snapshot but are gone now
/// ("had them → lost them" is the anomaly worth a startup diagnostic).
pub fn diff_vanished(old: &BTreeMap<String, bool>, new: &BTreeMap<String, bool>) -> Vec<String> {
    old.iter()
        .filter(|(name, existed)| **existed && new.get(*name) == Some(&false))
        .map(|(name, _)| name.clone())
        .collect()
}

/// Snapshot config key scoped to the installed edition. Each edition has its
/// own data directory by default, but a `.config` override can still point
/// both editions at ONE directory — the baseline stays edition-scoped so one
/// edition's file state never reads as the other's anomaly.
///
/// 按已安装版本划分的快照 config key。默认各版本数据目录独立,但 .config 覆盖
/// 可让两个版本指向同一目录——基线仍按版本划分,一个版本的文件状况不会误报到
/// 另一个版本头上。
fn snapshot_key(edition: &str) -> String {
    let slug: String = edition
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}::{slug}", CONFIG_NAME_RESOURCE_SNAPSHOT)
}

/// Startup resource check: diff the current snapshot against the one stored at
/// the previous startup, then persist the new snapshot. Returns the vanished
/// file list for the diagnostic modal (empty = healthy or first run).
/// `edition` (the app identifier) scopes the baseline to the installed edition.
///
/// 启动资源检测:当前快照与上次启动存储的快照 diff,然后持久化新快照。
/// 返回消失文件清单供诊断模态使用(空 = 健康或首次运行)。
/// `edition`(应用 identifier)将基线限定在本安装版本内。
pub fn startup_resource_check(edition: &str) -> Vec<String> {
    let key = snapshot_key(edition);
    let current = resource_snapshot();
    let stored = crate::repositories::config_repo::get_val(&key);
    let vanished = if stored.is_empty() {
        Vec::new() // first run of this edition: no baseline, nothing to report
    } else {
        match serde_json::from_str::<BTreeMap<String, bool>>(&stored) {
            Ok(old) => diff_vanished(&old, &current),
            Err(_) => Vec::new(),
        }
    };
    if let Ok(json) = serde_json::to_string(&current) {
        let _ = crate::repositories::config_repo::upsert(&key, &json);
    }
    vanished
}

/// Category impact of a vanished-file list (for the diagnostic modal copy).
pub fn affected_categories(vanished: &[String]) -> Vec<String> {
    let mut affected = Vec::new();
    let vector_files = [
        crate::global::EMBEDDING_MODEL_NAME,
        crate::global::EMBEDDING_TOKENIZER_NAME,
    ];
    if vanished.iter().any(|f| vector_files.contains(&f.as_str())) {
        affected.push("vector".to_string());
    }
    if vanished
        .iter()
        .any(|f| f == "vision.onnx" || f == "vision_tokenizer.json")
    {
        affected.push("vision".to_string());
    }
    if vanished
        .iter()
        .any(|f| f == crate::global::WHISPER_MODEL_NAME)
    {
        affected.push("audio".to_string());
    }
    if vanished
        .iter()
        .any(|f| f.to_lowercase().contains("ppocr") || f.to_lowercase().contains("ocr"))
    {
        affected.push("ocr".to_string());
    }
    affected
}

/// Serialize helper for commands
pub fn snapshot_json() -> Value {
    serde_json::to_value(resource_snapshot()).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_only_vanished() {
        let old: BTreeMap<String, bool> = [
            ("a".to_string(), true), ("b".to_string(), true), ("c".to_string(), false),
        ]
        .into_iter()
        .collect();
        let new: BTreeMap<String, bool> = [
            ("a".to_string(), false), ("b".to_string(), true), ("c".to_string(), true),
        ]
        .into_iter()
        .collect();
        assert_eq!(diff_vanished(&old, &new), vec!["a".to_string()]);
    }

    #[test]
    fn category_impact_maps_files() {
        let impacted = affected_categories(&[
            "embedding.onnx".to_string(),
            "ch_PP-OCRv4_rec_infer.onnx".to_string(),
        ]);
        assert!(impacted.contains(&"vector".to_string()));
        assert!(impacted.contains(&"ocr".to_string()));
        assert!(!impacted.contains(&"audio".to_string()));
    }
}
