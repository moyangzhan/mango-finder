use crate::utils::datetime_util;
use crate::utils::file_util::get_file_attributes_desc;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// Audio type classification for audio files
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum AudioType {
    #[default]
    Unknown = 0,
    Speech = 1, // Podcasts, audiobooks, voice recordings - use transcription text similarity
    Music = 2,  // Songs, instrumental - need audio fingerprint
    Mixed = 3,  // Music with vocals
}

impl From<i32> for AudioType {
    fn from(value: i32) -> Self {
        match value {
            1 => AudioType::Speech,
            2 => AudioType::Music,
            3 => AudioType::Mixed,
            _ => AudioType::Unknown,
        }
    }
}

impl From<AudioType> for i32 {
    fn from(value: AudioType) -> Self {
        value as i32
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FileMetadata {
    pub name: String,
    pub extension: String,
    pub category: String,
    pub attributes: u32,
    pub size: i64, //in bytes
    #[serde(with = "datetime_util")]
    pub created: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub modified: DateTime<Local>,
    pub author: String,
    /// Audio type classification (only applicable for audio category files)
    #[serde(default)]
    pub audio_type: Option<i32>,
}

impl FileMetadata {
    pub fn default() -> Self {
        Self {
            name: String::new(),
            extension: String::new(),
            category: String::new(),
            attributes: 0,
            size: 0,
            created: DateTime::default(),
            modified: DateTime::default(),
            author: String::new(),
            audio_type: None,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).unwrap_or("{}".to_string())
    }

    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or(Self::default())
    }

    pub fn to_text(&self) -> String {
        let attribute = get_file_attributes_desc(self.attributes);
        return format_args!(
            r#"file name:{},file extension:{},file category:{},size:{} bytes,creation time:{},last write time:{},author:{},file attributes:{}"#,
            self.name,
            self.extension,
            self.category,
            self.size,
            datetime_util::datetime_to_str(&self.created),
            datetime_util::datetime_to_str(&self.modified),
            self.author,
            attribute.join(", ")
        ).to_string();
    }
}
