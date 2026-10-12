use crate::entities::FileInfo;
use crate::enums::SimilarityType;
use crate::errors::AppError;
use crate::repositories::{file_content_embedding_repo, file_info_repo};
use crate::structs::file_metadata::AudioType;
use crate::structs::search_result::SearchResult;
use crate::traits::similarity_detector::SimilarityDetector;
use crate::utils::audio_util::{MusicFingerprint, compare_music_fingerprints};
use async_trait::async_trait;
use std::collections::HashSet;

pub struct AudioSimilarityDetector;

#[async_trait]
impl SimilarityDetector for AudioSimilarityDetector {
    async fn find_similars(
        &self,
        file_info: &FileInfo,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        // Use audio_type from database column for efficiency
        let audio_type = AudioType::from(file_info.audio_type);

        match audio_type {
            AudioType::Music | AudioType::Mixed => {
                self.find_similars_by_fingerprint(file_info, limit).await
            }
            AudioType::Speech | AudioType::Unknown => {
                self.find_similars_by_transcription(file_info, limit).await
            }
        }
    }
}

impl AudioSimilarityDetector {
    /// Find similar music files based on audio fingerprint
    /// 基于音频指纹查找相似音乐文件
    async fn find_similars_by_fingerprint(
        &self,
        file_info: &FileInfo,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        // Read fingerprint from database (already extracted during indexing)
        // 从数据库读取指纹（索引时已提取并存储）
        let source_fingerprint = match file_info.audio_fingerprint.as_ref() {
            Some(bytes) => match MusicFingerprint::from_bytes(bytes) {
                Some(fp) => fp,
                None => {
                    log::warn!("Failed to decode fingerprint for file: {}", file_info.id);
                    return Ok(Vec::new());
                }
            },
            None => {
                log::warn!("No fingerprint stored for music file: {}", file_info.id);
                return Ok(Vec::new());
            }
        };

        find_similars_by_fingerprint_value(&source_fingerprint, Some(file_info.id), limit).await
    }

    /// Find similar audio files based on transcription text embeddings.
    /// Uses all chunk embeddings with max-aggregation via the shared
    /// `find_similars_from_chunks` helper (consistent with the external path).
    /// 基于转录文本嵌入查找相似音频文件。
    /// 经共享的 `find_similars_from_chunks` 使用全部分块、最大分聚合
    /// (与外部现场路径一致)。
    async fn find_similars_by_transcription(
        &self,
        file_info: &FileInfo,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        let embeddings = file_content_embedding_repo::list_by_file_id(file_info.id)?;
        if embeddings.is_empty() {
            return Ok(Vec::new());
        }

        let chunks: Vec<(Vec<f32>, crate::structs::sparse_vector::SparseVector)> = embeddings
            .into_iter()
            .map(|e| (e.embedding.to_vec(), e.sparse_vec))
            .collect();

        crate::similarity::document_similarity::find_similars_from_chunks(
            &chunks,
            3, // Audio category
            Some(file_info.id),
            limit,
            SimilarityType::AudioTranscription,
        )
        .await
    }
}

/// Find similar music files with an in-memory source fingerprint
/// 使用内存中的源指纹检索相似音乐（已索引与外部现场计算共用）
///
/// `exclude_file_id` is None for external files that have no index row.
pub async fn find_similars_by_fingerprint_value(
    source_fingerprint: &MusicFingerprint,
    exclude_file_id: Option<i64>,
    limit: usize,
) -> Result<Vec<SearchResult>, AppError> {
    // Candidates must include BOTH Music and Mixed: the detector routes both
    // here and both store fingerprints at index time — querying Music only
    // meant mixed (vocal) tracks could never match each other.
    // 候选必须同时含 Music 与 Mixed:检测器把两者都路由到这里,且两者入库时
    // 都存指纹 —— 只查 Music 会导致人声混合曲目永远互不匹配。
    let mut music_file_ids =
        file_info_repo::list_ids_by_category_and_audio_type(3, AudioType::Music as i32)?;
    music_file_ids.extend(file_info_repo::list_ids_by_category_and_audio_type(
        3,
        AudioType::Mixed as i32,
    )?);
    let music_files = file_info_repo::list_meta_by_ids(&music_file_ids)?;

    let mut similarities: Vec<(FileInfo, f32)> = Vec::new();

    for candidate in music_files {
        if Some(candidate.id) == exclude_file_id {
            continue;
        }

        // Read fingerprint from database and calculate similarity score
        // 从数据库读取指纹并计算相似度
        if let Some(candidate_bytes) = candidate.audio_fingerprint.as_ref() {
            if let Some(candidate_fp) = MusicFingerprint::from_bytes(candidate_bytes) {
                let similarity = compare_music_fingerprints(source_fingerprint, &candidate_fp);

                if similarity >= 30.0 {
                    similarities.push((candidate, similarity));
                }
            }
        }
    }

    similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    similarities.truncate(limit);

    let results: Vec<SearchResult> = similarities
        .into_iter()
        .map(|(info, score)| SearchResult {
            score: score as usize,
            hit_types: vec![],
            file_info: info,
            matched_keywords: HashSet::new(),
            matched_chunk_ids: HashSet::new(),
            similarity_type: Some(SimilarityType::AudioFingerprint),
            source_device: None,
        })
        .collect();

    Ok(results)
}
