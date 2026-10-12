use crate::global::{
    SUPPORTED_AUDIO_EXTS, SUPPORTED_DOCS_EXTS, SUPPORTED_IMAGE_EXTS, SUPPORTED_VIDEO_EXTS,
};
use rusqlite::Result as SqlResult;
use rusqlite::types::{FromSql, FromSqlError, ToSql, ToSqlOutput};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    EnUs,
    ZhCn,
}

impl Locale {
    pub fn from(value: &str) -> Self {
        match value {
            "en-US" => Locale::EnUs,
            "zh-CN" => Locale::ZhCn,
            _ => Locale::EnUs,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Locale::EnUs => "en-US",
            Locale::ZhCn => "zh-CN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPlatformName {
    OpenAi,
    SiliconFlow,
    DashScope,
    DeepSeek,
    OpenAiCompatible,
}

impl ModelPlatformName {
    pub fn from(value: &str) -> Self {
        match value {
            "openai" => ModelPlatformName::OpenAi,
            "siliconflow" => ModelPlatformName::SiliconFlow,
            "dashscope" => ModelPlatformName::DashScope,
            "deepseek" => ModelPlatformName::DeepSeek,
            "openai_compatible" => ModelPlatformName::OpenAiCompatible,
            _ => ModelPlatformName::OpenAi,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            ModelPlatformName::OpenAi => "openai",
            ModelPlatformName::SiliconFlow => "siliconflow",
            ModelPlatformName::DashScope => "dashscope",
            ModelPlatformName::DeepSeek => "deepseek",
            ModelPlatformName::OpenAiCompatible => "openai_compatible",
        }
    }
}

// Model type: text, image, vision, embedding, rerank, asr, tts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelType {
    Text,
    Image,
    Vision,
    Embedding,
    Rerank,
    Asr,
    Tts,
}

impl From<&str> for ModelType {
    fn from(s: &str) -> Self {
        match s {
            "text" => ModelType::Text,
            "image" => ModelType::Image,
            "vision" => ModelType::Vision,
            "embedding" => ModelType::Embedding,
            "rerank" => ModelType::Rerank,
            "asr" => ModelType::Asr,
            "tts" => ModelType::Tts,
            _ => ModelType::Text,
        }
    }
}

impl From<ModelType> for &'static str {
    fn from(s: ModelType) -> Self {
        match s {
            ModelType::Text => "text",
            ModelType::Image => "image",
            ModelType::Vision => "vision",
            ModelType::Embedding => "embedding",
            ModelType::Rerank => "rerank",
            ModelType::Asr => "asr",
            ModelType::Tts => "tts",
        }
    }
}

//File index status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIndexStatus {
    Waiting = 1,
    Indexing = 2,
    Indexed = 3,
    IndexFailed = 4,
}
impl FileIndexStatus {
    pub fn value(self) -> i64 {
        self as i64
    }
}

impl From<i64> for FileIndexStatus {
    fn from(value: i64) -> Self {
        match value {
            1 => FileIndexStatus::Waiting,
            2 => FileIndexStatus::Indexing,
            3 => FileIndexStatus::Indexed,
            4 => FileIndexStatus::IndexFailed,
            _ => FileIndexStatus::Waiting, // Default value
        }
    }
}

// File category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Document = 1,
    Image = 2,
    Audio = 3,
    Video = 4,
    Other = 5,
}

impl FileCategory {
    pub fn value(self) -> i64 {
        self as i64
    }

    pub fn from_ext(ext: &str) -> Self {
        if ext.is_empty() {
            return FileCategory::Other;
        }
        if SUPPORTED_DOCS_EXTS.contains(&ext) {
            FileCategory::Document
        } else if SUPPORTED_IMAGE_EXTS.contains(&ext) {
            FileCategory::Image
        } else if SUPPORTED_AUDIO_EXTS.contains(&ext) {
            FileCategory::Audio
        } else if SUPPORTED_VIDEO_EXTS.contains(&ext) {
            FileCategory::Video
        } else {
            FileCategory::Other
        }
    }

    pub fn to_text(self) -> &'static str {
        self.into()
    }

    pub fn value_to_text(value: i64) -> &'static str {
        FileCategory::from(value).into()
    }

    pub fn is_document(&self) -> bool {
        matches!(self, FileCategory::Document)
    }

    pub fn is_media(&self) -> bool {
        matches!(
            self,
            FileCategory::Image | FileCategory::Audio | FileCategory::Video
        )
    }
}

impl From<&str> for FileCategory {
    fn from(s: &str) -> Self {
        match s {
            "document" => FileCategory::Document,
            "image" => FileCategory::Image,
            "audio" => FileCategory::Audio,
            "video" => FileCategory::Video,
            _ => FileCategory::Other,
        }
    }
}

impl From<i64> for FileCategory {
    fn from(value: i64) -> Self {
        match value {
            1 => FileCategory::Document,
            2 => FileCategory::Image,
            3 => FileCategory::Audio,
            4 => FileCategory::Video,
            _ => FileCategory::Other, // Default value
        }
    }
}

impl From<FileCategory> for &'static str {
    fn from(value: FileCategory) -> Self {
        match value {
            FileCategory::Document => "document",
            FileCategory::Image => "image",
            FileCategory::Audio => "audio",
            FileCategory::Video => "video",
            FileCategory::Other => "other",
        }
    }
}

impl Display for FileCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = <&'static str>::from(*self);
        write!(f, "{}", text)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileContentLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh")]
    Chinese,
    Multilingual,
}

impl From<&str> for FileContentLanguage {
    fn from(s: &str) -> Self {
        match s {
            "en" | "english" => FileContentLanguage::English,
            "zh" | "chinese" => FileContentLanguage::Chinese,
            "multilingual" => FileContentLanguage::Multilingual,
            _ => FileContentLanguage::English,
        }
    }
}

impl From<FileContentLanguage> for &'static str {
    fn from(value: FileContentLanguage) -> Self {
        match value {
            FileContentLanguage::Chinese => "chinese",
            FileContentLanguage::English => "english",
            FileContentLanguage::Multilingual => "multilingual",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexingTaskStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl From<&str> for IndexingTaskStatus {
    fn from(s: &str) -> Self {
        match s {
            "pending" => IndexingTaskStatus::Pending,
            "running" => IndexingTaskStatus::Running,
            "paused" => IndexingTaskStatus::Paused,
            "completed" => IndexingTaskStatus::Completed,
            "failed" => IndexingTaskStatus::Failed,
            "cancelled" => IndexingTaskStatus::Cancelled,
            _ => IndexingTaskStatus::Pending,
        }
    }
}

impl From<IndexingTaskStatus> for &'static str {
    fn from(status: IndexingTaskStatus) -> Self {
        match status {
            IndexingTaskStatus::Pending => "pending",
            IndexingTaskStatus::Running => "running",
            IndexingTaskStatus::Paused => "paused",
            IndexingTaskStatus::Completed => "completed",
            IndexingTaskStatus::Failed => "failed",
            IndexingTaskStatus::Cancelled => "cancelled",
        }
    }
}

impl ToSql for IndexingTaskStatus {
    fn to_sql(&self) -> SqlResult<rusqlite::types::ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(<&'static str>::from(*self)))
    }
}

impl FromSql for IndexingTaskStatus {
    fn column_result(value: rusqlite::types::ValueRef) -> SqlResult<Self, FromSqlError> {
        let value = value.as_str()?;
        let status = IndexingTaskStatus::from(value);
        Ok(status)
    }
}

impl Display for IndexingTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = <&'static str>::from(*self);
        write!(f, "{}", text)
    }
}

// Communication events with front-end

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum DownloadEvent {
    Start { url: String, download_id: String },
    Progress { download_id: String, progress: f64 },
    Finish { download_id: String },
    Error { download_id: String, error: String },
}

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum IndexingEvent {
    Start { task_id: i64, msg: String },
    Scan { task_id: i64, msg: String },
    Stop { task_id: i64, msg: String },
    Embed { task_id: i64, msg: String },
    Finish { task_id: i64, msg: String },
}

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum ContentStorageChangeEvent {
    Start {
        category: String,
        total: i64,
    },
    Progress {
        category: String,
        current: i64,
        total: i64,
    },
    Complete {
        category: String,
        changed: i64,
        failed: i64,
    },
    Error {
        category: String,
        message: String,
    },
    Cancelled {
        category: String,
        changed: i64,
        failed: i64,
    },
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub enum CommandResultCode {
    ERROR,
    SUCCESS,
    INDEXING,
}

pub enum TrayMenuItem {
    Show,
    Quit,
}
impl Display for TrayMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrayMenuItem::Show => write!(f, "show"),
            TrayMenuItem::Quit => write!(f, "quit"),
        }
    }
}

impl From<TrayMenuItem> for &'static str {
    fn from(value: TrayMenuItem) -> Self {
        match value {
            TrayMenuItem::Show => "show",
            TrayMenuItem::Quit => "quit",
        }
    }
}

#[derive(Debug, Clone)]
pub enum FsEvent {
    Rename { from: PathBuf, to: PathBuf },
    Create(PathBuf),
    Remove { path: PathBuf, is_file: bool },
    Modify(PathBuf),
    Other,
}

impl FsEvent {
    pub fn paths(&self) -> Vec<&PathBuf> {
        match self {
            FsEvent::Rename { from, to } => vec![from, to],
            FsEvent::Create(path) => vec![path],
            FsEvent::Remove { path, is_file: _ } => vec![path],
            FsEvent::Modify(path) => vec![path],
            FsEvent::Other => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    File,
    Directory,
}

#[derive(Debug)]
pub enum QueryIntent {
    PathOnly,
    SemanticOnly,
    Hybrid,
}

/// Device online status
/// 设备在线状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnlineStatus {
    Online,
    Offline,
    Unknown,
}

impl OnlineStatus {
    pub fn from(value: &str) -> Self {
        match value {
            "online" => OnlineStatus::Online,
            "offline" => OnlineStatus::Offline,
            _ => OnlineStatus::Unknown,
        }
    }
}

impl From<OnlineStatus> for &'static str {
    fn from(status: OnlineStatus) -> Self {
        match status {
            OnlineStatus::Online => "online",
            OnlineStatus::Offline => "offline",
            OnlineStatus::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for OnlineStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", <&'static str>::from(*self))
    }
}

/// Device pairing status
/// 设备配对状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingStatus {
    /// No pairing relationship
    None,
    /// Received pairing request, waiting for local user to respond
    PendingIn,
    /// Sent pairing request, waiting for remote device to respond
    PendingOut,
    /// Successfully paired
    Paired,
    /// Local rejected remote's request (subsequent requests will be auto-rejected)
    /// 本机拒绝了对方的请求（后续请求将被自动拒绝）
    Rejected,
    /// Remote rejected local's request (blocked by remote)
    /// 对方拒绝了本机的请求（被对方拉黑）
    Blocked,
}

impl PairingStatus {
    pub fn from(value: &str) -> Self {
        match value {
            "none" => PairingStatus::None,
            "pending_in" => PairingStatus::PendingIn,
            "pending_out" => PairingStatus::PendingOut,
            "paired" => PairingStatus::Paired,
            "rejected" => PairingStatus::Rejected,
            "blocked" => PairingStatus::Blocked,
            _ => PairingStatus::None,
        }
    }

    /// Get priority level for status transition validation
    /// 获取状态转换验证的优先级
    pub fn priority(&self) -> u8 {
        match self {
            PairingStatus::None => 0,
            PairingStatus::PendingIn | PairingStatus::PendingOut => 1,
            PairingStatus::Paired => 2,
            PairingStatus::Rejected | PairingStatus::Blocked => 3,
        }
    }

    /// Check if this status can transition to another status
    /// 检查是否可以转换到另一个状态
    ///
    /// Rules:
    /// - Priority: None (0) → PendingIn/PendingOut (1) → Paired (2) → Rejected (3)
    /// - Higher priority cannot override lower priority (e.g., Paired cannot go back to Pending)
    /// - Lower priority can transition to higher priority (e.g., None → Pending → Paired)
    /// - Exception: Manual operation can reset ANY status to None
    /// 规则:
    /// - 优先级: None (0) → PendingIn/PendingOut (1) → Paired (2) → Rejected (3)
    /// - 高优先级不能覆盖低优先级（例如：Paired 不能回到 Pending）
    /// - 低优先级可以转换到高优先级（例如：None → Pending → Paired）
    /// - 例外: 手动操作可以将任意状态重置为 None
    pub fn can_transition_to(&self, new_status: &PairingStatus, is_manual: bool) -> bool {
        // Manual operations can reset ANY status to None
        // 手动操作可以将任意状态重置为 None
        if is_manual && *new_status == PairingStatus::None {
            return true;
        }

        // Allow same status (no-op)
        // 允许相同状态（无操作）
        if self == new_status {
            return true;
        }

        // Allow transition from lower priority to higher priority
        // 允许从低优先级转换到高优先级
        self.priority() <= new_status.priority()
    }
}

impl From<PairingStatus> for &'static str {
    fn from(status: PairingStatus) -> Self {
        match status {
            PairingStatus::None => "none",
            PairingStatus::PendingIn => "pending_in",
            PairingStatus::PendingOut => "pending_out",
            PairingStatus::Paired => "paired",
            PairingStatus::Rejected => "rejected",
            PairingStatus::Blocked => "blocked",
        }
    }
}

impl std::fmt::Display for PairingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", <&'static str>::from(*self))
    }
}

/// Connect request status
/// 连接请求状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingRequestStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
    AutoRejected,
    Cancelled,
}

impl PairingRequestStatus {
    pub fn from(value: &str) -> Self {
        match value {
            "pending" => PairingRequestStatus::Pending,
            "accepted" => PairingRequestStatus::Accepted,
            "rejected" => PairingRequestStatus::Rejected,
            "expired" => PairingRequestStatus::Expired,
            "auto_rejected" => PairingRequestStatus::AutoRejected,
            "cancelled" => PairingRequestStatus::Cancelled,
            _ => PairingRequestStatus::Expired,
        }
    }
}

impl From<PairingRequestStatus> for &'static str {
    fn from(status: PairingRequestStatus) -> Self {
        match status {
            PairingRequestStatus::Pending => "pending",
            PairingRequestStatus::Accepted => "accepted",
            PairingRequestStatus::Rejected => "rejected",
            PairingRequestStatus::Expired => "expired",
            PairingRequestStatus::AutoRejected => "auto_rejected",
            PairingRequestStatus::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for PairingRequestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", <&'static str>::from(*self))
    }
}

/// Pairing API response status
/// 配对 API 响应状态
///
/// Used in pairing request/response API responses
/// 用于配对请求/响应 API 的响应
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingResponseStatus {
    /// Request is approved
    /// 请求已批准
    Approved,
    /// Device is already paired
    /// 设备已配对
    AlreadyPaired,
    /// Request is pending for approval
    /// 请求等待批准
    Pending,
    /// Request is rejected
    /// 请求已拒绝
    Rejected,
}

impl PairingResponseStatus {
    pub fn from_str(value: &str) -> Self {
        match value {
            "approved" => PairingResponseStatus::Approved,
            "already_paired" => PairingResponseStatus::AlreadyPaired,
            "pending" => PairingResponseStatus::Pending,
            "rejected" => PairingResponseStatus::Rejected,
            _ => PairingResponseStatus::Pending,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PairingResponseStatus::Approved => "approved",
            PairingResponseStatus::AlreadyPaired => "already_paired",
            PairingResponseStatus::Pending => "pending",
            PairingResponseStatus::Rejected => "rejected",
        }
    }
}

impl From<PairingResponseStatus> for &'static str {
    fn from(status: PairingResponseStatus) -> Self {
        status.as_str()
    }
}

impl std::fmt::Display for PairingResponseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HitType {
    PathKeyword,
    ContentKeyword,
    ContentSemantic,
    MetaSemantic,
}

/// Types of similarity detection
/// 相似性检测类型
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SimilarityType {
    /// Perceptual hash similarity (for images)
    /// 感知哈希相似（用于图片）
    ImageHash,
    /// Semantic similarity using embeddings (for images)
    /// 语义相似（嵌入向量，用于图片）
    ImageSemantic,
    /// Semantic similarity using embeddings (for documents)
    /// 语义相似（嵌入向量，用于文档）
    DocumentSemantic,
    /// Audio fingerprint similarity (for music)
    /// 音频指纹相似（用于音乐）
    AudioFingerprint,
    /// Audio transcription similarity (for speech)
    /// 音频转写相似（用于语音）
    AudioTranscription,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileParserMode {
    Local,
    SelfHosted,
    Remote,
}

impl Default for FileParserMode {
    fn default() -> Self {
        FileParserMode::Local
    }
}
