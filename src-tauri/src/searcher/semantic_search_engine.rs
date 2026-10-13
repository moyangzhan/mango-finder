use crate::embedding_service_manager::get_manager;
use crate::entities::{FileContentEmbedding, FileInfo, FileMetaEmbedding};
use crate::enums::HitType;
use crate::errors::AppError;
use crate::global::{QUERY_MIN_SCORE, SHORT_QUERY_LEN};
use crate::repositories::{
    file_content_embedding_repo, file_info_repo, file_metadata_embedding_repo,
};
use crate::searcher::fts_search_engine;
use crate::structs::embed_result::EmbedResult;
use crate::structs::search_result::SearchResult;
use crate::utils::search_util::STOPWORDS;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::{task, try_join};

#[derive(Debug, Clone)]
struct SearchTmp {
    file_id: i64,
    max_score: usize,
    chunk_ids: HashSet<i64>,
    hit_types: HashSet<HitType>,
    match_keywords: HashSet<String>,
}

pub async fn warmup_embedding_service() -> Result<(), AppError> {
    let mut manager = get_manager().write().await;
    manager.warmup().await?;
    Ok(())
}

pub async fn search(query: &str) -> Vec<SearchResult> {
    log::debug!("Search: {}", query);
    let start = Instant::now();
    let embed_result = {
        let mut manager = get_manager().write().await;
        manager.embed(query).await.unwrap_or_else(|e| {
            log::error!("embed query failed, error: {:?}", e);
            EmbedResult::default()
        })
    };
    if embed_result.dense.is_empty() {
        return Vec::new();
    }
    let short_query = embed_result.sparse.indices.len() <= SHORT_QUERY_LEN;
    let embedding = Arc::new(embed_result.dense);
    let indices = Arc::new(embed_result.sparse.indices);
    let values = Arc::new(embed_result.sparse.values);
    let (content_result, meta_result, fts_result) = try_join!(
        task::spawn_blocking({
            let (emb, idx, val) = (embedding.clone(), indices.clone(), values.clone());
            move || {
                file_content_embedding_repo::hybrid_search(&emb, &idx, &val, QUERY_MIN_SCORE)
                    .map_err(|e| {
                        log::error!("content search failed: {:?}", e);
                        e
                    })
                    .unwrap_or_default()
            }
        }),
        task::spawn_blocking({
            let (emb, idx, val) = (embedding.clone(), indices.clone(), values.clone());
            move || {
                file_metadata_embedding_repo::hybrid_search(&emb, &idx, &val, QUERY_MIN_SCORE)
                    .map_err(|e| {
                        log::error!("meta search failed: {:?}", e);
                        e
                    })
                    .unwrap_or_default()
            }
        }),
        async {
            if indices.is_empty() || !short_query {
                return Ok(Vec::new());
            }
            let lower_query = query.to_lowercase();
            let q = lower_query
                .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                .filter(|w| !w.is_empty() && !STOPWORDS.contains(w))
                .collect::<Vec<_>>()
                .join(" ");
            Ok(fts_search_engine::search(&q).await)
        },
    )
    .unwrap_or_else(|error| {
        log::error!("Search error: {:?}", error);
        (vec![], vec![], vec![])
    });
    let result = merge_and_filter_results(content_result, meta_result, fts_result);
    log::debug!("Search cost: {:?}", start.elapsed());
    result
}

/// Merge and filter the results from content and meta search
fn merge_and_filter_results(
    content_result: Vec<FileContentEmbedding>,
    meta_result: Vec<FileMetaEmbedding>,
    fts_result: Vec<SearchResult>,
) -> Vec<SearchResult> {
    if content_result.is_empty() && meta_result.is_empty() && fts_result.is_empty() {
        return Vec::new();
    }
    let mut file_map: HashMap<i64, SearchTmp> = HashMap::new();

    // 1. Process content-path results
    for item in content_result {
        log::debug!(
            "Content Search - file_id: {}, chunk_id: {}, score: {}",
            item.file_id,
            item.chunk_index,
            item.score
        );
        let entry = file_map.entry(item.file_id).or_insert(SearchTmp {
            file_id: item.file_id,
            max_score: 0,
            chunk_ids: HashSet::new(),
            hit_types: HashSet::new(),
            match_keywords: HashSet::new(),
        });
        entry.chunk_ids.insert(item.id);
        entry.hit_types.insert(HitType::ContentSemantic);
        if item.score > entry.max_score {
            entry.max_score = item.score;
        }
    }

    // 2. Process metadata results (1.1x boost)
    for item in meta_result {
        log::debug!(
            "Meta Search - file_id: {}, score: {}, distance: {}, sparse_score: {}",
            item.file_id,
            item.score,
            item.distance,
            item.sparse_score
        );
        let boosted_score = (item.score as f32 * 1.1).min(100.0) as usize;
        let entry = file_map.entry(item.file_id).or_insert(SearchTmp {
            file_id: item.file_id,
            max_score: 0,
            chunk_ids: HashSet::new(),
            hit_types: HashSet::new(),
            match_keywords: HashSet::new(),
        });
        entry.hit_types.insert(HitType::MetaSemantic);
        if boosted_score > entry.max_score {
            entry.max_score = boosted_score;
        }
    }

    let extact_words_match = !fts_result.is_empty();
    // 3. Process FTS results (keyword matches are precise, fixed high score)
    for item in fts_result {
        // +10 bonus
        let fts_boosted_score = (item.score.min(100) as usize + 10).min(100);
        let entry = file_map.entry(item.file_info.id).or_insert(SearchTmp {
            file_id: item.file_info.id,
            max_score: 0,
            chunk_ids: HashSet::new(),
            hit_types: HashSet::new(),
            match_keywords: HashSet::new(),
        });
        entry.hit_types.insert(HitType::ContentKeyword);
        for match_keyword in item.matched_keywords {
            log::debug!(
                "FTS Search - file_id: {}, keyword: {}",
                item.file_info.id,
                match_keyword
            );
            entry.match_keywords.insert(match_keyword);
        }
        for chunk_id in item.matched_chunk_ids {
            entry.chunk_ids.insert(chunk_id);
        }
        if fts_boosted_score > entry.max_score {
            entry.max_score = fts_boosted_score;
        }
    }

    // 4. With exact keyword matches, penalize documents missing the keyword
    if extact_words_match {
        for entry in file_map.values_mut() {
            // Absent from FTS results entirely
            if !entry.hit_types.contains(&HitType::ContentKeyword) {
                // Strategy: 60% score penalty (90 -> 54, 70 -> 42)
                // Yields top positions to results with exact keywords
                entry.max_score = (entry.max_score as f32 * 0.6) as usize;
            }
        }
    }

    let mut tmps: Vec<SearchTmp> = file_map.into_values().collect();
    tmps.sort_by(|a, b| b.max_score.cmp(&a.max_score));
    tmps.retain(|t| {
        t.max_score >= QUERY_MIN_SCORE || t.hit_types.contains(&HitType::ContentKeyword)
    });

    let file_ids: Vec<i64> = tmps.iter().map(|t| t.file_id).collect();
    let file_infos = file_info_repo::list_meta_by_ids(&file_ids).unwrap_or_default();
    if file_infos.is_empty() {
        log::debug!("file_infos is empty");
        return Vec::new();
    }

    let file_map: HashMap<i64, FileInfo> =
        file_infos.into_iter().map(|info| (info.id, info)).collect();
    // Return results in the sorted order
    tmps.into_iter()
        .filter_map(|tmp| {
            let file_id = tmp.file_id;
            let info = file_map.get(&file_id).cloned();
            if info.is_none() {
                return None;
            }
            Some(SearchResult {
                file_info: info.unwrap_or_default(),
                score: tmp.max_score,
                hit_types: tmp.hit_types.into_iter().collect(),
                matched_keywords: tmp.match_keywords.into_iter().collect(),
                matched_chunk_ids: tmp.chunk_ids.into_iter().collect(),
                similarity_type: None,
                source_device: None,
            })
        })
        .collect()
}
