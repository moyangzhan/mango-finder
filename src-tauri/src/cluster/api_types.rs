//! Shared API types for cluster communication
//!
//! This module defines the common request/response types used by both
//! HTTP server and HTTP client for inter-device communication.

use crate::enums::PairingResponseStatus;
use crate::structs::cluster_config::DeviceCapabilities;
use crate::structs::search_result::SearchResult;
use axum::response::{IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};

// ============================================================================ Error Codes
// ============================================================================

/// Error codes for cluster API responses
/// 集群 API 响应错误码
pub const CODE_SUCCESS: i32 = 0;
pub const CODE_MISSING_DEVICE_ID: i32 = 400;
pub const CODE_DEVICE_NOT_PAIRED: i32 = 403;
pub const CODE_NOT_FOUND: i32 = 404;
pub const CODE_INTERNAL_ERROR: i32 = 500;

// ============================================================================ Common Response
// ============================================================================

/// Common API response wrapper
/// 通用 API 响应包装
#[derive(Debug, Serialize, Deserialize)]
pub struct CommonResponse<T> {
    pub code: i32,
    pub data: Option<T>,
    #[serde(default)]
    pub msg: String,
}

impl<T: Serialize> CommonResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: CODE_SUCCESS,
            data: Some(data),
            msg: String::new(),
        }
    }

    pub fn error(code: i32, msg: &str) -> Self {
        Self {
            code,
            data: None,
            msg: msg.to_string(),
        }
    }
}

impl<T: Serialize + Clone> IntoResponse for CommonResponse<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

// ============================================================================ Ping API
// ============================================================================

/// Response data for GET /ping
/// GET /ping 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingData {
    pub device_id: String,
    pub device_name: String,
    pub status: String,
    pub version: String,
    pub index_count: i64,
    pub last_index_time: Option<String>,
    pub capabilities: DeviceCapabilities,
    pub timestamp: i64,
}

// ============================================================================ Local API /cli (local on-machine clients like the CLI)
// ============================================================================

/// POST /cli/search request
/// POST /cli/search 请求
#[derive(Debug, Deserialize)]
pub struct CliSearchRequest {
    pub query: String,
    /// "keyword" | "semantic" (default)
    pub search_type: Option<String>,
    pub limit: Option<usize>,
}

/// POST /cli/search response
/// POST /cli/search 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliSearchData {
    pub results: Vec<SearchResult>,
    /// Full match count before applying limit
    /// 应用 limit 前的完整匹配数
    pub total: usize,
}

/// POST /cli/similar/:file_id request
/// POST /cli/similar/:file_id 请求
#[derive(Debug, Deserialize)]
pub struct CliSimilarRequest {
    pub limit: Option<usize>,
}

/// POST /cli/similar/:file_id response
/// POST /cli/similar/:file_id 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliSimilarData {
    pub results: Vec<SearchResult>,
}

// ============================================================================ Search API
// ============================================================================

/// Response data for POST /search
/// POST /search 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchData {
    pub results: Vec<SearchResult>,
    #[serde(default)]
    pub total: usize,
    pub device_id: String,
    pub device_name: String,
}

// ============================================================================ Chunks API
// ============================================================================

/// Response data for POST /chunks
/// POST /chunks 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunksData {
    pub chunks: Vec<String>,
    pub device_id: String,
    pub device_name: String,
}

// ============================================================================ Find Similars API
// ============================================================================

/// Response data for POST /file/:file_id/cluster_similars
/// POST /file/:file_id/cluster_similars 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindSimilarsData {
    pub results: Vec<SearchResult>,
    #[serde(default)]
    pub total: usize,
    pub device_id: String,
    pub device_name: String,
}

// ============================================================================ File Content API
// ============================================================================

/// Response data for GET /file/:file_id/content
/// GET /file/:file_id/content 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContentData {
    pub content: String,
    pub device_id: String,
    pub device_name: String,
}

// ============================================================================ Pairing API
// ============================================================================

/// Response data for POST /pairing/request
/// POST /pairing/request 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequestData {
    pub status: PairingResponseStatus,
    pub message: String,
}

/// Response data for POST /pairing/respond
/// POST /pairing/respond 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRespondData {
    pub status: String,
}

/// Response data for POST /pairing/reset_notify
/// POST /pairing/reset_notify 响应数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetNotifyData {
    pub status: String,
}

// ============================================================================ Request Types
// ============================================================================

/// Request body for POST /chunks
/// POST /chunks 请求体
#[derive(Debug, Clone, Deserialize)]
pub struct ChunksRequest {
    pub ids: Vec<u32>,
}
