mod commands;
mod host_api;
mod output;
mod render;
mod serve;

use clap::CommandFactory;
use clap::Parser;
use mango_finder_lib::global::ACTIVE_LOCALE;
use mango_finder_lib::initializer;
use mango_finder_lib::utils::app_util;
use mango_finder_lib::utils::locale_util;

#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en-US");

fn main() {
    // Register sqlite-vec first; vec0 virtual tables (similar / index clear) need it
    mango_finder_lib::init_sqlite_extensions();

    let cli = commands::Cli::parse();

    // Record the output format so print_error can pick the error style
    output::init(&cli.output);

    // No subcommand: show help
    if cli.command.is_none() {
        let mut cmd = commands::Cli::command();
        cmd.print_help().ok();
        crate::outln!();
        return;
    }

    // Dispatch lightweight commands before the heavy initializer: version needs no DB/model, avoiding the init cost (DB migration check, ONNX runtime probing) on every invocation.
    if matches!(cli.command, Some(commands::Commands::Version)) {
        commands::handle_version();
        return;
    }

    // Init logging (non-silent mode)
    if !cli.quiet {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .format_timestamp(None)
            .init();
    }

    // Init paths (no Tauri); on failure print_error emits JSON and exits 1
    if let Err(e) = app_util::init_paths_standalone() {
        output::print_error(&format!(
            "Failed to initialize data directories: {}. Ensure the executable has write permissions to the data directory.",
            e
        ));
    }

    // Dispatch lightweight commands (help) before the heavy initializer too:
    // they print a static document and need no DB migration/ONNX; the locale comes from a direct single-row read (see lightweight_locale).
    if matches!(&cli.command, Some(commands::Commands::Help { .. })) {
        let locale = commands::lightweight_locale();
        rust_i18n::set_locale(locale.as_str());
        // handle_help reads the ACTIVE_LOCALE global for its language choice; set it too so document prose and table headers cannot diverge.
        if let Ok(mut guard) = ACTIVE_LOCALE.write() {
            *guard = locale;
        }
        if let Some(commands::Commands::Help { command }) = cli.command {
            commands::handle_help(command.as_deref());
        }
        return;
    }

    // Core service init (standalone=true: skips orphan task-lock cleanup, see the function comment)
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        initializer::process().await;
    });

    // The GUI applies the locale via a frontend set_active_locale call; the
    // CLI must apply it itself, otherwise t! falls back to en-US.
    locale_util::apply_persisted_locale();

    // Run the command
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
