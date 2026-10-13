use crate::entities::FileInfo;
use crate::enums::{FileCategory, HitType};
use crate::global::DEFAULT_PAGE_SIZE;
use crate::global::{CONFIG_NAME_PATHS_CACHE_VERSION, PATHS_CACHE, PATHS_CACHE_BUILD_TIME};
use crate::repositories::{config_repo, file_info_repo};
use crate::structs::search_result::SearchResult;
use crate::utils::file_util;
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use anyhow::{Result, anyhow};
use chrono::Local;
use rayon::prelude::*;
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

/// Cache version (CONFIG_NAME_PATHS_CACHE_VERSION): any process deleting index rows bumps it;
/// the process holding the cache rebuilds fully on mismatch at the next 10s tick, so cross-process deletions (e.g. CLI index clear) reach the resident cache within ≤10s.
/// 缓存版本号(CONFIG_NAME_PATHS_CACHE_VERSION):任何进程删除索引行时递增;
/// 持有缓存的进程在计时器里发现版本不一致即全量重建,使跨进程删除(如 CLI index clear)在 ≤10s 内对常驻缓存生效
static CACHE_VERSION: AtomicI64 = AtomicI64::new(i64::MIN);
/// 全量重建互斥:timer 与 init 可能并发触发 build_index,不保护会互相 clear/push 交错产生重复条目
/// Full-rebuild mutex: timer and init may trigger build_index concurrently
static BUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 共享的一次性初始化:首个调用方建缓存并启动计时器,并发/后续调用等待同一次构建
/// Shared one-shot init: the first caller builds and starts the timer, concurrent/later callers await the same build
static INIT: OnceCell<()> = OnceCell::const_new();

fn read_db_version() -> Option<i64> {
    config_repo::get_val(CONFIG_NAME_PATHS_CACHE_VERSION)
        .trim()
        .parse()
        .ok()
}

/// Notify other processes that index rows were removed (the process holding the cache rebuilds)
/// 通知其他进程:索引行有删除,持有缓存的进程应重建(本进程本地版本同步跳过)
pub fn notify_paths_removed() {
    // Only bump the DB version; do NOT store the local version so the timer always detects the mismatch and rebuilds (storing it here would suppress the rebuild for callers that don't prune the local cache)
    if let Err(e) = config_repo::increment(CONFIG_NAME_PATHS_CACHE_VERSION) {
        log::warn!("Failed to bump paths cache version: {}", e);
    }
}

/// Keyword-based exact match search for file paths
pub async fn search(query: &str) -> Vec<SearchResult> {
    // Readers must never observe an unbuilt cache (e.g. racing the daemon's post-bind warm build)
    init().await;
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
                        // Real DB id carried by the cache entry — the old line-number fabrication pointed follow-up commands (file/similar by id) at the wrong rows.
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

/// Build the path cache on first use and start the refresh timer; concurrent callers share the same build, later calls are no-ops.
/// 首次调用构建路径缓存并启动刷新计时器;并发调用共享同一次构建,后续为空操作
pub async fn init() {
    INIT.get_or_init(|| async {
        build_index().await;
        tokio::spawn(paths_index_timer());
    })
    .await;
}
async fn build_index() {
    // Concurrency guard: skip if a rebuild is in flight (next tick retries on version mismatch)
    if BUILDING.swap(true, AtomicOrdering::SeqCst) {
        return;
    }
    // Record the pre-build version: a bump during the build makes the next tick mismatch and rebuild again (self-healing) instead of adopting a half-built state as current.
    let version_before = read_db_version().unwrap_or(0);
    let mut build_complete = true;
    let size = 1000;
    // Count before clearing: on count failure (e.g. SQLITE_BUSY → 0) keep the cache rather than clearing it to empty and exiting early
    let total = match file_info_repo::count() {
        Ok(n) => n,
        Err(e) => {
            log::error!("build_index: count failed: {}", e);
            BUILDING.store(false, AtomicOrdering::SeqCst);
            return; // keep cache and version; retry next tick
        }
    };
    {
        let mut cache = PATHS_CACHE.write().await;
        cache.clear();
    }
    if total == 0 {
        log::debug!("file_info total == 0");
        CACHE_VERSION.store(version_before, AtomicOrdering::SeqCst);
        BUILDING.store(false, AtomicOrdering::SeqCst);
        return;
    }
    let pages = (total + size - 1) / size;
    log::debug!("total: {}, pages: {}", total, pages);
    for page in 1..=pages {
        let paths = match file_info_repo::list_paths(page, size, true) {
            Ok(f) => f,
            Err(e) => {
                log::error!("Error reading page {}: {}", page, e);
                build_complete = false;
                break;
            }
        };
        if paths.is_empty() {
            log::debug!("no new paths");
            break; // normal: no more pages
        }
        let mut cache = PATHS_CACHE.write().await;
        for (id, path) in paths {
            if path.is_empty() {
                continue;
            }
            (*cache).push((id, path));
        }
    }
    // Mid-read error: leave version/build_time so the next tick retries on mismatch
    if !build_complete {
        BUILDING.store(false, AtomicOrdering::SeqCst);
        return;
    }
    CACHE_VERSION.store(version_before, AtomicOrdering::SeqCst);
    BUILDING.store(false, AtomicOrdering::SeqCst);
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
        // A version mismatch means another process removed index rows, which the append-only push cannot see — rebuild in full.
        if let Some(db_version) = read_db_version() {
            if db_version != CACHE_VERSION.load(AtomicOrdering::SeqCst) {
                log::info!("paths cache version changed, rebuilding");
                build_index().await;
                continue;
            }
        }
        push_to_index().await;
    }
}

pub async fn push_to_index() {
    let last_build_time = PATHS_CACHE_BUILD_TIME.read().await.clone();
    let total = file_info_repo::count_by_min_update_time(&last_build_time).unwrap_or_default();
    if total == 0 {
        return;
    }
    // Gather changed rows without the lock, then reconcile in one write-guard pass via a path-to-slot map (O(1) per row)
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
        // Locate via a path-to-slot map (cached entries refresh the id, new entries take sequential slots), then apply
        let ops: Vec<(usize, i64, String)> = {
            // Existing entries borrow &str from the guard; batch-new use owned keys
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
                // Cached: refresh the id (row may have been re-created)
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
    {
        let mut paths = PATHS_CACHE.write().await;
        if is_file {
            (*paths).retain(|(_, item)| item != path);
        } else {
            let directory = format!("{}{}", path, std::path::MAIN_SEPARATOR);
            (*paths).retain(|(_, item)| !item.starts_with(directory.as_str()));
        }
    }
    // Local cache pruned; bump version for other resident hosts
    notify_paths_removed();
}

pub async fn clear() {
    {
        let mut paths = PATHS_CACHE.write().await;
        paths.clear();
    }
    notify_paths_removed();
}
