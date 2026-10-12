use crate::entities::FileInfo;
use crate::enums::{FileCategory, HitType};
use crate::global::DEFAULT_PAGE_SIZE;
use crate::global::{PATHS_CACHE, PATHS_CACHE_BUILD_TIME};
use crate::repositories::file_info_repo;
use crate::structs::search_result::SearchResult;
use crate::utils::file_util;
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use anyhow::{Result, anyhow};
use chrono::Local;
use rayon::prelude::*;
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Keyword-based exact match search for file paths
pub async fn search(query: &str) -> Vec<SearchResult> {
    let start = Instant::now();
    let keywords: Vec<&str> = query.split_whitespace().collect();
    let Ok(automaton) = create_automaton(&keywords) else {
        return vec![];
    };
    let take_num = if keywords.len() == 1 {
        DEFAULT_PAGE_SIZE
    } else {
        DEFAULT_PAGE_SIZE * 10
    };
    let paths_cache = PATHS_CACHE.read().await;
    let mut result: Vec<SearchResult> = (*paths_cache)
        .par_iter()
        .filter_map(|(file_id, path)| {
            if path.is_empty() {
                return None;
            }
            //Scan for all keywords in the path
            let matches: SmallVec<[usize; 5]> = automaton
                .find_iter(path)
                .map(|m| m.pattern().as_usize())
                .collect();

            if matches.is_empty() {
                None
            } else {
                let (file_name, ext) = file_util::get_name_ext(path);
                let match_keywords = keywords
                    .iter()
                    .enumerate()
                    .filter_map(|(i, k)| {
                        if matches.contains(&i) {
                            Some(k.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                let result = SearchResult {
                    file_info: FileInfo {
                        // Real DB id carried by the cache entry — the old
                        // line-number fabrication pointed follow-up commands
                        // (file/similar by id) at the wrong rows.
                        // 缓存条目携带的真实 DB id —— 旧的行号伪造会让
                        // 后续按 id 的命令(file/similar)命中错误的行。
                        id: *file_id,
                        path: path.to_string(),
                        name: file_name,
                        category: FileCategory::from_ext(ext.as_str()).value(),
                        file_ext: ext,
                        ..Default::default()
                    },
                    score: matches.len(),
                    hit_types: vec![HitType::PathKeyword],
                    matched_keywords: match_keywords,
                    matched_chunk_ids: HashSet::new(),
                    similarity_type: None,
                    source_device: None,
                };
                Some(result)
            }
        })
        .take_any(take_num)
        .collect();
    result.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.file_info.id.cmp(&b.file_info.id))
    });
    result.truncate(DEFAULT_PAGE_SIZE);
    log::debug!("path search time: {:?}", start.elapsed());
    result
}

pub async fn init() {
    build_index().await;
    tokio::spawn(paths_index_timer());
}
async fn build_index() {
    let size = 1000;
    {
        let mut cache = PATHS_CACHE.write().await;
        cache.clear();
    }
    let total = file_info_repo::count().unwrap_or_default();
    if total == 0 {
        log::debug!("file_info total == 0");
        return;
    }
    let pages = (total + size - 1) / size;
    log::debug!("total: {}, pages: {}", total, pages);
    for page in 1..=pages {
        let paths = match file_info_repo::list_paths(page, size, true) {
            Ok(f) => f,
            Err(e) => {
                log::error!("Error reading page {}: {}", page, e);
                break;
            }
        };
        if paths.is_empty() {
            log::debug!("no new paths");
            break;
        }
        let mut cache = PATHS_CACHE.write().await;
        for (id, path) in paths {
            if path.is_empty() {
                continue;
            }
            (*cache).push((id, path));
        }
    }
    let mut guard = PATHS_CACHE_BUILD_TIME.write().await;
    *guard = Local::now()
}

pub fn create_automaton(keywords: &[&str]) -> Result<AhoCorasick> {
    AhoCorasickBuilder::new()
        .match_kind(MatchKind::LeftmostLongest)
        .ascii_case_insensitive(true)
        .build(keywords)
        .map_err(|e| anyhow!("create automaton error:{}", e))
}

// PATHS_CACHE only accept two events: Remove and Add, the [rename] event from fs_event is seperate into Remove and Add
async fn paths_index_timer() {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        interval.tick().await;
        push_to_index().await;
    }
}

pub async fn push_to_index() {
    let last_build_time = PATHS_CACHE_BUILD_TIME.read().await.clone();
    let total = file_info_repo::count_by_min_update_time(&last_build_time).unwrap_or_default();
    if total == 0 {
        return;
    }
    // Gather all changed rows first (no lock held), then reconcile in ONE
    // pass under a single write guard: a path→slot map makes each row O(1)
    // instead of a linear scan of the whole cache per row.
    // 先无锁取回全部变更行,再在单次写锁内一次性对账:path→槽位 map 让每行
    // O(1) 命中,而不是每行对全缓存线性扫描。
    let page_size = 1000;
    let pages = (total as f64 / page_size as f64).ceil() as u32;
    let mut incoming: Vec<(i64, String)> = Vec::new();
    for page in 1..=pages {
        let paths =
            file_info_repo::list_paths_by_min_update_time(&last_build_time, page as i64, page_size)
                .unwrap_or_default();
        if paths.is_empty() {
            log::debug!("no new paths");
            break;
        }
        incoming.extend(paths);
    }

    if !incoming.is_empty() {
        let mut paths_guard = PATHS_CACHE.write().await;
        // Plan under an immutable borrow (path → slot map), then apply after
        // the map is dropped — mutating .0 while holding &str keys into the
        // same guard would not borrow-check. New entries get a "future" slot
        // (>= current len) assigned in encounter order; since apply pushes in
        // op order, slot == len holds at every push, and a duplicate path in
        // the same batch resolves to its scheduled slot (last write wins).
        // 规划阶段持不可变借用(path → 槽位 map),落地前丢弃 map —— 持有指向
        // 同一 guard 的 &str 键同时改 .0 无法通过借用检查。新条目按出现顺序
        // 领取"未来"槽位(>= 当前长度);落地按 op 顺序入栈,每次 push 时
        // slot == len 恰好成立,同批次重复路径则落到已排定的槽位(后写胜)。
        let ops: Vec<(usize, i64, String)> = {
            // Existing entries keyed by guard-borrowed &str; batch-scheduled
            // new entries in a separate owned map (borrowing local Strings
            // into the guard-lifetime map would not borrow-check).
            // 既有条目以 guard 借用的 &str 为键;本批新排定的条目放在独立的
            // owned map(把局部 String 的引用插进 guard 生命周期的 map 无法
            // 通过借用检查)。
            let slot_of: HashMap<&str, usize> = paths_guard
                .iter()
                .enumerate()
                .map(|(i, (_, p))| (p.as_str(), i))
                .collect();
            let mut scheduled: HashMap<String, usize> = HashMap::new();
            let mut next_new_slot = paths_guard.len();
            incoming
                .into_iter()
                .map(|(id, path)| {
                    let slot = if let Some(&slot) = slot_of.get(path.as_str()) {
                        slot
                    } else if let Some(&slot) = scheduled.get(&path) {
                        slot
                    } else {
                        let slot = next_new_slot;
                        next_new_slot += 1;
                        scheduled.insert(path.clone(), slot);
                        slot
                    };
                    (slot, id, path)
                })
                .collect()
        };
        for (slot, id, path) in ops {
            if slot < paths_guard.len() {
                // Path already cached (or pushed earlier in this batch):
                // refresh its id in case the row was deleted and re-created
                // with a new id.
                // 路径已在缓存(或本批次早前已入栈):刷新其 id,防的是同路径
                // 行被删除后以新 id 重建。
                paths_guard[slot].0 = id;
            } else {
                paths_guard.push((id, path));
            }
        }
    }
    let mut guard = PATHS_CACHE_BUILD_TIME.write().await;
    *guard = Local::now();
}

// Handle delete path
pub async fn remove_from_index(path: &str, is_file: bool) {
    let mut paths = PATHS_CACHE.write().await;
    if is_file {
        (*paths).retain(|(_, item)| item != path);
    } else {
        let directory = format!("{}{}", path, std::path::MAIN_SEPARATOR);
        (*paths).retain(|(_, item)| !item.starts_with(directory.as_str()));
    }
}

pub async fn clear() {
    let mut paths = PATHS_CACHE.write().await;
    paths.clear();
}
