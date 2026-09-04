use crate::entities::FileInfo;
use crate::enums::SimilarityType;
use crate::errors::AppError;
use crate::repositories::{file_content_embedding_repo, file_info_repo};
use crate::structs::file_metadata::AudioType;
use crate::structs::search_result::SearchResult;
use crate::traits::similarity_detector::SimilarityDetector;
use crate::utils::audio_util::{compare_music_fingerprints, MusicFingerprint};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

pub struct AudioSimilarityDetector;

#[async_trait]
impl SimilarityDetector for AudioSimilarityDetector {
    async fn find_similars(&self, file_info: &FileInfo, limit: usize) -> Result<Vec<SearchResult>, AppError> {
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

    /// Find similar audio files based on transcription text embeddings
    /// 基于转录文本嵌入查找相似音频文件
    async fn find_similars_by_transcription(
        &self,
        file_info: &FileInfo,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        let embeddings = file_content_embedding_repo::list_by_file_id(file_info.id)?;

        if embeddings.is_empty() {
            return Ok(Vec::new());
        }

        let source_embedding = &embeddings[0];
        let sparse_indices: Vec<u32> = source_embedding.sparse_vec.indices.clone();
        let sparse_values: Vec<f32> = source_embedding.sparse_vec.values.clone();

        // Hybrid search: combine dense vector similarity with sparse vector (BM25) scoring
        let similar_embeddings = file_content_embedding_repo::hybrid_search(
            &source_embedding.embedding,
            &sparse_indices,
            &sparse_values,
            10, // min_score threshold
        )?;

        // Filter to only Audio category files (category = 3)
        let audio_file_ids: HashSet<i64> = file_info_repo::list_ids_by_category(3)?
            .into_iter()
            .collect();

        // Aggregate scores by file_id, keeping the maximum score per file
        let mut file_scores: HashMap<i64, usize> = HashMap::new();
        for emb in &similar_embeddings {
            if emb.file_id == file_info.id {
                continue;
            }
            if !audio_file_ids.contains(&emb.file_id) {
                continue;
            }
            let max_score = file_scores.entry(emb.file_id).or_insert(0);
            *max_score = (*max_score).max(emb.score);
        }

        let mut sorted_files: Vec<(i64, usize)> = file_scores.into_iter().collect();
        sorted_files.sort_by(|a, b| b.1.cmp(&a.1));
        sorted_files.truncate(limit);

        let file_ids: Vec<i64> = sorted_files.iter().map(|(id, _)| *id).collect();
        let files = file_info_repo::list_by_ids(&file_ids)?;

        let file_map: HashMap<i64, FileInfo> = files.into_iter().map(|f| (f.id, f)).collect();
        let results: Vec<SearchResult> = sorted_files
            .into_iter()
            .filter_map(|(file_id, score)| {
                file_map.get(&file_id).map(|info| SearchResult {
                    score,
                    hit_types: vec![],
                    file_info: info.clone(),
                    matched_keywords: HashSet::new(),
                    matched_chunk_ids: HashSet::new(),
                    similarity_type: Some(SimilarityType::AudioTranscription),
                    source_device: None,
                })
            })
            .collect();

        Ok(results)
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
    // Query Music type files directly using audio_type index (category=3, audio_type=2)
    let music_file_ids = file_info_repo::list_ids_by_category_and_audio_type(3, AudioType::Music as i32)?;
    let music_files = file_info_repo::list_by_ids(&music_file_ids)?;

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
