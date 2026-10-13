use crate::render;
pub use crate::render::OutputKind;
use serde_json::Value;
use std::sync::OnceLock;

/// println! for a pipe-oriented CLI: write errors (e.g. a closed pipe after `mfd ... | head`) are ignored instead of panicking, so the process exits cleanly like standard Unix tools.
/// 面向管道场景的 println!:写失败(如 `mfd ... | head` 提前关闭管道)时忽略而非 panic,进程像标准 Unix 工具一样干净退出。
#[macro_export]
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, $($arg)*);
    }};
}

/// eprintln! counterpart of outln!: write errors are ignored
/// outln! 的 stderr 对应物:写失败同样忽略
#[macro_export]
macro_rules! outln_error {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, $($arg)*);
    }};
}

/// print_error callers do not carry the format argument; the format parsed by main is recorded here in a global.
/// print_error 的调用方不携带格式参数，用全局变量记录 main 解析出的输出格式
static OUTPUT_FORMAT: OnceLock<String> = OnceLock::new();

/// Called from main after parsing CLI args; records this run's output format
/// 在 main 解析 CLI 参数后调用，记录本次运行的输出格式
pub fn init(format: &str) {
    let _ = OUTPUT_FORMAT.set(format.to_string());
}

fn is_table_mode() -> bool {
    OUTPUT_FORMAT.get().map(|f| f == "table").unwrap_or(false)
}

/// Print a success envelope; the output format comes from the global set by main, so handlers do not thread a format parameter (single source of truth).
/// 输出成功信封;格式取自 main 设置的全局,handler 无需透传格式参数(单一来源)。
pub fn print_success(kind: OutputKind, data: &Value) {
    match is_table_mode() {
        true => render::render(kind, data),
        false => print_json(data),
    }
}

pub fn print_error(message: &str) {
    // table mode: single-line error to stderr so stdout keeps only table content; JSON mode keeps the envelope structure.
    if is_table_mode() {
        outln_error!("{}: {}", t!("cli.table.error"), message);
    } else {
        let error_response = serde_json::json!({
            "success": false,
            "data": null,
            "error": message
        });
        outln!("{}", serde_json::to_string_pretty(&error_response).unwrap());
    }
    std::process::exit(1);
}

fn print_json(data: &Value) {
    let response = serde_json::json!({
        "success": true,
        "data": data,
        "error": null
    });
    outln!("{}", serde_json::to_string_pretty(&response).unwrap());
}
