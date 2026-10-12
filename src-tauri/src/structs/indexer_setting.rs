use crate::enums::{FileCategory, FileParserMode};
use serde::{Deserialize, Deserializer, Serialize};

/// Per-content-type storage setting: "database" | "file" | "none"
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct ContentStorage {
    #[serde(default = "default_storage")]
    pub document: String,
    #[serde(default = "default_storage")]
    pub image: String,
    #[serde(default = "default_storage")]
    pub audio: String,
}

impl Default for ContentStorage {
    fn default() -> Self {
        Self {
            document: default_storage(),
            image: default_storage(),
            audio: default_storage(),
        }
    }
}

impl ContentStorage {
    pub fn get_for_category(&self, category: &FileCategory) -> &str {
        match category {
            FileCategory::Document => &self.document,
            FileCategory::Image => &self.image,
            FileCategory::Audio => &self.audio,
            _ => &self.document,
        }
    }

    pub fn set_for_category(&mut self, category: &str, value: String) {
        match category {
            "document" => self.document = value,
            "image" => self.image = value,
            "audio" => self.audio = value,
            _ => {}
        }
    }
}

fn default_storage() -> String {
    "database".to_string()
}

/// Old boolean-based format for backward compatibility with v0.11.0
#[derive(Deserialize)]
struct OldSaveParsedContent {
    #[serde(default = "default_true")]
    document: bool,
    #[serde(default = "default_true")]
    image: bool,
    #[serde(default = "default_true")]
    audio: bool,
}

fn default_true() -> bool {
    true
}

impl From<OldSaveParsedContent> for ContentStorage {
    fn from(_old: OldSaveParsedContent) -> Self {
        // Old format only supported boolean for document; image/audio were always database
        Self {
            document: if _old.document { "database" } else { "none" }.to_string(),
            image: "database".to_string(),
            audio: "database".to_string(),
        }
    }
}

/// Parse ContentStorage from a JSON value, handling both old boolean and new string formats
fn parse_content_storage(value: &serde_json::Value) -> ContentStorage {
    let Some(obj) = value.as_object() else {
        return ContentStorage::default();
    };
    let first_value = obj.values().next();
    // New format: values are strings like "database"
    if first_value.map_or(false, |v| v.is_string()) {
        serde_json::from_value(value.clone()).unwrap_or_default()
    } else if first_value.map_or(false, |v| v.is_boolean()) {
        let old: OldSaveParsedContent =
            serde_json::from_value(value.clone()).unwrap_or_else(|_| OldSaveParsedContent {
                document: true,
                image: true,
                audio: true,
            });
        ContentStorage::from(old)
    } else {
        ContentStorage::default()
    }
}

#[derive(Debug, Clone)]
pub struct IndexerSetting {
    pub is_private: bool,    // Deprecated: kept for backward compatibility
    pub parser_mode: String, // "local" | "selfhosted" | "remote" | "mixed" - quick preset
    pub image_parser_mode: FileParserMode,
    pub audio_parser_mode: FileParserMode,
    pub ignore_dirs: Vec<String>,
    pub ignore_path_prefixes: Vec<String>, // Full path prefixes to ignore
    pub ignore_exts: Vec<String>,
    pub ignore_files: Vec<String>, // File absolute path
    pub content_storage: ContentStorage,
}

impl Serialize for IndexerSetting {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("IndexerSetting", 9)?;
        state.serialize_field("is_private", &self.is_private)?;
        state.serialize_field("parser_mode", &self.parser_mode)?;
        state.serialize_field("image_parser_mode", &self.image_parser_mode)?;
        state.serialize_field("audio_parser_mode", &self.audio_parser_mode)?;
        state.serialize_field("ignore_dirs", &self.ignore_dirs)?;
        state.serialize_field("ignore_path_prefixes", &self.ignore_path_prefixes)?;
        state.serialize_field("ignore_exts", &self.ignore_exts)?;
        state.serialize_field("ignore_files", &self.ignore_files)?;
        state.serialize_field("content_storage", &self.content_storage)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for IndexerSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| D::Error::custom("Expected a JSON object for IndexerSetting"))?;

        // Resolve content_storage: accept both new "content_storage" and legacy "save_parsed_content"
        let content_storage = if let Some(cs_value) = obj.get("content_storage") {
            parse_content_storage(cs_value)
        } else if let Some(spc_value) = obj.get("save_parsed_content") {
            let old: OldSaveParsedContent =
                serde_json::from_value(spc_value.clone()).map_err(D::Error::custom)?;
            ContentStorage::from(old)
        } else {
            ContentStorage::default()
        };

        Ok(IndexerSetting {
            is_private: obj
                .get("is_private")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            parser_mode: obj
                .get("parser_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("local")
                .to_string(),
            image_parser_mode: obj
                .get("image_parser_mode")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            audio_parser_mode: obj
                .get("audio_parser_mode")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            ignore_dirs: obj
                .get("ignore_dirs")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            ignore_path_prefixes: obj
                .get("ignore_path_prefixes")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            ignore_exts: obj
                .get("ignore_exts")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            ignore_files: obj
                .get("ignore_files")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            content_storage,
        })
    }
}

impl Default for IndexerSetting {
    fn default() -> Self {
        Self {
            is_private: true,
            parser_mode: "local".to_string(),
            image_parser_mode: FileParserMode::Local,
            audio_parser_mode: FileParserMode::Local,
            ignore_exts: vec![],
            ignore_files: vec![],
            ignore_dirs: vec![],
            ignore_path_prefixes: vec![],
            content_storage: ContentStorage::default(),
        }
    }
}
