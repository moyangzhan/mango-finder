pub mod fts_search_engine;
pub mod path_search_engine;
pub mod semantic_search_engine;

use crate::enums::QueryIntent;
use crate::structs::search_result::SearchResult;
use crate::utils::search_util;
use std::collections::HashMap;
use tokio::{task, try_join};

pub async fn keyword_search(query: &str) -> Vec<SearchResult> {
    if query.is_empty() {
        return Vec::new();
    }
    log::debug!("keyword_search query: {:?}", query);
    let query = query.to_owned();
    let (path_results, fts_results) = try_join!(
        task::spawn({
            let query = query.clone();
            async move { Ok(path_search_engine::search(&query).await) }
        }),
        task::spawn(async move { Ok(fts_search_engine::search(&query).await) }),
    )
    .map(|(path_res, fts_res)| {
        (
            path_res.unwrap_or_else(|_: Vec<SearchResult>| Vec::new()),
            fts_res.unwrap_or_else(|_: Vec<SearchResult>| Vec::new()),
        )
    })
    .unwrap_or_else(|_| (Vec::new(), Vec::new()));
    fuse_results(path_results, fts_results)
}

pub async fn semantic_search(query: &str) -> Vec<SearchResult> {
    semantic_search_engine::search(query).await
}

/// Dispatch by search type string ("keyword" → keyword search, anything else
/// → semantic). Single source of this dispatch, used by the GUI command, the
/// CLI in-process path and the /cli/search handler so all three behave alike.
/// 按搜索类型字符串分发("keyword" → 关键词,其余 → 语义)。此分发只维护
/// 一份,GUI 命令、CLI 进程内路径与 /cli/search handler 共用,行为一致。
pub async fn search_by_type(query: &str, search_type: Option<&str>) -> Vec<SearchResult> {
    match search_type {
        Some("keyword") => keyword_search(query).await,
        _ => semantic_search(query).await,
    }
}

pub async fn search_with_intent(query: &str) -> Vec<SearchResult> {
    let intent = search_util::detect_intent(query);
    match intent {
        QueryIntent::PathOnly => path_search_engine::search(query).await,

        QueryIntent::SemanticOnly => semantic_search_engine::search(query).await,

        QueryIntent::Hybrid => parallel_search(query).await,
    }
}

async fn parallel_search(query: &str) -> Vec<SearchResult> {
    let query = query.to_owned();
    let (path_results, semantic_results) = try_join!(
        task::spawn({
            let query = query.clone();
            async move { Ok(path_search_engine::search(&query).await) }
        }),
        task::spawn(async move { Ok(semantic_search_engine::search(&query).await) }),
    )
    .map(|(path_res, semantic_res)| {
        (
            path_res.unwrap_or_else(|_: Vec<SearchResult>| Vec::new()),
            semantic_res.unwrap_or_else(|_: Vec<SearchResult>| Vec::new()),
        )
    })
    .unwrap_or_else(|_| (Vec::new(), Vec::new()));

    fuse_results(path_results, semantic_results)
}

fn fuse_results(
    left_results: Vec<SearchResult>,
    right_results: Vec<SearchResult>,
) -> Vec<SearchResult> {
    let mut map: HashMap<String, SearchResult> = HashMap::new();
    for r in left_results {
        map.entry(r.file_info.path.clone()).or_insert(r);
    }
    for mut r in right_results {
        let matched_chunk_ids = r.matched_chunk_ids.clone();
        let matched_keywords = r.matched_keywords.clone();
        map.entry(r.file_info.path.clone())
            .and_modify(|e| {
                e.score = (e.score as f32 * 0.6 + r.score as f32 * 0.4).round() as usize;
                e.hit_types.append(&mut r.hit_types);
                e.matched_chunk_ids.extend(matched_chunk_ids.clone());
                e.matched_keywords.extend(matched_keywords.clone());
            })
            .or_insert_with(|| {
                let score = r.score;
                let file_info = r.file_info.clone();
                SearchResult {
                    file_info,
                    hit_types: r.hit_types.clone(),
                    matched_chunk_ids,
                    matched_keywords,
                    score: (score as f32 * 0.4).round() as usize,
                    similarity_type: None,
                    source_device: None,
                }
            });
    }
    let mut results: Vec<_> = map.into_values().collect();
    results.sort_by(|a, b| b.score.cmp(&a.score));
    results
}
