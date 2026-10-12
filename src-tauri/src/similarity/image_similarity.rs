use crate::entities::FileInfo;
use crate::enums::SimilarityType;
use crate::errors::AppError;
use crate::repositories::{file_content_embedding_repo, file_info_repo};
use crate::structs::image_similarity_candidate::ImageSimilarityCandidate;
use crate::structs::search_result::SearchResult;
use crate::traits::similarity_detector::SimilarityDetector;
use async_trait::async_trait;
use image_hasher::{HashAlg, HasherConfig, ImageHash};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

/// Candidate limit for ANN pre-filtering
/// ANN 预过滤的候选数量限制
pub const ANN_CANDIDATE_LIMIT: usize = 200;

pub struct ImageSimilarityDetector;

#[async_trait]
impl SimilarityDetector for ImageSimilarityDetector {
    /// Find similar images using two-stage approach:
    /// 1. Use sqlite-vec ANN search for semantic similarity (embeddings)
    /// 2. Calculate perceptual hash distance for top candidates
    /// 3. Combine scores: max(visual_score, semantic_score)
    ///
    /// 使用两阶段方法查找相似图片：
    /// 1. 使用 sqlite-vec ANN 搜索语义相似性（嵌入向量）
    /// 2. 对候选结果计算感知哈希距离
    /// 3. 合并分数：max(视觉分数, 语义分数)
    async fn find_similars(
        &self,
        file_info: &FileInfo,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        // Get source hash: use stored hash if available, otherwise calculate from file
        let source_hash = match &file_info.image_hash {
            Some(hash_bytes) => bytes_to_image_hash(hash_bytes),
            None => {
                let source_path = Path::new(&file_info.path);
                if !source_path.exists() {
                    return Ok(Vec::new());
                }
                let source_image = match image::open(source_path) {
                    Ok(img) => img,
                    Err(e) => {
                        log::warn!("Failed to open source image {}: {}", file_info.path, e);
                        return Ok(Vec::new());
                    }
                };
                let hasher = create_hasher();
                hasher.hash_image(&source_image)
            }
        };

        // Stage 1: Use ANN search to find semantic candidates
        let candidates = file_content_embedding_repo::find_image_similarity_candidates(
            file_info.id,
            ANN_CANDIDATE_LIMIT,
        )?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Stage 2: Calculate hybrid scores for candidates
        score_image_candidates(&source_hash, &candidates, Some(file_info.id), limit)
    }
}

/// Score candidates against the source hash and build search results
/// 依据源哈希对候选计算融合分并组装结果（已索引与外部现场计算共用）
///
/// `exclude_file_id` is None for external files that have no index row.
pub fn score_image_candidates(
    source_hash: &ImageHash,
    candidates: &[ImageSimilarityCandidate],
    exclude_file_id: Option<i64>,
    limit: usize,
) -> Result<Vec<SearchResult>, AppError> {
    // (file_id, score, similarity_type)
    let mut file_results: Vec<(i64, usize, SimilarityType)> = Vec::new();

    for candidate in candidates {
        if Some(candidate.file_id) == exclude_file_id {
            continue;
        }

        // Calculate visual similarity score using stored hash
        let visual_score =
            match calculate_visual_score_from_stored(source_hash, &candidate.image_hash) {
                Some(score) => score,
                None => continue,
            };

        // Use pre-computed semantic score from ANN search
        let semantic_score = candidate.semantic_score;

        // Take maximum of visual and semantic scores via the shared helper,
        // so the rule lives in exactly one place.
        // 复用共享 helper 取视觉/语义最大值,规则只维护一份
        let (final_score, similarity_type) = merge_hybrid_scores(visual_score, semantic_score);

        // Only include if similarity >= 30%
        if final_score >= 30 {
            file_results.push((candidate.file_id, final_score, similarity_type));
        }
    }

    // Sort by score and take top N
    file_results.sort_by(|a, b| b.1.cmp(&a.1));
    file_results.truncate(limit);

    // Fetch file info for results
    let file_ids: Vec<i64> = file_results.iter().map(|(id, _, _)| *id).collect();
    let file_list = file_info_repo::list_meta_by_ids(&file_ids)?;
    let file_map: HashMap<i64, FileInfo> = file_list.into_iter().map(|f| (f.id, f)).collect();

    let results: Vec<SearchResult> = file_results
        .into_iter()
        .filter_map(|(file_id, score, similarity_type)| {
            file_map.get(&file_id).map(|info| SearchResult {
                score,
                hit_types: vec![],
                file_info: info.clone(),
                matched_keywords: HashSet::new(),
                matched_chunk_ids: HashSet::new(),
                similarity_type: Some(similarity_type),
                source_device: None,
            })
        })
        .collect();

    Ok(results)
}

/// Create hasher with Gradient algorithm for perceptual hash
/// 创建使用 Gradient 算法的哈希器
pub fn create_hasher() -> image_hasher::Hasher {
    HasherConfig::new()
        .hash_size(8, 8)
        .hash_alg(HashAlg::Gradient)
        .to_hasher()
}

/// Calculate image hash from file path
/// 从文件路径计算图像哈希
pub fn calculate_image_hash(path: &str) -> Option<Vec<u8>> {
    let image_path = Path::new(path);
    if !image_path.exists() {
        return None;
    }

    let img = image::open(image_path).ok()?;
    let hasher = create_hasher();
    let hash = hasher.hash_image(&img);
    Some(image_hash_to_bytes(&hash))
}

/// Convert ImageHash to bytes for storage
/// 将 ImageHash 转换为字节用于存储
pub fn image_hash_to_bytes(hash: &ImageHash) -> Vec<u8> {
    hash.as_bytes().to_vec()
}

/// Convert bytes back to ImageHash
/// 将字节转换回 ImageHash
pub fn bytes_to_image_hash(bytes: &[u8]) -> ImageHash {
    ImageHash::from_bytes(bytes).unwrap_or_else(|_| {
        // Return a zero hash if conversion fails
        ImageHash::from_bytes(&[0u8; 8]).unwrap()
    })
}

/// Calculate visual similarity score using stored hash bytes
/// 使用存储的哈希字节计算视觉相似度分数
pub fn calculate_visual_score_from_stored(
    source_hash: &ImageHash,
    candidate_hash_bytes: &Option<Vec<u8>>,
) -> Option<usize> {
    // Use stored hash if available
    let candidate_hash = match candidate_hash_bytes {
        Some(hash_bytes) => bytes_to_image_hash(hash_bytes),
        None => return None, // No hash available, skip this candidate
    };

    // Calculate hamming distance (lower = more similar)
    let distance = source_hash.dist(&candidate_hash);

    // Convert distance to score (0-100)
    // Max distance for 8x8 hash is 64 bits
    // distance 0 = 100, distance 10 = 0
    if distance <= 10 {
        Some(((10 - distance) * 10) as usize)
    } else {
        // For distances > 10, convert to a low score
        // distance 11 = ~9, distance 64 = 0
        let score = ((64 - distance) as f32 * 100.0 / 53.0) as usize;
        Some(score.min(29)) // Cap at 29 for distances > 10
    }
}

/// Calculate visual similarity score from hash bytes directly
/// 直接从哈希字节计算视觉相似度分数
pub fn calculate_visual_score(source_hash: &ImageHash, candidate_hash_bytes: &[u8]) -> usize {
    let candidate_hash = bytes_to_image_hash(candidate_hash_bytes);
    let distance = source_hash.dist(&candidate_hash);

    // Convert distance to score (0-100)
    if distance <= 10 {
        ((10 - distance) * 10) as usize
    } else {
        let score = ((64 - distance) as f32 * 100.0 / 53.0) as usize;
        score.min(29)
    }
}

/// Merge visual and semantic scores, taking the maximum
/// 合并视觉和语义分数，取最大值
pub fn merge_hybrid_scores(visual_score: usize, semantic_score: usize) -> (usize, SimilarityType) {
    if visual_score >= semantic_score {
        (visual_score, SimilarityType::ImageHash)
    } else {
        (semantic_score, SimilarityType::ImageSemantic)
    }
}
