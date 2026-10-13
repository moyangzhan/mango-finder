//! /ping plus /cluster data endpoints serving remote devices' requests
//! /ping 与 /cluster 数据端点:服务远程设备的请求

use crate::cluster::api_types::{
    CODE_DEVICE_NOT_PAIRED, CODE_INTERNAL_ERROR, CODE_MISSING_DEVICE_ID, CODE_NOT_FOUND,
    ChunksData, ChunksRequest, CommonResponse, FindSimilarsData, PingData, SearchData,
};
use crate::enums::PairingStatus;
use crate::global::STORAGE_PATH;
use crate::repositories::{device_repo, file_content_embedding_repo, file_info_repo};
use crate::searcher;
use crate::server::http_server::ServerState;
use crate::similarity::{local_similarity_service, remote_similarity_service};
use crate::structs::cluster_config::{
    DeviceCapabilities, RemoteFindSimilarsRequest, RemoteSearchRequest,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Json, Response},
};
use rust_i18n::t;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;

/// Cache for processed request IDs to prevent loops
/// 用于防止循环的已处理请求ID缓存
/// Key: request_id, Value: timestamp when processed
static PROCESSED_REQUESTS: RwLock<Option<HashMap<String, Instant>>> = RwLock::new(None);

/// Cache expiration time in seconds (10 minutes)
/// 缓存过期时间（10分钟）
const REQUEST_CACHE_TTL_SECS: u64 = 600;

/// Verify that the requester device is paired (returns CommonResponse error)
/// 验证请求者设备是否已配对（返回 CommonResponse 错误）
fn verify_paired_device<T: Serialize>(headers: &HeaderMap) -> Result<&str, CommonResponse<T>> {
    let requester_id = headers
        .get("X-Device-ID")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            log::warn!("Missing X-Device-ID header");
            CommonResponse::error(CODE_MISSING_DEVICE_ID, "Missing X-Device-ID header")
        })?;

    let device = device_repo::get_by_device_id(requester_id).map_err(|e| {
        log::error!("Failed to get device: {}", e);
        CommonResponse::error(CODE_INTERNAL_ERROR, "Failed to get device")
    })?;

    match device {
        Some(d) if d.pairing_status == PairingStatus::Paired => Ok(requester_id),
        Some(d) => {
            log::warn!(
                "Device {} not paired, status: {:?}",
                requester_id,
                d.pairing_status
            );
            Err(CommonResponse::error(
                CODE_DEVICE_NOT_PAIRED,
                "Device not paired",
            ))
        }
        None => {
            log::warn!("Device {} not found", requester_id);
            Err(CommonResponse::error(
                CODE_DEVICE_NOT_PAIRED,
                "Device not found",
            ))
        }
    }
}

/// Verify that the requester device is paired (returns StatusCode for binary responses)
/// 验证请求者设备是否已配对（为二进制响应返回 StatusCode）
fn verify_paired_device_status(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let requester_id = headers
        .get("X-Device-ID")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            log::warn!("Missing X-Device-ID header");
            StatusCode::BAD_REQUEST
        })?;

    let device = device_repo::get_by_device_id(requester_id).map_err(|e| {
        log::error!("Failed to get device: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    match device {
        Some(d) if d.pairing_status == PairingStatus::Paired => Ok(requester_id),
        Some(d) => {
            log::warn!(
                "Device {} not paired, status: {:?}",
                requester_id,
                d.pairing_status
            );
            Err(StatusCode::FORBIDDEN)
        }
        None => {
            log::warn!("Device {} not found", requester_id);
            Err(StatusCode::FORBIDDEN)
        }
    }
}

/// Initialize the processed requests cache
pub fn init_request_cache() {
    let mut cache = PROCESSED_REQUESTS.write().unwrap_or_else(|e| {
        log::error!("PROCESSED_REQUESTS lock poisoned, recovering: {}", e);
        e.into_inner()
    });
    if cache.is_none() {
        *cache = Some(HashMap::new());
    }
}

/// Check if request has been processed, and mark as processed if not
/// Returns true if the request was already processed (should be ignored)
/// Returns false if this is a new request (should be processed)
fn check_and_mark_request(request_id: &str) -> bool {
    let mut cache_guard = PROCESSED_REQUESTS.write().unwrap_or_else(|e| {
        log::error!("PROCESSED_REQUESTS lock poisoned, recovering: {}", e);
        e.into_inner()
    });
    if let Some(ref mut cache) = *cache_guard {
        let now = Instant::now();

        // Check if already processed
        if let Some(&timestamp) = cache.get(request_id) {
            if now.duration_since(timestamp).as_secs() < REQUEST_CACHE_TTL_SECS {
                log::warn!(
                    "Request {} already processed, ignoring to prevent loop",
                    request_id
                );
                return true;
            }
            // Expired, remove old entry
            cache.remove(request_id);
        }

        // Mark as processed
        cache.insert(request_id.to_string(), now);

        // Periodically clean up expired entries (every 100 requests or so)
        if cache.len() > 100 {
            cache.retain(|_, &mut timestamp| {
                now.duration_since(timestamp).as_secs() < REQUEST_CACHE_TTL_SECS
            });
        }

        false
    } else {
        // Cache not initialized, allow request
        false
    }
}

/// GET /ping - Health check and device info (merged with /device/info)
/// GET /ping - 健康检查和设备信息（合并了 /device/info）
pub async fn handle_ping(State(state): State<Arc<ServerState>>) -> CommonResponse<PingData> {
    log::info!("GET /ping");
    let index_count = file_info_repo::count().unwrap_or(0);
    let capabilities = DeviceCapabilities::default();

    CommonResponse::success(PingData {
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
        status: "online".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        index_count,
        last_index_time: None,
        capabilities,
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

/// POST /search - Remote search request
pub async fn handle_search(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(req): Json<RemoteSearchRequest>,
) -> CommonResponse<SearchData> {
    log::info!(
        "POST /cluster/search query={}, type={}, limit={}",
        req.query,
        req.search_type,
        req.limit
    );
    // Verify requester is paired
    let _requester_id: &str = match verify_paired_device(&headers) {
        Ok(id) => id,
        Err(e) => return e,
    };

    // Execute local search
    let results = match req.search_type.as_str() {
        "keyword" => searcher::keyword_search(&req.query).await,
        _ => {
            // Default to semantic search
            searcher::semantic_search(&req.query).await
        }
    };

    // Limit results
    let results: Vec<_> = results.into_iter().take(req.limit).collect();
    let total = results.len();

    CommonResponse::success(SearchData {
        results,
        total,
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
    })
}

/// POST /chunks - Get text chunks by IDs for remote devices
/// POST /chunks - 根据ID获取文本片段（供远程设备调用）
pub async fn handle_chunks(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(req): Json<ChunksRequest>,
) -> CommonResponse<ChunksData> {
    log::info!("POST /chunks ids={:?}", req.ids);
    // Verify requester is paired
    let _requester_id: &str = match verify_paired_device(&headers) {
        Ok(id) => id,
        Err(e) => return e,
    };

    // Get chunks by IDs
    let chunks = match file_content_embedding_repo::list_chunks_by_ids(&req.ids) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to get chunks: {}", e);
            return CommonResponse::error(CODE_INTERNAL_ERROR, "Failed to get chunks");
        }
    };

    CommonResponse::success(ChunksData {
        chunks,
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
    })
}

/// POST /cluster/file/:file_id/similars - Find similar files across devices with dual loop prevention
/// POST /cluster/file/:file_id/similars - 跨设备查找相似文件（双重防循环机制）
///
/// This endpoint is called by remote devices to find similar files.
/// It searches locally and forwards the request to other devices.
///
/// Dual loop prevention (双重保证防止死循环):
/// 1. exclude_device_ids: Devices to skip when forwarding requests
///    排除列表：转发请求时跳过的设备
/// 2. request_id: Unique ID cached for 10 minutes to prevent reprocessing
///    请求ID：缓存10分钟，防止重复处理同一请求
pub async fn handle_find_similars(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(file_id): Path<i64>,
    Json(req): Json<RemoteFindSimilarsRequest>,
) -> CommonResponse<FindSimilarsData> {
    log::info!(
        "POST /cluster/file/{}/similars request_id={}, limit={}",
        file_id,
        req.request_id,
        req.limit
    );

    // Verify requester is paired first
    let requester_id: &str = match verify_paired_device(&headers) {
        Ok(id) => id,
        Err(e) => return e,
    };

    // Check if this request has already been processed (prevent loops)
    if check_and_mark_request(&req.request_id) {
        log::warn!(
            "Request {} already processed, returning empty results to prevent loop",
            req.request_id
        );
        return CommonResponse::success(FindSimilarsData {
            results: vec![],
            total: 0,
            device_id: state.device_id.clone(),
            device_name: state.device_name.clone(),
        });
    }

    // Get file info
    let file_info = match file_info_repo::get_by_id(file_id) {
        Ok(Some(f)) => f,
        Ok(None) => {
            log::warn!("File not found: {}", file_id);
            return CommonResponse::error(
                CODE_NOT_FOUND,
                &t!("message.file-not-found").to_string(),
            );
        }
        Err(e) => {
            log::error!("Failed to get file: {}", e);
            return CommonResponse::error(
                CODE_INTERNAL_ERROR,
                &t!("message.internal-error").to_string(),
            );
        }
    };

    let limit = req.limit;

    // Build exclude list: add ourselves and the requester
    let mut exclude_device_ids = req.exclude_device_ids.clone();
    exclude_device_ids.push(state.device_id.clone());
    if !requester_id.is_empty() && !exclude_device_ids.contains(&requester_id.to_string()) {
        exclude_device_ids.push(requester_id.to_string());
    }

    // 1. Search locally
    let local_results =
        match local_similarity_service::find_similars_by_file_id(&file_info, limit).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to find local similar files: {}", e);
                return CommonResponse::error(
                    CODE_INTERNAL_ERROR,
                    &t!("message.internal-error").to_string(),
                );
            }
        };

    // 2. Search on other remote devices (with updated exclude list)
    //    Pass the same request_id to prevent loops
    let remote_results = remote_similarity_service::remote_find_similars_with_exclude(
        &file_info,
        limit,
        &exclude_device_ids,
        &req.request_id,
    )
    .await;

    // 3. Merge results
    let mut all_results = local_results;
    all_results.extend(remote_results);
    all_results.sort_by(|a, b| b.score.cmp(&a.score));
    all_results.truncate(limit);
    let total = all_results.len();

    CommonResponse::success(FindSimilarsData {
        results: all_results,
        total,
        device_id: state.device_id.clone(),
        device_name: state.device_name.clone(),
    })
}

/// GET /file/:file_id - Download file from this device by file ID
/// Only indexed files can be downloaded (security measure)
pub async fn handle_file_download(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(file_id): Path<i64>,
) -> Result<Response<Body>, StatusCode> {
    log::info!("GET /file/{}", file_id);
    // Verify requester is paired
    let requester_id = verify_paired_device_status(&headers)?;

    // Get file info from database by file_id (only indexed files)
    let file_info = file_info_repo::get_by_id(file_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let file_path = &file_info.path;

    log::info!(
        "Remote file request from {}: {} (id: {})",
        requester_id,
        file_path,
        file_id
    );

    // Read file data (limit to 200 MB to avoid OOM on large files)
    const MAX_DOWNLOAD_SIZE: u64 = 200 * 1024 * 1024;
    let file_meta = std::fs::metadata(file_path).map_err(|e| {
        log::error!("Failed to stat file {}: {}", file_path, e);
        StatusCode::NOT_FOUND
    })?;
    if file_meta.len() > MAX_DOWNLOAD_SIZE {
        log::warn!(
            "File too large for download ({} bytes): {}",
            file_meta.len(),
            file_path
        );
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let file_data = std::fs::read(file_path).map_err(|e| {
        log::error!("Failed to read file {}: {}", file_path, e);
        StatusCode::NOT_FOUND
    })?;

    // Determine content type based on extension
    let content_type = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext.to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "html" | "htm" => "text/html",
            "json" => "application/json",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            _ => "application/octet-stream",
        })
        .unwrap_or("application/octet-stream");

    // Build response with file data
    // Sanitize filename and encode per RFC 5987 to handle special characters and prevent header injection
    let safe_filename = file_info
        .name
        .replace('\\', "")
        .replace('"', "")
        .replace('\r', "")
        .replace('\n', "");
    let encoded_filename: String = safe_filename
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
                vec![b as char]
            } else {
                format!("%{:02X}", b).chars().collect()
            }
        })
        .collect();
    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        safe_filename, encoded_filename
    );

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header("X-Device-Id", &state.device_id)
        .header("X-Device-Name", &state.device_name)
        .body(Body::from(file_data))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

/// GET /file/:file_id/content - Get file content/parsed content by file ID for remote devices
/// GET /file/:file_id/content - 获取文件内容/解析内容（供远程设备调用）
pub async fn handle_file_content(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Path(file_id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    log::info!("GET /file/{}/content", file_id);
    // Verify requester is paired
    verify_paired_device_status(&headers)?;

    // Get file info from database
    let mut file_info = file_info_repo::get_by_id(file_id)
        .map_err(|e| {
            log::error!("Failed to get file info: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // In "file" storage mode, content_ref_path points to a .md file — resolve to actual content
    if let Some(ref relative_path) = file_info.content_ref_path {
        if let Some(storage) = STORAGE_PATH.get() {
            let md_path = std::path::Path::new(storage).join(relative_path);
            file_info.content = std::fs::read_to_string(&md_path).unwrap_or_else(|e| {
                log::warn!(
                    "Failed to read parsed document {}: {}",
                    md_path.display(),
                    e
                );
                String::new()
            });
        } else {
            file_info.content = String::new();
        }
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "content": file_info.content,
            "deviceId": state.device_id,
            "deviceName": state.device_name
        }
    })))
}
