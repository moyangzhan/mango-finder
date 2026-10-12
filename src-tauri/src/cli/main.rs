mod commands;
mod local_api;
mod output;
mod render;
mod serve;

use clap::CommandFactory;
use clap::Parser;
use mango_finder_lib::global::ACTIVE_LOCALE;
use mango_finder_lib::initializer;
use mango_finder_lib::utils::app_util;

#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en-US");

fn main() {
    // 注册 sqlite-vec 扩展，否则 vec0 虚拟表（similar / index clear 等）不可用
    mango_finder_lib::init_sqlite_extensions();

    let cli = commands::Cli::parse();

    // Record the output format so print_error can pick the error style
    // 记录输出格式供 print_error 选择错误输出样式
    output::init(&cli.output);

    // 如果没有子命令，显示帮助信息
    if cli.command.is_none() {
        let mut cmd = commands::Cli::command();
        cmd.print_help().ok();
        crate::outln!();
        return;
    }

    // 轻量命令在重初始化前分发：version 无需数据库/模型，避免每次多付
    // 初始化成本（DB 迁移检查、ONNX 运行时探测等）
    // Dispatch lightweight commands before the heavy initializer: version
    // needs no DB/model, avoiding the init cost (DB migration check, ONNX
    // runtime probing) on every invocation.
    if matches!(cli.command, Some(commands::Commands::Version)) {
        commands::handle_version();
        return;
    }

    // 初始化日志（非静默模式）
    if !cli.quiet {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .format_timestamp(None)
            .init();
    }

    // 初始化路径（不依赖 Tauri）；失败时 print_error 会输出 JSON 并以退出码 1 结束
    if let Err(e) = app_util::init_paths_standalone() {
        output::print_error(&format!(
            "Failed to initialize data directories: {}. Ensure the executable has write permissions to the data directory.",
            e
        ));
    }

    // 轻量命令(help)同样在重初始化前分发:输出的是静态文档,不需要
    // DB 迁移/ONNX;locale 用直连读取(见 lightweight_locale)
    // Dispatch lightweight commands (help) before the heavy initializer too:
    // they print a static document and need no DB migration/ONNX; the locale
    // comes from a direct single-row read (see lightweight_locale).
    if matches!(&cli.command, Some(commands::Commands::Help { .. })) {
        let locale = commands::lightweight_locale();
        rust_i18n::set_locale(locale.as_str());
        // handle_help 的语言判断读 ACTIVE_LOCALE 全局,这里一并写入,
        // 避免文档正文与表格头语言不一致
        // handle_help reads the ACTIVE_LOCALE global for its language choice;
        // set it too so document prose and table headers cannot diverge.
        if let Ok(mut guard) = ACTIVE_LOCALE.write() {
            *guard = locale;
        }
        if let Some(commands::Commands::Help { command }) = cli.command {
            commands::handle_help(command.as_deref());
        }
        return;
    }

    // 初始化核心服务(standalone=true:跳过孤儿任务锁清理,详见函数注释)
    // Core service init (standalone=true: skips orphan task-lock cleanup,
    // see the function comment)
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        initializer::process(true).await;
    });

    // The GUI sets the rust-i18n locale via the frontend calling
    // set_active_locale; the CLI must set it itself from the persisted
    // active_locale, otherwise t! always falls back to en-US.
    // GUI 由前端调用 set_active_locale 设置 rust-i18n locale；
    // CLI 需根据持久化的 active_locale 自行设置，否则 t! 一律回落 en-US
    if let Ok(locale) = ACTIVE_LOCALE.read() {
        rust_i18n::set_locale(locale.as_str());
    }

    // 执行命令
    let command = cli.command.unwrap();
    rt.block_on(async {
        match command {
            commands::Commands::Search {
                query,
                r#type,
                device,
                limit,
                help_doc,
            } => {
                if help_doc {
                    commands::handle_help(Some("search"));
                    return;
                }
                // clap 保证非 help_doc 时 query 必有值
                // clap guarantees query is present when help_doc is false
                let Some(query) = query else { return };
                commands::handle_search(&query, &r#type, device, limit).await;
            }
            commands::Commands::Similar {
                file,
                device,
                limit,
                index,
                help_doc,
            } => {
                if help_doc {
                    commands::handle_help(Some("similar"));
                    return;
                }
                // clap guarantees file is present when help_doc is false
                // clap 保证非 help_doc 时 file 必有值
                let Some(file) = file else { return };
                commands::handle_similar(&file, device, limit, index).await;
            }
            commands::Commands::Index { action, help_doc } => {
                if help_doc {
                    commands::handle_help(Some("index"));
                    return;
                }
                match action {
                    Some(action) => commands::handle_index(action).await,
                    None => commands::handle_help(Some("index")),
                }
            }
            commands::Commands::File {
                id,
                open,
                device,
                help_doc,
            } => {
                if help_doc {
                    commands::handle_help(Some("file"));
                    return;
                }
                match id {
                    Some(id) => commands::handle_file(id, open, device).await,
                    None => commands::handle_help(Some("file")),
                }
            }
            commands::Commands::Device { action, help_doc } => {
                if help_doc {
                    commands::handle_help(Some("device"));
                    return;
                }
                match action {
                    Some(action) => commands::handle_device(action).await,
                    None => commands::handle_help(Some("device")),
                }
            }
            commands::Commands::Status => {
                commands::handle_status().await;
            }
            commands::Commands::Version => {
                commands::handle_version();
            }
            commands::Commands::Help { command } => {
                commands::handle_help(command.as_deref());
            }
            commands::Commands::Check => {
                commands::handle_check().await;
            }
            commands::Commands::Locale { value } => {
                commands::handle_locale(value);
            }
            commands::Commands::Shutdown => {
                commands::handle_shutdown().await;
            }
            commands::Commands::Serve => {
                serve::run().await;
            }
        }
    });
}
