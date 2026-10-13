//! Similarity search service
//!
//! This module provides unified similarity search functionality that combines local and remote device searches.

use crate::audio_parser::AudioParser;
use crate::cluster::http_client;
use crate::embedding_service_manager;
use crate::entities::FileInfo;
use crate::enums::{FileCategory, PairingStatus, SimilarityType};
use crate::global::{CLIENT_ID, EXT_TO_DOC_LOADER, MAX_DOCUMENT_LOAD_CHARS};
use crate::image_parser;
use crate::repositories::{device_repo, file_content_embedding_repo, file_info_repo};
use crate::similarity::{
    audio_similarity, document_similarity, image_similarity, local_similarity_service,
    remote_similarity_service,
};
use crate::structs::file_metadata::AudioType;
use crate::structs::image_similarity_candidate::ImageSimilarityCandidate;
use crate::structs::search_result::{SearchResult, SourceDevice};
use crate::structs::sparse_vector::SparseVector;
use crate::utils::audio_util;
use crate::utils::text_util;

/// Find similar files for a local file
/// 为本地文件查找相似文件
///
/// Searches both local device and remote devices in parallel, then merges and sorts results by score.
pub async fn find_similars_for_local_file(
    file_id: i64,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    let file_info =
        file_info_repo::get_by_id(file_id)?.ok_or_else(|| "File not found".to_string())?;

    // Search local and remote devices in parallel
    let (local_results, mut remote_results) = tokio::join!(
        local_similarity_service::find_similars_by_file_id(&file_info, limit),
        remote_similarity_service::remote_find_similars(&file_info, limit)
    );

    let local_results = local_results.map_err(|e| e.to_string())?;

    // Merge and sort results
    let mut all_results = local_results;
    all_results.append(&mut remote_results);
    all_results.sort_by(|a, b| b.score.cmp(&a.score));
    all_results.truncate(limit);

    Ok(all_results)
}

/// Find similar files for a remote file (cross-device)
/// 为远程文件查找相似文件（跨设备）
///
/// Simplified approach:
/// 1. Send request to source device to find similars (it will search locally + forward to other devices)
/// 2. Search locally in parallel
/// 3. Merge results
pub async fn find_similars_for_remote_file(
    source_device_id: &str,
    remote_file_id: i64,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    log::info!(
        "find_similars_for_remote_file: source_device_id={}, remote_file_id={}, limit={}",
        source_device_id,
        remote_file_id,
        limit
    );

    // Check test mode
    if crate::global::TEST_MODE_REMOTE_DEVICE.load(std::sync::atomic::Ordering::Relaxed) {
        log::info!("Using mock device for test mode");
        let mock_file_info = FileInfo {
            id: remote_file_id,
            category: 2, // Image
            ..Default::default()
        };
        return Ok(crate::cluster::mock::generate_mock_similar_results(
            &mock_file_info,
            limit,
        ));
    }

    // Get source device info
    let source_device = device_repo::get_by_device_id(source_device_id)
        .map_err(|e| format!("Failed to get device: {}", e))?
        .ok_or_else(|| "Device not found".to_string())?;

    if source_device.pairing_status != PairingStatus::Paired {
        return Err("Device is not paired".to_string());
    }

    // Get local device ID for exclude list
    let local_device_id = crate::read_lock!(CLIENT_ID).clone();
    let exclude_device_ids = vec![local_device_id.clone()];

    // Generate a unique request ID for this similarity search
    let request_id = uuid::Uuid::new_v4().to_string();
    log::info!("Generated request_id for similarity search: {}", request_id);

    // 1. Request source device to find similars (it will search locally + forward to other devices)
    let remote_results = match http_client::find_similars(
        remote_file_id,
        Some(&request_id),
        &exclude_device_ids,
        limit,
        &source_device.ip_address,
        source_device.port,
    )
    .await
    {
        Ok(data) => {
            let mut results = data.results;
            // Add source device info
            for result in &mut results {
                if result.source_device.is_none() {
                    result.source_device = Some(SourceDevice {
                        device_id: source_device.device_id.clone(),
                        device_name: source_device.name.clone(),
                    });
                }
            }
            results
        }
        Err(e) => {
            log::warn!("Failed to find similars on source device: {}", e);
            Vec::new()
        }
    };

    // 2. Also search locally
    // Note: We don't have the file locally, so we can only search if we have features
    // For now, we rely on the source device's search results

    // 3. Return results
    log::info!("Total results from remote: {}", remote_results.len());
    Ok(remote_results)
}

/// Outcome of an on-the-fly similarity query for an un-indexed external file
/// 未索引外部文件现场相似度查询的结果
pub struct ExternalSimilarOutcome {
    pub results: Vec<SearchResult>,
    /// Number of chunk embeddings computed on the fly (documents / transcribed audio); 0 for images and music
    pub chunk_count: usize,
    /// Detected audio subtype, present only for audio files
    pub audio_type: Option<AudioType>,
    /// Category the file was processed as
    /// 实际按哪种类别处理
    pub category: FileCategory,
}

/// Find similar indexed files for an external local file that is not indexed.
/// 为未索引的外部本地文件查找相似文件
///
/// The file is parsed / captioned / transcribed on the fly and its embeddings are computed in memory; nothing is written to the index.
pub async fn find_similars_for_external_file(
    path: &str,
    limit: usize,
) -> Result<ExternalSimilarOutcome, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match FileCategory::from_ext(&ext) {
        FileCategory::Document => find_similars_for_external_document(path, &ext, limit).await,
        FileCategory::Image => find_similars_for_external_image(path, limit).await,
        FileCategory::Audio => find_similars_for_external_audio(path, limit).await,
        _ => Err(format!(
            "On-the-fly similarity is not supported for {} files; index it first with 'mfd index start <dir>'",
            if ext.is_empty() {
                "extension-less".to_string()
            } else {
                format!(".{}", ext)
            }
        )),
    }
}

/// 文档：解析取文本 → 分块嵌入 → 库内检索
async fn find_similars_for_external_document(
    path: &str,
    ext: &str,
    limit: usize,
) -> Result<ExternalSimilarOutcome, String> {
    let doc_loader = EXT_TO_DOC_LOADER
        .read()
        .await
        .get(ext)
        .cloned()
        .ok_or_else(|| format!("Unsupported document extension: .{}", ext))?;

    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let path_owned = path.to_string();
    let parse_result = tokio::task::spawn_blocking(move || {
        doc_loader.load_max_with_id(
            std::path::Path::new(&path_owned),
            0,
            &name,
            MAX_DOCUMENT_LOAD_CHARS,
        )
    })
    .await
    .map_err(|e| format!("Failed to parse file: {}", e))?;
    let content = parse_result.map_err(|e| format!("Failed to parse file: {}", e))?;

    if content.trim().is_empty() {
        return Ok(ExternalSimilarOutcome {
            results: Vec::new(),
            chunk_count: 0,
            audio_type: None,
            category: FileCategory::Document,
        });
    }

    let chunks = embed_text_to_chunks(&text_util::collapse_newlines(&content)).await?;
    let chunk_count = chunks.len();
    let results = document_similarity::find_similars_from_chunks(
        &chunks,
        1,
        None,
        limit,
        SimilarityType::DocumentSemantic,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(ExternalSimilarOutcome {
        results,
        chunk_count,
        audio_type: None,
        category: FileCategory::Document,
    })
}

/// 图片：现场感知哈希 + 打标嵌入；打标/嵌入失败时退化为纯视觉哈希全量比对
async fn find_similars_for_external_image(
    path: &str,
    limit: usize,
) -> Result<ExternalSimilarOutcome, String> {
    // Decoding and captioning are heavy sync work — off the async worker via spawn_blocking; run them separately so a captioning panic (JoinError) cannot discard the already-computed hash.
    let hash_path = path.to_string();
    let source_hash_bytes =
        tokio::task::spawn_blocking(move || image_similarity::calculate_image_hash(&hash_path))
            .await
            .unwrap_or(None);
    let source_hash_bytes =
        source_hash_bytes.ok_or_else(|| format!("Failed to decode image: {}", path))?;
    let source_hash = image_similarity::bytes_to_image_hash(&source_hash_bytes);

    // Parser init/analyze failure degrades to the visual-only fallback below (same path as an empty caption)
    let content = match image_parser::ImageParser::new().await {
        Ok(parser) => match parser.analyze(path).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Image caption failed, visual-only fallback: {}", e);
                String::new()
            }
        },
        Err(e) => {
            log::warn!("Image parser init failed, visual-only fallback: {}", e);
            String::new()
        }
    };

    // With a semantic vector use ANN candidates; otherwise scan all image hashes for pure visual comparison.
    // The three "no vector" cases (empty caption / no chunks / embed failure) collapse into a single fallback branch, so the fallback behavior is maintained in one place.
    let mut chunk_count = 0;
    let caption_chunks: Option<Vec<_>> = if content.trim().is_empty() {
        None
    } else {
        // Match the indexing write path: collapse consecutive newlines before embedding
        match embed_text_to_chunks(&text_util::collapse_newlines(&content)).await {
            Ok(chunks) if chunks.is_empty() => None,
            Ok(chunks) => Some(chunks),
            Err(e) => {
                log::warn!(
                    "Failed to embed image caption, falling back to visual-only scan: {}",
                    e
                );
                None
            }
        }
    };
    let candidates = match caption_chunks {
        Some(chunks) => {
            chunk_count = chunks.len();
            file_content_embedding_repo::find_image_similarity_candidates_by_vector(
                &chunks[0].0,
                image_similarity::ANN_CANDIDATE_LIMIT,
            )
            .map_err(|e| e.to_string())?
        }
        None => visual_scan_candidates(&source_hash_bytes, limit)?,
    };

    let results = image_similarity::score_image_candidates(&source_hash, &candidates, None, limit)
        .map_err(|e| e.to_string())?;

    Ok(ExternalSimilarOutcome {
        results,
        chunk_count,
        audio_type: None,
        category: FileCategory::Image,
    })
}

/// 音频:解码一次、采样点复用给所有消费者,全部放 spawn_blocking;
/// 分类与索引期同构(特征优先、转写内容兜底);音乐/混合走指纹,语音/未知走转写嵌入
async fn find_similars_for_external_audio(
    path: &str,
    limit: usize,
) -> Result<ExternalSimilarOutcome, String> {
    let decode_path = path.to_string();

    // Decode once + feature classification + music fingerprint, all inside spawn_blocking
    let (audio_type, fingerprint, samples) = tokio::task::spawn_blocking(move || {
        let samples = audio_util::load_audio_to_f32(&decode_path)
            .map_err(|e| format!("Failed to decode audio: {}", e))?;
        let feature_type = audio_util::detect_audio_type_by_features(&samples);
        let fingerprint = audio_util::extract_music_fingerprint(&samples);
        Ok::<_, String>((feature_type, fingerprint, samples))
    })
    .await
    .map_err(|e| format!("Audio decode task failed: {}", e))??;

    match audio_type {
        AudioType::Music | AudioType::Mixed => {
            let results =
                audio_similarity::find_similars_by_fingerprint_value(&fingerprint, None, limit)
                    .await
                    .map_err(|e| e.to_string())?;
            Ok(ExternalSimilarOutcome {
                results,
                chunk_count: 0,
                audio_type: Some(audio_type),
                category: FileCategory::Audio,
            })
        }
        AudioType::Speech | AudioType::Unknown => {
            // Transcribe with the same parser mode as indexing; local reuses the decoded samples, remote reads the file
            let parser = AudioParser::new()
                .await
                .map_err(|e| format!("Failed to init audio parser: {}", e))?;
            let transcription = parser
                .transcribe(path, Some(&samples))
                .await
                .map_err(|e| format!("Failed to transcribe audio: {}", e))?;

            // Feature Unknown: reclassify by transcript (same fallback as indexing); reclassified Music/Mixed goes through the fingerprint path (legacy behavior)
            let audio_type = if audio_type == AudioType::Unknown {
                audio_util::detect_audio_type_from_transcription(&transcription)
            } else {
                audio_type
            };

            if matches!(audio_type, AudioType::Music | AudioType::Mixed) {
                let results =
                    audio_similarity::find_similars_by_fingerprint_value(&fingerprint, None, limit)
                        .await
                        .map_err(|e| e.to_string())?;
                return Ok(ExternalSimilarOutcome {
                    results,
                    chunk_count: 0,
                    audio_type: Some(audio_type),
                    category: FileCategory::Audio,
                });
            }

            if transcription.trim().is_empty() {
                return Ok(ExternalSimilarOutcome {
                    results: Vec::new(),
                    chunk_count: 0,
                    audio_type: Some(audio_type),
                    category: FileCategory::Audio,
                });
            }

            let chunks =
                embed_text_to_chunks(&text_util::collapse_newlines(&transcription)).await?;
            let chunk_count = chunks.len();
            let results = document_similarity::find_similars_from_chunks(
                &chunks,
                3,
                None,
                limit,
                SimilarityType::AudioTranscription,
            )
            .await
            .map_err(|e| e.to_string())?;

            Ok(ExternalSimilarOutcome {
                results,
                chunk_count,
                audio_type: Some(audio_type),
                category: FileCategory::Audio,
            })
        }
    }
}

/// 视觉回退:经 SQLite UDF 在库内按汉明距离过滤,只取匹配行
/// Visual fallback: SQLite UDF filters by hamming distance in-DB
fn visual_scan_candidates(
    source_hash: &[u8],
    limit: usize,
) -> Result<Vec<ImageSimilarityCandidate>, String> {
    // score >= 30 needs distance <= 7 (score = (10 - d) * 10);
    // SQL LIMIT = max(user limit, ANN_CANDIDATE_LIMIT), no truncation
    let sql_limit = (limit.max(image_similarity::ANN_CANDIDATE_LIMIT)) as i64;
    let rows = file_info_repo::find_visual_candidates_by_hash(source_hash, 7, sql_limit)
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(file_id, image_hash)| ImageSimilarityCandidate {
            file_id,
            semantic_score: 0,
            image_hash,
        })
        .collect())
}

/// 文本分块并逐块计算嵌入（仅内存，不写库），供外部文件现场计算使用
async fn embed_text_to_chunks(content: &str) -> Result<Vec<(Vec<f32>, SparseVector)>, String> {
    let chunks = {
        let mut manager = embedding_service_manager::get_manager().write().await;
        let embedding_service = manager.service().await.map_err(|e| e.to_string())?;
        text_util::split_text(content, &embedding_service.tokenizer)
            .map_err(|e| format!("Failed to split text: {}", e))?
    };

    let mut result = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        if chunk.trim().is_empty() {
            continue;
        }
        let mut manager = embedding_service_manager::get_manager().write().await;
        let embed_result = manager
            .embed(chunk)
            .await
            .map_err(|e| format!("Failed to embed text: {}", e))?;
        result.push((embed_result.dense, embed_result.sparse));
    }
    Ok(result)
}
