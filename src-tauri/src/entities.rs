use crate::enums::{FileIndexStatus, IndexingTaskStatus, OnlineStatus, PairingRequestStatus};
use crate::structs::file_metadata::FileMetadata;
use crate::structs::sparse_vector::SparseVector;
use crate::utils::datetime_util;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub id: i64,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SelfHostedPlatform {
    pub id: i64,
    pub name: String,
    pub title: String,
    pub host: String,
    pub port: i32,
    pub remark: String,
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

impl Default for SelfHostedPlatform {
    fn default() -> Self {
        Self {
            id: 0,
            name: "".to_string(),
            title: "".to_string(),
            host: "127.0.0.1".to_string(),
            port: 11434,
            remark: "".to_string(),
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModelPlatform {
    pub id: i64,
    pub name: String,
    pub title: String,
    pub logo: String,
    pub base_url: String,
    pub api_key: String,
    pub remark: String,
    pub is_proxy_enable: bool,
    pub is_openai_api_compatible: bool,
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

impl Default for ModelPlatform {
    fn default() -> Self {
        Self {
            id: 0,
            name: "".to_string(),
            title: "".to_string(),
            logo: "".to_string(),
            base_url: "".to_string(),
            api_key: "".to_string(),
            remark: "".to_string(),
            is_proxy_enable: false,
            is_openai_api_compatible: false,
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }
}

impl ModelPlatform {
    pub fn is_enable(&self) -> bool {
        !self.api_key.is_empty()
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AiModel {
    pub id: i64,
    pub name: String,
    pub title: String,
    pub remark: String,
    pub model_types: String,
    pub setting: String,
    pub platform: String,
    pub context_window: i32,
    pub max_input_tokens: i32,
    pub max_output_tokens: i32,
    pub input_types: String,
    pub properties: String,
    pub is_reasoner: bool,
    pub is_thinking_closable: bool,
    pub is_free: bool,
    pub is_enable: bool,
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FileInfo {
    pub id: i64,
    pub name: String,
    pub category: i64,
    pub path: String,
    pub content: String,
    /// Relative path to the parsed content file (e.g. `parsed_documents/{id}_{name}.md`).
    /// When `Some`, content is stored on disk and `content` is empty.
    /// When `None`, content (if any) is in the `content` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_ref_path: Option<String>,
    pub metadata: FileMetadata,
    pub file_ext: String,
    pub file_size: i64,
    pub md5: String,
    pub content_index_status: i64,
    pub content_index_status_msg: String,
    pub meta_index_status: i64,
    pub meta_index_status_msg: String,
    pub is_invalid: bool,
    pub invalid_reason: String,
    /// Audio type classification (only for audio category files)
    /// Values: 0=Unknown, 1=Speech, 2=Music, 3=Mixed
    pub audio_type: i32,
    /// Perceptual hash for image similarity (only for image category files)
    /// 8 bytes for 8x8 gradient hash
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_hash: Option<Vec<u8>>,
    /// Audio fingerprint for music similarity (only for audio category files with music type)
    /// Contains spectral_histogram (10 f32) + energy_bands (8 f32) + avg_zcr (f32) + tempo_estimate (f32)
    /// Total: 20 f32 values = 80 bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_fingerprint: Option<Vec<u8>>,
    #[serde(with = "datetime_util")]
    pub file_create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub file_update_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

impl Default for FileInfo {
    fn default() -> Self {
        Self {
            id: 0,
            name: "".to_string(),
            category: 0,
            path: "".to_string(),
            content: "".to_string(),
            content_ref_path: None,
            metadata: FileMetadata::default(),
            file_ext: "".to_string(),
            file_size: 0,
            file_create_time: DateTime::default(),
            file_update_time: DateTime::default(),
            md5: "".to_string(),
            is_invalid: false,
            invalid_reason: "".to_string(),
            audio_type: 0, // 0=Unknown (default for non-audio files and unindexed audio)
            image_hash: None, // No hash by default (only for image files)
            audio_fingerprint: None, // No fingerprint by default (only for music files)
            content_index_status: FileIndexStatus::Waiting.value(),
            content_index_status_msg: "".to_string(),
            meta_index_status: FileIndexStatus::Waiting.value(),
            meta_index_status_msg: "".to_string(),
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FileMetaEmbedding {
    pub id: i64,
    pub file_id: i64,
    #[serde(skip, default = "default_embedding")]
    pub embedding: [f32; 256],
    pub sparse_vec: SparseVector,

    pub distance: f32,     // for search result
    pub sparse_score: f32, // for search result
    pub score: usize,      // for search result, weighted score of distance and sparse_score
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FileContentEmbedding {
    pub id: i64,
    pub file_id: i64,
    pub chunk_index: i64,
    pub chunk_text: String,
    #[serde(skip, default = "default_embedding_1024")]
    pub embedding: [f32; 1024],
    pub sparse_vec: SparseVector,

    pub distance: f32,     // for search result
    pub sparse_score: f32, // for search result
    pub score: usize,      // for search result, weighted score of distance and sparse_score
}

impl Default for FileContentEmbedding {
    fn default() -> Self {
        Self {
            id: 0,
            file_id: 0,
            embedding: default_embedding_1024(),
            chunk_index: 0,
            chunk_text: "".to_string(),
            sparse_vec: SparseVector::default(),

            distance: -0.1,
            sparse_score: 0.0,
            score: 0,
        }
    }
}

fn default_embedding() -> [f32; 256] {
    [0.0; 256]
}

fn default_embedding_1024() -> [f32; 1024] {
    [0.0; 1024]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingTask {
    pub id: i64,
    pub paths: String,
    pub embedding_model: String,
    pub status: IndexingTaskStatus,
    pub start_time: Option<DateTime<Local>>,
    pub end_time: Option<DateTime<Local>>,
    pub duration: i64, // milliseconds
    pub total_cnt: i64,
    pub content_processed_cnt: i64,
    pub content_indexed_success_cnt: i64,
    pub content_indexed_failed_cnt: i64,
    pub content_indexed_skipped_cnt: i64,
    pub remark: String,
    pub config_json: String,
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

impl Default for IndexingTask {
    fn default() -> Self {
        Self {
            id: 0,
            paths: "".to_string(),
            embedding_model: "".to_string(),
            status: IndexingTaskStatus::Pending,
            start_time: None,
            end_time: None,
            duration: 0,
            total_cnt: 0,
            content_processed_cnt: 0,
            content_indexed_success_cnt: 0,
            content_indexed_failed_cnt: 0,
            content_indexed_skipped_cnt: 0,
            remark: "".to_string(),
            config_json: "".to_string(),
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtsSearchResult {
    pub file_id: i64,
    pub chunk_ids: HashSet<i64>,
    pub matched_keywords: HashSet<String>,
    pub score: usize, // 0 - 100
}

/// Remote device entity
/// 远程设备实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: i64,
    /// Remote device's unique ID (UUID)
    pub device_id: String,
    /// Device display name
    pub name: String,
    /// IP address
    pub ip_address: String,
    /// Service port
    pub port: i32,
    /// Remote software version
    pub version: String,
    /// Online status: online, offline, unknown
    pub online_status: OnlineStatus,
    /// Pairing status: none, pending_in, pending_out, paired, rejected
    pub pairing_status: crate::enums::PairingStatus,
    /// Remark explaining the pairing status change
    /// 配对状态变化的说明
    pub pairing_remark: String,
    /// Last seen timestamp
    #[serde(with = "datetime_util")]
    pub last_seen: DateTime<Local>,
    /// First discovered timestamp
    #[serde(with = "datetime_util")]
    pub first_discovered: DateTime<Local>,
    /// Number of indexed files on remote device
    pub index_count: i64,
    /// JSON: supported search types
    pub capabilities: String,
    /// Discovery method: mdns, manual
    pub discovery_method: String,
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            id: 0,
            device_id: "".to_string(),
            name: "".to_string(),
            ip_address: "".to_string(),
            port: 7890,
            version: "".to_string(),
            online_status: OnlineStatus::Unknown,
            pairing_status: crate::enums::PairingStatus::None,
            pairing_remark: "".to_string(),
            last_seen: Local::now(),
            first_discovered: Local::now(),
            index_count: 0,
            capabilities: "{}".to_string(),
            discovery_method: "mdns".to_string(),
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }
}

/// Pairing request log entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub id: i64,
    /// Remote device ID
    pub device_id: String,
    /// Remote device name
    pub device_name: String,
    /// Remote IP address
    pub ip_address: String,
    /// Remote service port
    pub port: i32,
    /// Direction: "in" (received) or "out" (sent)
    pub direction: String,
    /// Request status
    pub status: PairingRequestStatus,
    /// Remark describing the handling result
    pub remark: String,
    /// Time when responded
    #[serde(with = "datetime_util::option")]
    pub response_time: Option<DateTime<Local>>,
    /// Request creation time
    #[serde(with = "datetime_util")]
    pub create_time: DateTime<Local>,
    /// Record update time
    #[serde(with = "datetime_util")]
    pub update_time: DateTime<Local>,
}

impl Default for PairingRequest {
    fn default() -> Self {
        Self {
            id: 0,
            device_id: "".to_string(),
            device_name: "".to_string(),
            ip_address: "".to_string(),
            port: 7890,
            direction: "in".to_string(),
            status: PairingRequestStatus::Pending,
            remark: "".to_string(),
            response_time: None,
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }
}
