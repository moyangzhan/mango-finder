use clap::{Parser, Subcommand};
use local_ip_address::list_afinet_netifas;
use serde_json;
use std::net::UdpSocket;
use std::path::Path;
use std::time::Instant;

use mango_finder_lib::entities::FileInfo;
use mango_finder_lib::enums::FileCategory;
use mango_finder_lib::global::{
    ACTIVE_LOCALE, CLIENT_ID, DB_PATH, EMBEDDING_MODEL_PATH, EMBEDDING_TOKENIZER_PATH,
    FS_WATCHER_SETTING, STORAGE_PATH,
};
use mango_finder_lib::indexer_service;
use mango_finder_lib::repositories::{
    config_repo, device_repo, file_content_embedding_repo, file_content_fts_repo, file_info_repo,
    file_metadata_embedding_repo, indexing_task_repo,
};
use mango_finder_lib::searcher;
use mango_finder_lib::similarity::similarity_service;
use mango_finder_lib::structs::file_metadata::AudioType;

use crate::local_api;
use crate::output::{self, OutputKind};
use mango_finder_lib::structs::search_result::SearchResult;

#[derive(Parser)]
// 使用自定义的 Help 子命令（支持 [command] 参数），需禁用 clap 内置的 help 子命令，
// 否则 debug 构建会因子命令重名触发 clap 的 debug_asserts panic
#[command(
    name = "mfd",
    about = "Mango Finder Command Line Interface",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output format: json (default) or table
    #[arg(
        long,
        default_value = "json",
        env = "MANGO_FINDER_OUTPUT",
        global = true
    )]
    pub output: String,

    /// Suppress logs, only output result
    #[arg(long, env = "MANGO_FINDER_QUIET", global = true)]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Search documents
    Search {
        /// Search query
        #[arg(required_unless_present = "help_doc")]
        query: Option<String>,
        /// Search type: semantic (default), keyword
        // 值白名单:拼错直接报错,而不是静默改走语义搜索
        // Value whitelist: a typo fails fast instead of silently running a
        // semantic search
        #[arg(long, default_value = "semantic", value_parser = ["semantic", "keyword"])]
        r#type: String,
        /// Remote device ID (optional)
        #[arg(long)]
        device: Option<String>,
        /// Max results
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Show detailed help for this command
        #[arg(long = "help-doc")]
        help_doc: bool,
    },

    /// Find similar files by file ID, file name, or absolute path
    Similar {
        /// File ID, file name, or absolute path
        #[arg(required_unless_present = "help_doc")]
        file: Option<String>,
        /// Source device ID (optional)
        #[arg(long)]
        device: Option<String>,
        /// Max results
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Add an un-indexed external file to the index while querying
        #[arg(long)]
        index: bool,
        /// Show detailed help for this command
        #[arg(long = "help-doc")]
        help_doc: bool,
    },

    /// Index management
    Index {
        #[command(subcommand)]
        action: Option<IndexAction>,
        /// Show detailed help for this command
        #[arg(long = "help-doc")]
        help_doc: bool,
    },

    /// File operations
    File {
        /// File ID
        id: Option<i64>,
        /// Open file with system default program
        #[arg(long)]
        open: bool,
        /// Device ID (optional)
        #[arg(long)]
        device: Option<String>,
        /// Show detailed help for this command
        #[arg(long = "help-doc")]
        help_doc: bool,
    },

    /// Device management
    Device {
        #[command(subcommand)]
        action: Option<DeviceAction>,
        /// Show detailed help for this command
        #[arg(long = "help-doc")]
        help_doc: bool,
    },

    /// Show application status
    Status,

    /// Show version
    Version,

    /// Show help documentation
    Help {
        /// Command name (optional, shows full help if not specified)
        command: Option<String>,
    },

    /// Check system status and connectivity
    Check,

    /// Get or set locale
    Locale {
        /// Locale to set (e.g., zh-CN, en-US). If not specified, show current locale.
        value: Option<String>,
    },

    /// Shut down the local resident service (only stops the CLI daemon;
    /// a GUI-hosted service is left to the GUI)
    Shutdown,

    /// Run as the local service host in foreground (spawned on demand by mfd itself)
    #[command(hide = true)]
    Serve,
}

#[derive(Subcommand)]
pub enum IndexAction {
    /// Show index status
    Status,
    /// Start indexing (runs in foreground until done)
    Start {
        /// Paths to index
        paths: Vec<String>,
    },
    /// List indexed files
    List {
        #[arg(long, default_value = "1")]
        page: i64,
        #[arg(long, default_value = "20")]
        page_size: i64,
    },
    /// Clear all index
    Clear,
}

#[derive(Subcommand)]
pub enum DeviceAction {
    /// List online devices
    List,
}

pub async fn handle_search(query: &str, search_type: &str, device: Option<String>, limit: usize) {
    let start = Instant::now();

    if let Some(_device_id) = device {
        // TODO: 跨设备搜索
        output::print_error("Remote device search not yet implemented");
        return;
    }

    // Fast path: reuse the warm model of the local resident service (the GUI
    // or the on-demand daemon); on failure silently fall back to the
    // in-process search. Output structure is identical across both paths —
    // both resolve (results, total) here and share one success tail.
    // 快速路径:复用本机常驻服务(GUI 或按需拉起的 daemon)的热模型;
    // 失败静默回落进程内搜索。两条路径在此统一求出 (results, total),
    // 共用同一组装/输出尾段,结构天然一致
    let (results, total) = if let Some(port) = local_api::ensure_server().await {
        match local_api::search(query, search_type, limit, port).await {
            Ok((results, total)) => {
                log::info!("search served by local API (port {})", port);
                (results, total)
            }
            Err(e) => {
                log::warn!("local API search failed, fallback to in-process: {}", e);
                in_process_search(query, search_type, limit).await
            }
        }
    } else {
        in_process_search(query, search_type, limit).await
    };

    let elapsed = start.elapsed();
    let data = serde_json::json!({
        "results": results,
        "total": total,
        "returned": limit.min(total),
        "elapsed_ms": elapsed.as_millis()
    });

    output::print_success(OutputKind::Search, &data);
}

/// In-process search: full match set then apply limit.
/// The dispatch lives in `searcher::search_by_type` (shared with the server).
/// 进程内搜索:取全量匹配再应用 limit。分发逻辑在
/// `searcher::search_by_type`(与服务端共用)。
async fn in_process_search(
    query: &str,
    search_type: &str,
    limit: usize,
) -> (Vec<SearchResult>, usize) {
    // 进程内关键词搜索前确保路径缓存已构建:该缓存原由 GUI 的 ui_mounted
    // 初始化,CLI 进程不经过它 —— 不构建则路径匹配半边恒为空(与 GUI 结果
    // 不一致的预存在问题,此处顺带修复)
    // Ensure the path cache is built before in-process keyword search: it is
    // initialized by the GUI's ui_mounted, which a CLI process never runs —
    // without it the path-match half of results is always empty (a
    // pre-existing divergence from GUI results, fixed here as well).
    if search_type == "keyword" {
        searcher::path_search_engine::init().await;
    }
    let results = searcher::search_by_type(query, Some(search_type)).await;
    let total = results.len();
    (results.into_iter().take(limit).collect(), total)
}

/// Similarity query: prefer the local resident service (warm model), fall
/// back to in-process; both paths call the same underlying function, so the
/// output is identical.
/// 相似查询:优先走本机常驻服务(热模型),回落进程内;
/// 两条路径底层调用相同函数,输出一致
async fn find_similars(file_id: i64, limit: usize) -> Result<Vec<SearchResult>, String> {
    if let Some(port) = local_api::ensure_server().await {
        match local_api::similars(file_id, limit, port).await {
            Ok(results) => {
                log::info!("similar served by local API (port {})", port);
                return Ok(results);
            }
            Err(e) => {
                // Keep the fallback reason visible, mirroring handle_search
                // 与 handle_search 一致,保留回落原因日志
                log::warn!("local API similar failed, fallback to in-process: {}", e);
            }
        }
    }
    similarity_service::find_similars_for_local_file(file_id, limit).await
}

pub async fn handle_shutdown() {
    let Some(port) = local_api::detect().await else {
        let data = serde_json::json!({ "message": "No running local service" });
        output::print_success(OutputKind::Shutdown, &data);
        return;
    };

    // Ignore the request result and let the poll decide the final state:
    // the daemon may exit between detect and POST (idle timeout / a
    // concurrent yield), which surfaces as a transport error although the
    // service actually stopped.
    // 忽略请求结果、由轮询决定最终状态:daemon 可能在 detect 与 POST 之间
    // 已退出(空闲超时/并发让位),表现为传输错误但服务实际已停止。
    let _ = local_api::request_shutdown(port).await;

    // Poll for the real exit: the daemon exits within ~1s (graceful drain),
    // a GUI host keeps answering. Require two consecutive unreachable probes
    // so a momentary slow /ping (busy GUI) is not misread as "stopped";
    // after the deadline it is a GUI host.
    // 轮询真实退出:daemon 约 1s 内退出(优雅排水),GUI 宿主持续应答。
    // 需连续两次探测不通,避免 GUI 忙碌导致单次 ping 超时被误判为已停止;
    // 超时仍未退出则判定为 GUI 宿主
    // Daemon drain is bounded at 10s; allow 12s before declaring a GUI host
    let deadline = Instant::now() + std::time::Duration::from_secs(12);
    let mut consecutive_gone = 0;
    let (status, message) = loop {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        if local_api::detect_port(port).await.is_none() {
            consecutive_gone += 1;
            if consecutive_gone >= 2 {
                break ("stopped", "Local service (daemon) stopped");
            }
        } else {
            consecutive_gone = 0;
        }
        if Instant::now() >= deadline {
            break (
                "running",
                "Service is hosted by the GUI; it stops when the GUI exits",
            );
        }
    };
    let data = serde_json::json!({ "status": status, "message": message });
    output::print_success(OutputKind::Shutdown, &data);
}

pub async fn handle_similar(file: &str, device: Option<String>, limit: usize, index: bool) {
    if let Some(_device_id) = device {
        // TODO: 跨设备相似搜索
        output::print_error("Remote device similarity search not yet implemented");
        return;
    }

    let target = match resolve_file_arg(file) {
        Ok(target) => target,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let start = Instant::now();
    match target {
        SimilarTarget::Indexed {
            file,
            same_name_matches,
        } => {
            match find_similars(file.id, limit).await {
                Ok(results) => {
                    let elapsed = start.elapsed();
                    let mut data = serde_json::json!({
                        "mode": "indexed",
                        "file": {
                            "id": file.id,
                            "name": file.name,
                            "path": file.path,
                        },
                        "results": results,
                        "total": results.len(),
                        "elapsed_ms": elapsed.as_millis()
                    });
                    if same_name_matches.len() > 1 {
                        // 同名文件取 id 最小的一个，其余命中项列出供用户改用 ID 或路径重试
                        data["notice"] = serde_json::json!(format!(
                            "{} files share the name '{}'; using id={} ({}). Pass a file ID or absolute path to pick another one.",
                            same_name_matches.len(),
                            file.name,
                            file.id,
                            file.path
                        ));
                        // 列表用于"换一个文件重试",排除已选中的那条,
                        // 免得它占掉 5 个名额之一
                        // The list exists to retry with ANOTHER file; exclude
                        // the chosen entry so it does not waste one of the 5
                        // slots.
                        data["matched_files"] = serde_json::json!(
                            same_name_matches
                                .iter()
                                .filter(|f| f.id != file.id)
                                .take(5)
                                .map(|f| serde_json::json!({"id": f.id, "path": f.path}))
                                .collect::<Vec<_>>()
                        );
                    }
                    output::print_success(OutputKind::Similar, &data);
                }
                Err(e) => {
                    output::print_error(&format!("Failed to find similar files: {}", e));
                }
            }
        }
        SimilarTarget::External { path, name } => {
            warn_external_slowness(&path);

            // --index：先单文件入库（解析与嵌入只做一次），再走已索引快路径查询；
            // 入库失败时回退纯现场计算并提示
            if index {
                // 入库前规范化为绝对路径（Windows 下统一 \），与目录扫描产生的行格式一致，
                // 避免监听增删、路径比较按字符串匹配时失效
                let stored_path = normalize_store_path(&path);
                let indexed_file = match indexer_service::index_file(&stored_path).await {
                    Ok(()) => file_info_repo::get_by_path(&stored_path).ok().flatten(),
                    Err(e) => {
                        log::warn!("Failed to index '{}': {}", stored_path, e);
                        None
                    }
                };
                if let Some(file) = indexed_file {
                    // 视频/其他类别只入库元数据、永不嵌入,相似查询恒为空;
                    // 给出与无 --index 时一致的明确报错,而不是"已入库"的空结果
                    // Video/Other categories only get a metadata row and are
                    // never embedded (similarity is always empty); fail with
                    // the same clear error as without --index instead of an
                    // empty "indexed" success.
                    if !matches!(
                        FileCategory::from_ext(&file.file_ext),
                        FileCategory::Document | FileCategory::Image | FileCategory::Audio
                    ) {
                        output::print_error(&format!(
                            "On-the-fly similarity is not supported for {} files",
                            file.file_ext
                        ));
                        return;
                    }
                    match find_similars(file.id, limit).await {
                        Ok(results) => {
                            let elapsed = start.elapsed();
                            let mut data = serde_json::json!({
                                "mode": "external",
                                "indexed": true,
                                "file": {
                                    "id": file.id,
                                    "name": file.name,
                                    "path": file.path,
                                },
                                "results": results,
                                "total": results.len(),
                                "elapsed_ms": elapsed.as_millis()
                            });
                            data["notice"] = serde_json::json!(build_indexed_notice(&file).await);
                            output::print_success(OutputKind::Similar, &data);
                            return;
                        }
                        Err(e) => {
                            output::print_error(&format!("Failed to find similar files: {}", e));
                            return;
                        }
                    }
                }
                run_external_on_the_fly(
                    &path,
                    &name,
                    limit,
                    start,
                    Some("Note: the file was not added to the index (excluded by indexer settings, or the insert failed; see logs); the result was computed on the fly instead.".to_string()),
                )
                .await;
            } else {
                run_external_on_the_fly(&path, &name, limit, start, None).await;
            }
        }
    }
}

/// 现场计算并输出结果；extra_notice 用于附加提示（如 --index 入库失败的说明）
async fn run_external_on_the_fly(
    path: &str,
    name: &str,
    limit: usize,
    start: Instant,
    extra_notice: Option<String>,
) {
    match similarity_service::find_similars_for_external_file(path, limit).await {
        Ok(outcome) => {
            let elapsed = start.elapsed();
            let mut data = serde_json::json!({
                "mode": "external",
                "file": {
                    "id": serde_json::Value::Null,
                    "name": name,
                    "path": path,
                },
                "chunk_count": outcome.chunk_count,
                "results": outcome.results,
                "total": outcome.results.len(),
                "elapsed_ms": elapsed.as_millis()
            });
            let mut notice = build_external_notice(path, &outcome);
            if let Some(extra) = extra_notice {
                notice = format!("{} {}", notice, extra);
            }
            data["notice"] = serde_json::json!(notice);
            output::print_success(OutputKind::Similar, &data);
        }
        Err(e) => {
            output::print_error(&format!("Failed to find similar files: {}", e));
        }
    }
}

/// --index 入库成功后的提示：说明已入库与快路径，路径不在监听目录时补充变更不自动更新的说明
async fn build_indexed_notice(file: &FileInfo) -> String {
    let mut notice = format!(
        "'{}' was not indexed; it has now been added to the index (id={}). Future queries on it use the fast path.",
        file.path, file.id
    );
    if !in_watched_directory(&file.path).await {
        notice.push_str(" Note: this path is not in a watched directory; changes to the file will not be re-indexed automatically.");
    }
    notice
}

/// 规范化为存储路径格式：绝对路径，Windows 下分隔符统一为反斜杠，
/// 与目录扫描写入 file_info 的行格式保持一致
/// Canonicalize an existing path to its on-disk case. Windows paths are
/// case-insensitive but the index matches paths with exact SQL comparisons,
/// so a differently-cased input must be resolved to the indexed form first
/// (both for lookup and before inserting via --index, which would otherwise
/// create a duplicate row).
/// 将存在的路径规范化为磁盘上的真实大小写。Windows 路径不区分大小写,但索引
/// 按字符串精确匹配;大小写不同的输入必须先解析为索引中的形态(查找与
/// --index 入库两侧都需要,否则会插入重复行)。
fn canonical_case_path(path: &str) -> Option<String> {
    let p = Path::new(path);
    if !p.exists() {
        return None;
    }
    let canon = std::fs::canonicalize(p).ok()?;
    let s = canon.to_string_lossy();
    // Windows canonicalize 的 verbatim 前缀与索引行格式不一致,需转换:
    // \\?\UNC\server\share → \\server\share,其余 \\?\ 前缀直接剥掉
    // Windows canonicalize's verbatim prefixes do not match indexed rows:
    // \\?\UNC\server\share → \\server\share; the plain \\?\ is stripped.
    let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else {
        s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
    };
    if std::path::MAIN_SEPARATOR == '\\' {
        Some(s.replace('/', "\\"))
    } else {
        Some(s)
    }
}

fn normalize_store_path(path: &str) -> String {
    canonical_case_path(path).unwrap_or_else(|| {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
        let normalized = absolute.to_string_lossy().into_owned();
        if std::path::MAIN_SEPARATOR == '\\' {
            normalized.replace('/', "\\")
        } else {
            normalized
        }
    })
}

/// Prefix comparison that matches Windows path semantics: case-insensitive on
/// Windows (full Unicode folding — watched dirs may contain non-ASCII),
/// exact on Unix. Byte-slicing is guarded by is_char_boundary so CJK paths
/// cannot panic.
/// 前缀比较按各平台路径语义:Windows 不区分大小写(全 Unicode 折叠 —— 监听
/// 目录可能含非 ASCII 字符),Unix 精确。字节切片先检查 char 边界,
/// 中文等多字节路径不会 panic。
fn path_has_prefix(path: &str, prefix: &str) -> bool {
    #[cfg(windows)]
    {
        let n = prefix.len();
        path.is_char_boundary(n) && path[..n].to_lowercase() == prefix.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        let _ = prefix;
        path.starts_with(prefix)
    }
}

/// 判断路径是否位于任一监听目录之下（含目录本身；分隔符感知，避免同前缀兄弟目录误判；
/// Windows 下大小写不敏感）
/// Path equality with platform semantics (case-insensitive on Windows).
/// 按平台语义的路径相等比较(Windows 不区分大小写)。
fn path_equals(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.to_lowercase() == b.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Whether the path is under any watched directory (inclusive; separator-aware
/// to avoid sibling-directory false positives; case-insensitive on Windows).
async fn in_watched_directory(path: &str) -> bool {
    let Some(parent) = Path::new(path).parent().and_then(|p| p.to_str()) else {
        return false;
    };
    let setting = FS_WATCHER_SETTING.read().await;
    // 首个分支必须是相等而非前缀:E:\docs2 会以 E:\docs 开头但不是其子目录
    // The first arm must be equality, not prefix: E:\docs2 starts with
    // E:\docs but is not inside it.
    setting.directories.iter().any(|dir| {
        path_equals(parent, dir)
            || path_has_prefix(parent, &format!("{}{}", dir, std::path::MAIN_SEPARATOR))
            || path_has_prefix(parent, &format!("{}/", dir))
    })
}

enum SimilarTarget {
    /// 已索引文件（file 为解析结果；same_name_matches 仅按文件名解析且存在多个同名文件时非空）
    Indexed {
        file: FileInfo,
        same_name_matches: Vec<FileInfo>,
    },
    /// 磁盘上存在但未索引的外部文件，走现场计算
    External { path: String, name: String },
}

/// 解析 similar 命令的文件参数：纯数字按文件 ID；含路径分隔符按绝对路径精确匹配（未命中但文件存在时转现场计算）；其余按文件名精确匹配
fn resolve_file_arg(arg: &str) -> Result<SimilarTarget, String> {
    // 纯数字优先按 ID 解析（文件名恰为纯数字的情况需改用路径指定）
    if let Ok(id) = arg.parse::<i64>() {
        return match file_info_repo::get_by_id(id) {
            Ok(Some(file)) => Ok(SimilarTarget::Indexed {
                file,
                same_name_matches: Vec::new(),
            }),
            Ok(None) => Err(format!(
                "File not found by id: {}. Numeric input is treated as a file ID; pass a file name or absolute path instead.",
                arg
            )),
            Err(e) => Err(format!("Failed to get file by id: {}", e)),
        };
    }

    if arg.contains('\\') || arg.contains('/') {
        // Windows 路径大小写不敏感而索引按字符串精确匹配:
        // 先解析为磁盘真实大小写,再尝试原样输入与翻转分隔符
        // (Windows 下索引中的路径为 \,而输入常为 /)
        // Windows paths are case-insensitive while the index matches exactly:
        // resolve to the on-disk case first, then try the input as-is and
        // the separator-flipped form (indexed paths use \, input often /).
        let canonical = canonical_case_path(arg);
        let flipped: String = arg
            .chars()
            .map(|c| match c {
                '\\' => '/',
                '/' => '\\',
                _ => c,
            })
            .collect();
        let mut candidates: Vec<&str> = Vec::new();
        if let Some(c) = canonical.as_deref() {
            candidates.push(c);
        }
        candidates.push(arg);
        candidates.push(flipped.as_str());
        for candidate in candidates {
            match file_info_repo::get_by_path(candidate) {
                Ok(Some(file)) => {
                    return Ok(SimilarTarget::Indexed {
                        file,
                        same_name_matches: Vec::new(),
                    });
                }
                Ok(None) => {}
                Err(e) => return Err(format!("Failed to get file by path: {}", e)),
            }
        }
        // 未索引但磁盘上存在的文件转现场计算（不写索引，速度取决于文件类型）
        let path = Path::new(arg);
        if path.is_dir() {
            return Err(format!("Path is a directory, not a file: {}", arg));
        }
        if path.exists() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            return Ok(SimilarTarget::External {
                path: arg.to_string(),
                name,
            });
        }
        return Err(format!("File not found by path: {}", arg));
    }

    match file_info_repo::list_by_name(arg) {
        Ok(matches) if matches.is_empty() => Err(format!(
            "File not found by name: {}. Bare names only match indexed files; pass an absolute path to query an un-indexed file.",
            arg
        )),
        Ok(matches) => {
            let file = matches[0].clone();
            Ok(SimilarTarget::Indexed {
                file,
                same_name_matches: matches,
            })
        }
        Err(e) => Err(format!("Failed to get file by name: {}", e)),
    }
}

/// 现场计算前按文件类别向 stderr 提示耗时预期（--quiet 时不输出）
fn warn_external_slowness(path: &str) {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match FileCategory::from_ext(&ext) {
        FileCategory::Document => {
            log::warn!(
                "'{}' is not indexed; parsing and embedding it on the fly. Model load plus per-chunk inference may make this slow for large files.",
                path
            );
        }
        FileCategory::Image => {
            log::warn!(
                "'{}' is not indexed; computing its perceptual hash and caption embedding on the fly (model load or a remote call may take a few seconds).",
                path
            );
        }
        FileCategory::Audio => {
            log::warn!(
                "'{}' is not indexed; fingerprinting or transcribing it on the fly. Transcription of long audio may take minutes.",
                path
            );
        }
        _ => {}
    }
}

/// 现场计算完成后按实际处理路径组装结果中的提示文案
fn build_external_notice(
    path: &str,
    outcome: &similarity_service::ExternalSimilarOutcome,
) -> String {
    let index_tip = "Index the directory with 'mfd index start <dir>' to make repeat queries fast.";
    let what = match outcome.audio_type {
        Some(AudioType::Music | AudioType::Mixed) => {
            "its audio fingerprint was extracted on the fly".to_string()
        }
        Some(_) => format!(
            "it was transcribed and {} chunk(s) were embedded on the fly",
            outcome.chunk_count
        ),
        None if outcome.category == FileCategory::Image => {
            "its perceptual hash and caption embedding were computed on the fly".to_string()
        }
        None => format!(
            "it was parsed and {} chunk(s) were embedded on the fly",
            outcome.chunk_count
        ),
    };
    format!(
        "'{}' is not indexed; {}. Nothing was written to the index. {}",
        path, what, index_tip
    )
}

pub async fn handle_index(action: IndexAction) {
    match action {
        IndexAction::Status => {
            let total = file_info_repo::count().unwrap_or(0);
            // 与 GUI 同口径:按 content_index_status 统计已索引文件数,
            // 而非 embedding 分块行数(一个 30 块的 PDF 曾显示 indexed>total)
            // Same metric as the GUI: distinct files with content_index_status
            // = Indexed, not the embedding-chunk row count (a 30-chunk PDF
            // used to show indexed > total).
            let indexed = count_indexed_files();

            // 索引任务状态持久化在数据库中，跨进程可见；
            // 进程内的原子状态（INDEXING 等）对一次性的 CLI 进程没有意义
            let recent_tasks = indexing_task_repo::list(1, 5, "id", "desc").unwrap_or_default();
            let latest_task = recent_tasks.first();

            let data = serde_json::json!({
                "total_files": total,
                "indexed_files": indexed,
                "latest_task": latest_task,
                "recent_tasks": recent_tasks
            });
            output::print_success(OutputKind::IndexStatus, &data);
        }
        IndexAction::Start { paths } => {
            if paths.is_empty() {
                output::print_error("No paths specified");
                return;
            }

            // CLI 是一次性进程：后台 spawn 的任务会随进程退出被终止，
            // 因此这里同步执行索引并输出最终结果。进度日志输出到 stderr，
            // 需要中途停止时按 Ctrl+C 终止进程。
            // 入参路径统一规范化为绝对路径(与磁盘真实大小写一致),
            // 否则相对路径会以相对形式入库,后续按路径的查询/打开全部失效
            // Normalize every input path to its absolute on-disk form —
            // relative arguments would otherwise be stored relative and break
            // every later path-based lookup/open.
            let paths: Vec<String> = paths.iter().map(|p| normalize_store_path(p)).collect();
            log::info!("Indexing started for paths: {:?}", paths);
            match indexer_service::start_indexing(paths.clone(), "cli").await {
                Ok(true) => {
                    let data = serde_json::json!({
                        "message": "Indexing finished",
                        "paths": paths,
                        "tip": "Use 'mfd index status' to see indexed file counts"
                    });
                    output::print_success(OutputKind::IndexStart, &data);
                }
                Ok(false) => {
                    // skipped: a scan/index task is already active in this process
                    let data = serde_json::json!({
                        "message": "Indexing skipped: another indexing task is active",
                        "paths": paths
                    });
                    output::print_success(OutputKind::IndexStart, &data);
                }
                Err(e) => {
                    output::print_error(&format!("Indexing failed: {}", e));
                }
            }
        }
        IndexAction::List { page, page_size } => {
            // 分页钳制:负 page_size 会成为 SQLite 的无上限 LIMIT(整表倾倒),
            // 非正 page 的负 OFFSET 被 SQLite 当 0(静默重复第 1 页)
            // Clamp pagination: a negative page_size becomes SQLite's
            // unbounded LIMIT (full-table dump); a non-positive page yields a
            // negative OFFSET that SQLite treats as 0 (repeats page 1).
            let page = page.clamp(1, 1_000_000); // also caps (page-1)*size overflow
            let page_size = page_size.clamp(1, 1000);
            match file_info_repo::list(page, page_size) {
                Ok(files) => {
                    let data = serde_json::json!({
                        "files": files,
                        "page": page,
                        "page_size": page_size
                    });
                    output::print_success(OutputKind::IndexList, &data);
                }
                Err(e) => {
                    output::print_error(&format!("Failed to list files: {}", e));
                }
            }
        }
        IndexAction::Clear => {
            // 与 GUI 的 clear_index 对齐:除四张表外,还要清路径缓存与磁盘上的
            // parsed_documents / extracted_images,否则 CLI-only 工作流会无限堆积
            // 磁盘残留
            // Mirror the GUI's clear_index: besides the four tables, also
            // clear the path cache and the on-disk parsed_documents /
            // extracted_images, otherwise CLI-only workflows accumulate disk
            // leftovers forever.
            let clear_result = (|| -> Result<(), String> {
                file_content_fts_repo::clear().map_err(|e| e.to_string())?;
                file_content_embedding_repo::clear().map_err(|e| e.to_string())?;
                file_metadata_embedding_repo::clear().map_err(|e| e.to_string())?;
                file_info_repo::clear().map_err(|e| e.to_string())?;
                Ok(())
            })();
            if clear_result.is_ok() {
                searcher::path_search_engine::clear().await;
                if let Some(storage) = STORAGE_PATH.get() {
                    for dir in ["parsed_documents", "extracted_images"] {
                        let dir_path = std::path::Path::new(storage).join(dir);
                        if dir_path.exists() {
                            let _ = std::fs::remove_dir_all(&dir_path);
                        }
                    }
                }
            }
            match clear_result {
                Ok(()) => {
                    let data = serde_json::json!({
                        "message": "Index cleared"
                    });
                    output::print_success(OutputKind::IndexClear, &data);
                }
                Err(e) => {
                    output::print_error(&format!("Failed to clear index: {}", e));
                }
            }
        }
    }
}

pub async fn handle_file(id: i64, open: bool, device: Option<String>) {
    if let Some(_device_id) = device {
        // TODO: 从远程设备获取文件
        output::print_error("Remote device file fetch not yet implemented");
        return;
    }

    match file_info_repo::get_by_id(id) {
        Ok(Some(mut file)) => {
            // 与 GUI/cluster 读取路径一致:file 存储模式下 content 为空、
            // 正文在 content_ref_path 指向的 .md 里,需读回填充
            // Same as the GUI/cluster read paths: under file storage mode the
            // content column is empty and the parsed text lives in the .md
            // referenced by content_ref_path — read it back.
            if file.content.is_empty() {
                if let (Some(rel), Some(storage)) =
                    (file.content_ref_path.as_deref(), STORAGE_PATH.get())
                {
                    let md_path = std::path::Path::new(storage).join(rel);
                    file.content = std::fs::read_to_string(&md_path).unwrap_or_default();
                }
            }
            if open {
                if !Path::new(&file.path).exists() {
                    output::print_error(&format!("File path no longer exists: {}", file.path));
                    return;
                }
                if let Err(e) = open_with_system(&file.path) {
                    output::print_error(&e);
                    return;
                }
            }
            output::print_success(OutputKind::File, &serde_json::to_value(&file).unwrap());
        }
        Ok(None) => {
            output::print_error(&format!("File not found: {}", id));
        }
        Err(e) => {
            output::print_error(&format!("Failed to get file: {}", e));
        }
    }
}

pub async fn handle_device(action: DeviceAction) {
    match action {
        DeviceAction::List => match device_repo::list() {
            Ok(devices) => {
                let data = serde_json::json!({
                    "devices": devices,
                    "total": devices.len()
                });
                output::print_success(OutputKind::DeviceList, &data);
            }
            Err(e) => {
                output::print_error(&format!("Failed to list devices: {}", e));
            }
        },
    }
}

/// Count distinct indexed files (content_index_status = Indexed) across all
/// categories, matching the GUI's metric.
/// 按类别累计已索引文件数(content_index_status = Indexed),与 GUI 口径一致。
fn count_indexed_files() -> i64 {
    file_info_repo::count_indexed().unwrap_or(0)
}

pub async fn handle_status() {
    let total_files = file_info_repo::count().unwrap_or(0);
    let indexed_files = count_indexed_files();
    let locale = current_locale();

    let data = serde_json::json!({
        "total_files": total_files,
        "indexed_files": indexed_files,
        "locale": locale,
        "version": env!("CARGO_PKG_VERSION")
    });

    output::print_success(OutputKind::Status, &data);
}

pub fn handle_version() {
    let data = serde_json::json!({
        "name": "mfd",
        "version": env!("CARGO_PKG_VERSION")
    });

    output::print_success(OutputKind::Version, &data);
}

pub fn handle_locale(value: Option<String>) {
    match value {
        Some(locale) => {
            // 设置新的 locale:合法值取自 enums::Locale,与 GUI 校验同源
            // Set a new locale: valid values come from enums::Locale, the
            // same source the GUI validates against.
            let valid = locale == mango_finder_lib::enums::Locale::ZhCn.text()
                || locale == mango_finder_lib::enums::Locale::EnUs.text();
            if !valid {
                output::print_error(&format!(
                    "Invalid locale '{}'. Valid values: {}, {}",
                    locale,
                    mango_finder_lib::enums::Locale::ZhCn.text(),
                    mango_finder_lib::enums::Locale::EnUs.text()
                ));
                return;
            }

            // 更新全局变量(锁毒化时跳过全局写入,不 panic)
            // Update the global (skip on a poisoned lock instead of panicking)
            if let Ok(mut guard) = ACTIVE_LOCALE.write() {
                *guard = locale.clone();
            }
            rust_i18n::set_locale(locale.as_str());

            // 更新数据库
            if let Err(e) = config_repo::update_by_name("active_locale", &locale) {
                output::print_error(&format!("Failed to save locale: {}", e));
                return;
            }

            let data = serde_json::json!({
                "locale": locale,
                "message": "Locale updated successfully"
            });
            output::print_success(OutputKind::LocaleSet, &data);
        }
        None => {
            // 显示当前 locale
            let locale = current_locale();

            let data = serde_json::json!({
                "locale": locale
            });
            output::print_success(OutputKind::LocaleGet, &data);
        }
    }
}

pub async fn handle_check() {
    // 获取本地 IP
    let local_ip = get_local_ip_address();

    // 获取客户端 ID
    let client_id = CLIENT_ID
        .read()
        .map(|id| id.clone())
        .unwrap_or_else(|_| "unknown".to_string());

    // 获取索引文件数量:与 status/index status 同口径(content_index_status),
    // 否则"已扫描未嵌入"状态下 check 的 no-indexed-files 建议不会触发
    // Same indexed-files metric as status/index status (content_index_status);
    // otherwise check's no-indexed-files recommendation never fires for
    // scanned-but-never-embedded libraries.
    let indexed_files = count_indexed_files();
    let total_embeddings = file_content_embedding_repo::count().unwrap_or(0);

    // 集群设置:走与真实服务一致的类型化来源(端口来源/默认值均一致),
    // 超出合法范围的端口回退默认值,避免 as u16 截断后探测错误端口
    // Cluster setting: use the same typed source as the real service (same
    // port origin/default); out-of-range values fall back instead of an
    // `as u16` truncation probing the wrong port.
    let setting = mango_finder_lib::cluster::get_cluster_setting().await;
    let device_name = if setting.device_name.is_empty() {
        "unknown"
    } else {
        setting.device_name.as_str()
    };
    let cluster_enabled = setting.enabled;
    // local_port() already applies the valid-range clamp (single source)
    // local_port() 已含合法区间钳制(单一来源)
    let port: u16 = local_api::local_port().await as u16;

    // 端口检查:若本机服务(GUI/daemon)正持有该端口,它是健康状态而非占用问题
    // Port check: a port held by our own local service (GUI/daemon) is a
    // healthy state, not an occupancy problem.
    let own_service = local_api::detect().await.is_some();
    let port_available = if own_service {
        true
    } else {
        mango_finder_lib::cluster::http_server::is_port_available(port as i32).await
    };

    // 获取网络接口信息
    let network_interfaces = get_network_interfaces();

    // 获取存储路径
    let storage_path = STORAGE_PATH
        .get()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // 获取数据库路径
    let db_path = DB_PATH
        .get()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // 检查模型配置
    let model_status = check_model_status();

    // 获取已配对设备数量
    let paired_devices = get_paired_device_count();

    // 构建检查结果
    let check = serde_json::json!({
        "system": {
            "version": env!("CARGO_PKG_VERSION"),
            "client_id": client_id,
            "device_name": device_name,
        },
        "network": {
            "local_ip": local_ip,
            "interfaces": network_interfaces,
            "port": port,
            "port_available": port_available,
        },
        "storage": {
            "path": storage_path,
            "db_path": db_path,
            "indexed_files": indexed_files,
            "total_embeddings": total_embeddings,
        },
        "cluster": {
            "enabled": cluster_enabled,
            "paired_devices": paired_devices,
        },
        "ai_model": model_status,
        "recommendations": get_recommendations(&local_ip, port, port_available, cluster_enabled, indexed_files, paired_devices)
    });

    output::print_success(OutputKind::Check, &check);
}

fn get_local_ip_address() -> Option<String> {
    // 与集群广播同源(cluster::http_client::get_local_ip,mDNS 广播用的
    // 就是它),保证 check 报告的 IP 与 GUI 对外通告的一致
    // Same source as the cluster broadcast (cluster::http_client::get_local_ip
    // — what mDNS advertises), so check reports the IP the GUI announces.
    if let Ok(ip) = mango_finder_lib::cluster::http_client::get_local_ip() {
        return Some(ip);
    }

    // 方法1：通过 UDP 连接获取最可能的本地 IP
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return Some(addr.ip().to_string());
            }
        }
    }

    // 方法2：从网络接口列表获取第一个 IPv4 地址
    if let Ok(interfaces) = list_afinet_netifas() {
        for (_, ip) in interfaces {
            if ip.is_ipv4() && !ip.is_loopback() {
                return Some(ip.to_string());
            }
        }
    }

    None
}

fn get_network_interfaces() -> Vec<serde_json::Value> {
    let mut interfaces = Vec::new();

    if let Ok(network_interfaces) = list_afinet_netifas() {
        for (name, ip) in network_interfaces {
            if ip.is_ipv4() {
                interfaces.push(serde_json::json!({
                    "name": name,
                    "ip": ip.to_string()
                }));
            }
        }
    }

    interfaces
}

fn check_model_status() -> serde_json::Value {
    let embedding_model = EMBEDDING_MODEL_PATH
        .get()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let tokenizer = EMBEDDING_TOKENIZER_PATH
        .get()
        .map(|s| s.to_string())
        .unwrap_or_default();

    let embedding_exists = !embedding_model.is_empty() && Path::new(&embedding_model).exists();
    let tokenizer_exists = !tokenizer.is_empty() && Path::new(&tokenizer).exists();

    serde_json::json!({
        "embedding_model": {
            "path": embedding_model,
            "exists": embedding_exists,
        },
        "tokenizer": {
            "path": tokenizer,
            "exists": tokenizer_exists,
        },
        "ready": embedding_exists && tokenizer_exists
    })
}

fn get_paired_device_count() -> i64 {
    match device_repo::count_paired() {
        Ok(count) => count,
        Err(_) => 0,
    }
}

fn get_recommendations(
    local_ip: &Option<String>,
    port: u16,
    port_available: bool,
    cluster_enabled: bool,
    indexed_files: i64,
    paired_devices: i64,
) -> Vec<String> {
    let mut recommendations = Vec::new();

    if local_ip.is_none() {
        recommendations.push(t!("cli.check.no-ip").to_string());
    }

    if !port_available {
        recommendations.push(t!("cli.check.port-occupied", port = port).to_string());
    }

    if !cluster_enabled {
        recommendations.push(t!("cli.check.cluster-disabled").to_string());
    }

    if indexed_files == 0 {
        recommendations.push(t!("cli.check.no-indexed-files").to_string());
    }

    if paired_devices == 0 && cluster_enabled {
        recommendations.push(t!("cli.check.no-paired-devices").to_string());
    }

    if recommendations.is_empty() {
        recommendations.push(t!("cli.check.all-normal").to_string());
    }

    recommendations
}

/// Read active_locale without the heavy initializer (pre-init help path):
/// a single direct config-row read; falls back to en-US when the DB/table/
/// row is unavailable.
/// 免重初始化读取 active_locale(help 预分发路径用):直连读一行 config,
/// DB/表/行不可用时回落 en-US。
pub fn lightweight_locale() -> String {
    // config_repo::get_val walks the shared open_db() (busy_timeout applies);
    // empty string (row/db unavailable) falls back to en-US
    // config_repo::get_val 走共享 open_db()(含 busy_timeout);
    // 空串(DB/行不可用)回落 en-US
    let val = mango_finder_lib::repositories::config_repo::get_val("active_locale");
    if val.is_empty() {
        "en-US".to_string()
    } else {
        val
    }
}

/// Current locale from the ACTIVE_LOCALE global with an en-US fallback
/// (single read helper for status/locale/help).
/// 读取 ACTIVE_LOCALE(回落 en-US)的统一入口(status/locale/help 共用)。
fn current_locale() -> String {
    ACTIVE_LOCALE
        .read()
        .map(|l| l.clone())
        .unwrap_or_else(|_| "en-US".to_string())
}

pub fn handle_help(command: Option<&str>) {
    let is_zh = current_locale() == "zh-CN";
    // docs/cli_cn.md / cli.md are the single source of truth for help:
    // full help = the whole document, per-command help = its section.
    // docs/cli_cn.md / cli.md 是帮助的唯一事实来源:
    // 全量帮助 = 整份文档,单命令帮助 = 从中截取对应 section
    let doc = if is_zh {
        include_str!("../../../docs/cli_cn.md")
    } else {
        include_str!("../../../docs/cli.md")
    };

    match command {
        Some(cmd) => match extract_command_section(doc, cmd) {
            Some(section) => crate::outln!("{}", section),
            None => {
                // The available-command list is also derived from the doc's
                // section headings, so no extra list is maintained.
                // 可用命令列表同样取自文档的 section 标题,避免再维护一份
                let available: Vec<&str> = doc
                    .lines()
                    .filter_map(|l| l.strip_prefix("### "))
                    .filter_map(|h| h.split_whitespace().next())
                    .collect();
                if is_zh {
                    crate::outln!("未知命令: {}\n可用命令: {}", cmd, available.join(", "));
                } else {
                    crate::outln!(
                        "Unknown command: {}\nAvailable commands: {}",
                        cmd,
                        available.join(", ")
                    );
                }
            }
        },
        None => crate::outln!("{}", doc),
    }
}

/// Extract a single command's section from the CLI doc:
/// from the `### <command>` heading line up to the next `##`/`###` heading.
/// 从 CLI 文档中提取单个命令的 section:
/// `### <command>` 标题行起,到下一个 `##`/`###` 标题为止
fn extract_command_section(doc: &str, command: &str) -> Option<String> {
    let heading_prefix = format!("### {}", command);
    let mut out: Vec<&str> = Vec::new();
    let mut in_section = false;
    for line in doc.lines() {
        if in_section {
            if line.starts_with("## ") || line.starts_with("### ") {
                break;
            }
            out.push(line);
        } else if line == heading_prefix || line.starts_with(&format!("{} ", heading_prefix)) {
            in_section = true;
            out.push(line);
        }
    }
    if !in_section {
        return None;
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    Some(out.join("\n"))
}

/// Open a file with the OS default handler; stdio is nulled so the opened
/// application does not inherit our pipe write ends and hang the caller.
/// Spawn failures are reported to the caller (which fails the command)
/// instead of silently printing success.
/// 用系统默认程序打开文件;标准流置 null 避免被打开的程序继承本进程的
/// 管道写端导致调用方挂起。spawn 失败如实上报(由调用方令命令失败),
/// 而不是静默地打印成功。
fn open_with_system(path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // 不经 cmd.exe:cmd 会对双引号内的 %VAR% 做第一阶段展开,文件名含
        // 已定义环境变量名(如 %PATH%)时会被改写后静默打开失败;
        // explorer 直接接收 Win32 argv,与 GUI 的 open_directory 同路,
        // % 与 & 等元字符天然安全
        // No cmd.exe in the chain: cmd phase-1 expands %VAR% even inside
        // double quotes, so a filename containing a defined env-var name
        // (e.g. %PATH%) gets rewritten and the open fails silently; explorer
        // takes a plain Win32 argv (same route as the GUI's open_directory),
        // so % and & are safe by construction.
        std::process::Command::new("explorer")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
        Ok(())
    }
}
