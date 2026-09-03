use clap::{Parser, Subcommand};
use local_ip_address::list_afinet_netifas;
use serde_json;
use std::net::{TcpListener, UdpSocket};
use std::path::Path;
use std::time::Instant;

use mango_finder_lib::searcher;
use mango_finder_lib::similarity::similarity_service;
use mango_finder_lib::indexer_service;
use mango_finder_lib::repositories::{file_info_repo, file_content_embedding_repo, file_content_fts_repo, file_metadata_embedding_repo, device_repo, indexing_task_repo, config_repo};
use mango_finder_lib::global::{CLIENT_ID, STORAGE_PATH, DB_PATH, EMBEDDING_MODEL_PATH, EMBEDDING_TOKENIZER_PATH, ACTIVE_LOCALE};

use crate::output;

#[derive(Parser)]
// 使用自定义的 Help 子命令（支持 [command] 参数），需禁用 clap 内置的 help 子命令，
// 否则 debug 构建会因子命令重名触发 clap 的 debug_asserts panic
#[command(name = "mfd", about = "Mango Finder Command Line Interface", disable_help_subcommand = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output format: json (default) or table
    #[arg(long, default_value = "json", env = "MANGO_FINDER_OUTPUT", global = true)]
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
        query: String,
        /// Search type: semantic (default), keyword
        #[arg(long, default_value = "semantic")]
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

    /// Find similar files by file ID
    Similar {
        /// File ID
        file_id: i64,
        /// Source device ID (optional)
        #[arg(long)]
        device: Option<String>,
        /// Max results
        #[arg(long, default_value = "10")]
        limit: usize,
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

pub async fn handle_search(query: &str, search_type: &str, device: Option<String>, limit: usize, output_format: &str) {
    let start = Instant::now();
    
    let results = if let Some(_device_id) = device {
        // TODO: 跨设备搜索
        output::print_error("Remote device search not yet implemented");
        return;
    } else {
        match search_type {
            "keyword" => searcher::keyword_search(query).await,
            _ => searcher::semantic_search(query).await,
        }
    };

    let elapsed = start.elapsed();
    let total = results.len();
    let limited_results: Vec<_> = results.into_iter().take(limit).collect();

    let data = serde_json::json!({
        "results": limited_results,
        "total": total,
        "returned": limit.min(total),
        "elapsed_ms": elapsed.as_millis()
    });

    output::print_success(&data, output_format);
}

pub async fn handle_similar(file_id: i64, device: Option<String>, limit: usize, output_format: &str) {
    if let Some(_device_id) = device {
        // TODO: 跨设备相似搜索
        output::print_error("Remote device similarity search not yet implemented");
        return;
    }

    let start = Instant::now();
    match similarity_service::find_similars_for_local_file(file_id, limit).await {
        Ok(results) => {
            let elapsed = start.elapsed();
            let data = serde_json::json!({
                "results": results,
                "total": results.len(),
                "elapsed_ms": elapsed.as_millis()
            });
            output::print_success(&data, output_format);
        }
        Err(e) => {
            output::print_error(&format!("Failed to find similar files: {}", e));
        }
    }
}

pub async fn handle_index(action: IndexAction, output_format: &str) {
    match action {
        IndexAction::Status => {
            let total = file_info_repo::count().unwrap_or(0);
            let indexed = file_content_embedding_repo::count().unwrap_or(0);

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
            output::print_success(&data, output_format);
        }
        IndexAction::Start { paths } => {
            if paths.is_empty() {
                output::print_error("No paths specified");
                return;
            }

            // CLI 是一次性进程：后台 spawn 的任务会随进程退出被终止，
            // 因此这里同步执行索引并输出最终结果。进度日志输出到 stderr，
            // 需要中途停止时按 Ctrl+C 终止进程。
            log::info!("Indexing started for paths: {:?}", paths);
            match indexer_service::start_indexing(paths.clone(), "cli").await {
                Ok(_) => {
                    let data = serde_json::json!({
                        "message": "Indexing finished",
                        "paths": paths,
                        "tip": "Use 'mfd index status' to see indexed file counts"
                    });
                    output::print_success(&data, output_format);
                }
                Err(e) => {
                    output::print_error(&format!("Indexing failed: {}", e));
                }
            }
        }
        IndexAction::List { page, page_size } => {
            match file_info_repo::list(page, page_size) {
                Ok(files) => {
                    let data = serde_json::json!({
                        "files": files,
                        "page": page,
                        "page_size": page_size
                    });
                    output::print_success(&data, output_format);
                }
                Err(e) => {
                    output::print_error(&format!("Failed to list files: {}", e));
                }
            }
        }
        IndexAction::Clear => {
            let clear_result = (|| -> Result<(), String> {
                file_content_fts_repo::clear().map_err(|e| e.to_string())?;
                file_content_embedding_repo::clear().map_err(|e| e.to_string())?;
                file_metadata_embedding_repo::clear().map_err(|e| e.to_string())?;
                file_info_repo::clear().map_err(|e| e.to_string())?;
                Ok(())
            })();
            match clear_result {
                Ok(()) => {
                    let data = serde_json::json!({
                        "message": "Index cleared"
                    });
                    output::print_success(&data, output_format);
                }
                Err(e) => {
                    output::print_error(&format!("Failed to clear index: {}", e));
                }
            }
        }
    }
}

pub async fn handle_file(id: i64, open: bool, device: Option<String>, output_format: &str) {
    if let Some(_device_id) = device {
        // TODO: 从远程设备获取文件
        output::print_error("Remote device file fetch not yet implemented");
        return;
    }

    match file_info_repo::get_by_id(id) {
        Ok(Some(file)) => {
            if open {
                if !Path::new(&file.path).exists() {
                    output::print_error(&format!("File path no longer exists: {}", file.path));
                    return;
                }
                open_with_system(&file.path);
            }
            output::print_success(&serde_json::to_value(&file).unwrap(), output_format);
        }
        Ok(None) => {
            output::print_error(&format!("File not found: {}", id));
        }
        Err(e) => {
            output::print_error(&format!("Failed to get file: {}", e));
        }
    }
}

pub async fn handle_device(action: DeviceAction, output_format: &str) {
    match action {
        DeviceAction::List => {
            match device_repo::list() {
                Ok(devices) => {
                    let data = serde_json::json!({
                        "devices": devices,
                        "total": devices.len()
                    });
                    output::print_success(&data, output_format);
                }
                Err(e) => {
                    output::print_error(&format!("Failed to list devices: {}", e));
                }
            }
        }
    }
}

pub async fn handle_status(output_format: &str) {
    let total_files = file_info_repo::count().unwrap_or(0);
    let indexed_files = file_content_embedding_repo::count().unwrap_or(0);
    let locale = ACTIVE_LOCALE
        .read()
        .map(|l| l.clone())
        .unwrap_or_else(|_| "en-US".to_string());

    let data = serde_json::json!({
        "total_files": total_files,
        "indexed_files": indexed_files,
        "locale": locale,
        "version": env!("CARGO_PKG_VERSION")
    });

    output::print_success(&data, output_format);
}

pub fn handle_version(output_format: &str) {
    let data = serde_json::json!({
        "name": "mfd",
        "version": env!("CARGO_PKG_VERSION")
    });

    output::print_success(&data, output_format);
}

pub fn handle_locale(value: Option<String>, output_format: &str) {
    match value {
        Some(locale) => {
            // 设置新的 locale
            let valid_locales = ["zh-CN", "en-US"];
            if !valid_locales.contains(&locale.as_str()) {
                output::print_error(&format!("Invalid locale '{}'. Valid values: {:?}", locale, valid_locales));
                return;
            }
            
            // 更新全局变量
            *ACTIVE_LOCALE.write().unwrap() = locale.clone();
            
            // 更新数据库
            if let Err(e) = config_repo::update_by_name("active_locale", &locale) {
                output::print_error(&format!("Failed to save locale: {}", e));
                return;
            }
            
            let data = serde_json::json!({
                "locale": locale,
                "message": "Locale updated successfully"
            });
            output::print_success(&data, output_format);
        }
        None => {
            // 显示当前 locale
            let locale = ACTIVE_LOCALE
                .read()
                .map(|l| l.clone())
                .unwrap_or_else(|_| "en-US".to_string());
            
            let data = serde_json::json!({
                "locale": locale
            });
            output::print_success(&data, output_format);
        }
    }
}

pub fn handle_check(output_format: &str) {
    // 获取本地 IP
    let local_ip = get_local_ip_address();
    
    // 获取客户端 ID
    let client_id = CLIENT_ID
        .read()
        .map(|id| id.clone())
        .unwrap_or_else(|_| "unknown".to_string());
    
    // 获取索引文件数量
    let indexed_files = file_info_repo::count().unwrap_or(0);
    let total_embeddings = file_content_embedding_repo::count().unwrap_or(0);
    
    // 获取集群设置（端口可在其中配置）
    let cluster_setting = get_cluster_setting();
    let device_name = cluster_setting.get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let cluster_enabled = cluster_setting.get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let port = cluster_setting.get("port")
        .and_then(|v| v.as_i64())
        .unwrap_or(15678) as u16;

    // 检查端口是否可用
    let port_available = check_port_available(port);
    
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

    output::print_success(&check, output_format);
}

fn get_local_ip_address() -> Option<String> {
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

fn get_cluster_setting() -> serde_json::Value {
    // 从数据库读取集群设置
    match config_repo::get_one("cluster_setting") {
        Ok(Some(config)) => {
            serde_json::from_str(&config.value).unwrap_or_else(|_| serde_json::json!({}))
        }
        _ => serde_json::json!({})
    }
}

fn check_port_available(port: u16) -> bool {
    TcpListener::bind(format!("0.0.0.0:{}", port)).is_ok()
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

pub fn handle_help(command: Option<&str>) {
    match command {
        Some(cmd) => {
            // 显示单个命令的帮助
            let help_content = get_command_help(cmd);
            println!("{}", help_content);
        }
        None => {
            // 显示完整的帮助文档
            let is_zh = ACTIVE_LOCALE
                .read()
                .map(|locale| locale.as_str() == "zh-CN")
                .unwrap_or(false);
            
            let doc = if is_zh {
                include_str!("../../../docs/cli_cn.md")
            } else {
                include_str!("../../../docs/cli.md")
            };
            println!("{}", doc);
        }
    }
}

fn get_command_help(command: &str) -> String {
    let is_zh = ACTIVE_LOCALE
        .read()
        .map(|locale| locale.as_str() == "zh-CN")
        .unwrap_or(false);
    
    match command {
        "search" => {
            if is_zh {
                r#"search - 搜索文档

用法:
    mfd search <query> [选项]

参数:
    <query>    搜索关键词

选项:
    --type <type>      搜索类型: semantic (默认), keyword
    --device <id>      远程设备 ID (可选)
    --limit <n>        最大结果数 (默认: 10)
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 语义搜索
    mfd search "机器学习"

    # 关键词搜索
    mfd search "report.docx" --type keyword

    # 限制结果数
    mfd search "AI" --limit 5

说明:
    - 语义搜索使用 AI embedding 进行语义匹配
    - 关键词搜索使用全文搜索进行精确匹配
    - 结果按相关性分数 (0-100) 排序"#
            } else {
                r#"search - Search documents

USAGE:
    mfd search <query> [OPTIONS]

ARGS:
    <query>    Search query string

OPTIONS:
    --type <type>      Search type: semantic (default), keyword
    --device <id>      Remote device ID (optional)
    --limit <n>        Max results (default: 10)
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Semantic search
    mfd search "machine learning"

    # Keyword search
    mfd search "report.docx" --type keyword

    # Limit results
    mfd search "AI" --limit 5

NOTES:
    - Semantic search uses AI embeddings for meaning-based matching
    - Keyword search uses full-text search for exact word matching
    - Results are ranked by relevance score (0-100)"#
            }
        }
        "similar" => {
            if is_zh {
                r#"similar - 查找相似文件

用法:
    mfd similar <file_id> [选项]

参数:
    <file_id>    文件 ID

选项:
    --device <id>      远程设备 ID (可选)
    --limit <n>        最大结果数 (默认: 10)
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 查找相似文件
    mfd similar 123

    # 限制结果数
    mfd similar 123 --limit 5

说明:
    - 基于文件内容相似度，而非文件名
    - 支持文档 (语义)、图片 (感知哈希)、音频 (指纹)"#
            } else {
                r#"similar - Find similar files by file ID

USAGE:
    mfd similar <file_id> [OPTIONS]

ARGS:
    <file_id>    File ID to find similar files for

OPTIONS:
    --device <id>      Remote device ID (optional)
    --limit <n>        Max results (default: 10)
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Find similar files
    mfd similar 123

    # Limit results
    mfd similar 123 --limit 5

NOTES:
    - Similarity is based on file content, not filename
    - Works for documents (semantic), images (perceptual hash), and audio (fingerprint)"#
            }
        }
        "index" => {
            if is_zh {
                r#"index - 索引管理

用法:
    mfd index <action>

操作:
    status              显示索引状态和最近任务
    start <paths...>    在前台执行索引任务，完成后返回
    list                列出已索引文件
    clear               清空所有索引

选项:
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 显示状态
    mfd index status

    # 开始索引（同步执行直到完成）
    mfd index start "C:\Documents" "D:\Projects"

    # 列出文件 (分页)
    mfd index list --page 1 --page-size 50

    # 清空索引
    mfd index clear

说明:
    - index start 在前台执行，进度日志输出到 stderr
    - 需要中途停止时按 Ctrl+C 终止
    - 使用 index status 查看已索引文件数量和任务历史
    - index clear 会永久删除所有索引数据"#
            } else {
                r#"index - Index management

USAGE:
    mfd index <action>

ACTIONS:
    status              Show index status and recent tasks
    start <paths...>    Start indexing in foreground, returns when done
    list                List indexed files
    clear               Clear all index

OPTIONS:
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Show status
    mfd index status

    # Start indexing (runs in foreground until finished)
    mfd index start "C:\Documents" "D:\Projects"

    # List files (with pagination)
    mfd index list --page 1 --page-size 50

    # Clear all index
    mfd index clear

NOTES:
    - index start runs in the foreground; progress logs go to stderr
    - Press Ctrl+C to stop it before completion
    - Use index status to see indexed file counts and task history
    - index clear deletes all indexed data permanently"#
            }
        }
        "file" => {
            if is_zh {
                r#"file - 文件操作

用法:
    mfd file <id> [选项]

参数:
    <id>    文件 ID

选项:
    --open             使用系统默认程序打开文件
    --device <id>      远程设备 ID (可选)
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 获取文件信息
    mfd file 123

    # 使用系统默认程序打开文件
    mfd file 123 --open

说明:
    - --open 使用系统默认程序打开任意类型的文件
    - 文件路径不存在时会报错"#
            } else {
                r#"file - File operations

USAGE:
    mfd file <id> [OPTIONS]

ARGS:
    <id>    File ID

OPTIONS:
    --open             Open file with system default program
    --device <id>      Remote device ID (optional)
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Get file info
    mfd file 123

    # Open file with the system default program
    mfd file 123 --open

NOTES:
    - --open works for any indexed file type
    - Errors if the file path no longer exists"#
            }
        }
        "device" => {
            if is_zh {
                r#"device - 设备管理

用法:
    mfd device <action>

操作:
    list    列出在线设备

选项:
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 列出在线设备
    mfd device list

说明:
    - 只显示已配对且在线的设备
    - 使用 GUI 管理设备配对"#
            } else {
                r#"device - Device management

USAGE:
    mfd device <action>

ACTIONS:
    list    List online devices

OPTIONS:
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # List online devices
    mfd device list

NOTES:
    - Shows paired and online devices only
    - Use the GUI to manage device pairing"#
            }
        }
        "status" => {
            if is_zh {
                r#"status - 显示应用状态

用法:
    mfd status [选项]

选项:
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 显示状态
    mfd status

说明:
    - 显示总文件数、已索引文件数、索引状态
    - 显示当前语言设置和版本号"#
            } else {
                r#"status - Show application status

USAGE:
    mfd status [OPTIONS]

OPTIONS:
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Show status
    mfd status

NOTES:
    - Shows total files, indexed files, and indexing status
    - Shows current locale and version"#
            }
        }
        "version" => {
            if is_zh {
                r#"version - 显示版本信息

用法:
    mfd version [选项]

选项:
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 显示版本
    mfd version"#
            } else {
                r#"version - Show version information

USAGE:
    mfd version [OPTIONS]

OPTIONS:
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Show version
    mfd version"#
            }
        }
        "check" => {
            if is_zh {
                r#"check - 检查系统状态

用法:
    mfd check [选项]

选项:
    --output <format>  输出格式: json (默认), table
    --quiet            静默模式
    -h, --help         显示帮助

示例:
    # 检查系统状态
    mfd check

检查内容:
    - 系统信息 (版本、设备名称、客户端 ID)
    - 网络状态 (本地 IP、网络接口、端口可用性)
    - 存储状态 (路径、已索引文件数量)
    - 集群状态 (是否启用、已配对设备数量)
    - AI 模型状态 (模型文件是否存在)"#
            } else {
                r#"check - Check system status

USAGE:
    mfd check [OPTIONS]

OPTIONS:
    --output <format>  Output format: json (default), table
    --quiet            Suppress logs
    -h, --help         Print help

EXAMPLES:
    # Check system status
    mfd check

Check items:
    - System info (version, device name, client ID)
    - Network status (local IP, network interfaces, port availability)
    - Storage status (path, indexed files count)
    - Cluster status (enabled, paired devices count)
    - AI model status (model files existence)"#
            }
        }
        "locale" => {
            if is_zh {
                r#"locale - 获取或设置语言

用法:
    mfd locale [value]

参数:
    value    要设置的语言 (如 zh-CN, en-US)
             如果不指定，则显示当前语言

选项:
    -h, --help    显示帮助

示例:
    # 显示当前语言
    mfd locale

    # 设置为中文
    mfd locale zh-CN

    # 设置为英文
    mfd locale en-US"#
            } else {
                r#"locale - Get or set locale

USAGE:
    mfd locale [value]

ARGS:
    value    Locale to set (e.g., zh-CN, en-US)
             If not specified, show current locale

OPTIONS:
    -h, --help    Print help

EXAMPLES:
    # Show current locale
    mfd locale

    # Set locale to Chinese
    mfd locale zh-CN

    # Set locale to English
    mfd locale en-US"#
            }
        }
        "help" => {
            if is_zh {
                r#"help - 显示帮助文档

用法:
    mfd help [command]

参数:
    command    命令名称 (可选，显示完整帮助)

示例:
    # 显示完整帮助
    mfd help

    # 显示 search 命令帮助
    mfd help search"#
            } else {
                r#"help - Show help documentation

USAGE:
    mfd help [command]

ARGS:
    command    Command name (optional, shows full help)

EXAMPLES:
    # Show full help
    mfd help

    # Show search command help
    mfd help search"#
            }
        }
        _ => {
            return if is_zh {
                format!("未知命令: {}\n可用命令: search, similar, index, file, device, status, version, check, locale, help", command)
            } else {
                format!("Unknown command: {}\nAvailable commands: search, similar, index, file, device, status, version, check, locale, help", command)
            };
        }
    }.to_string()
}

fn open_with_system(path: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(path)
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(path)
            .spawn();
    }
}

