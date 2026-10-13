use comfy_table::{ContentArrangement, Table};
use serde_json::Value;
use std::io::IsTerminal;

/// Identifies which command produced the data passed to print_success; table mode picks the dedicated renderer accordingly.
/// JSON mode ignores this and keeps the output as-is.
/// 标识 print_success 收到的数据来自哪个命令，table 模式据此选择专用渲染器。
/// JSON 模式不使用该信息，输出保持原样。
#[derive(Debug, Clone, Copy)]
pub enum OutputKind {
    Search,
    Similar,
    IndexStatus,
    IndexStart,
    IndexList,
    IndexClear,
    File,
    DeviceList,
    Status,
    Version,
    Check,
    LocaleGet,
    LocaleSet,
    Shutdown,
}

/// When piped/redirected (non-TTY) comfy-table cannot detect the terminal width and dynamic layout degrades to no truncation; fix a width so redirected output stays deterministic.
/// 管道/重定向（非 TTY）时 comfy-table 检测不到终端宽度，动态排版会退化为不截断，固定一个宽度保证重定向输出确定
const PIPE_TABLE_WIDTH: u16 = 120;

pub fn render(kind: OutputKind, data: &Value) {
    match kind {
        OutputKind::Search => render_search(data),
        OutputKind::Similar => render_similar(data),
        OutputKind::IndexStatus => render_index_status(data),
        OutputKind::IndexStart => render_index_start(data),
        OutputKind::IndexList => render_index_list(data),
        OutputKind::File => render_file(data),
        OutputKind::DeviceList => render_device_list(data),
        OutputKind::Check => render_check(data),
        // Flat key-value payloads (index clear / status / version / locale / shutdown) use the generic renderer.
        OutputKind::IndexClear
        | OutputKind::Status
        | OutputKind::Version
        | OutputKind::LocaleGet
        | OutputKind::LocaleSet
        | OutputKind::Shutdown => render_kv(data),
    }
}

fn render_search(data: &Value) {
    let results = data["results"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if results.is_empty() {
        crate::outln!("{}", t!("cli.table.no_results"));
        return;
    }
    print_results_summary(data);
    crate::outln!();
    add_results_table(results);
}

fn render_similar(data: &Value) {
    let file = &data["file"];
    match file["id"].as_i64() {
        Some(id) => crate::outln!(
            "{}: {} (id={})",
            t!("cli.table.query_file"),
            file["name"].as_str().unwrap_or(""),
            id
        ),
        None => crate::outln!(
            "{}: {}",
            t!("cli.table.query_file"),
            file["name"].as_str().unwrap_or("")
        ),
    }
    crate::outln!("  {}", file["path"].as_str().unwrap_or(""));

    if data["mode"].as_str() == Some("external") {
        if data["indexed"].as_bool() == Some(true) {
            crate::outln!("{}", t!("cli.table.indexed_now"));
        }
        if let Some(cc) = data["chunk_count"].as_i64() {
            crate::outln!("{}: {}", label("chunk_count"), cc);
        }
    }

    crate::outln!();
    let results = data["results"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if results.is_empty() {
        crate::outln!("{}", t!("cli.table.no_results"));
        return;
    }
    print_results_summary(data);
    crate::outln!();
    add_results_table(results);

    // Footnotes outside the table.
    if let Some(notice) = data["notice"].as_str() {
        crate::outln!("\n{}: {}", t!("cli.table.notice"), notice);
    }
    if let Some(matches) = data["matched_files"].as_array() {
        crate::outln!("\n{}:", t!("cli.table.matched_files"));
        for m in matches {
            crate::outln!(
                "  id={}  {}",
                m["id"].as_i64().unwrap_or(0),
                m["path"].as_str().unwrap_or("")
            );
        }
    }
}

fn render_index_status(data: &Value) {
    render_kv(&serde_json::json!({
        "total_files": data["total_files"],
        "indexed_files": data["indexed_files"],
    }));

    let tasks = data["recent_tasks"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if tasks.is_empty() {
        return;
    }
    crate::outln!("\n{}:", t!("cli.table.recent_tasks"));
    let mut table = new_table(vec![
        t!("cli.table.id").to_string(),
        t!("cli.table.status").to_string(),
        t!("cli.table.path").to_string(),
        t!("cli.table.files").to_string(),
        t!("cli.table.failed").to_string(),
        t!("cli.table.duration").to_string(),
        t!("cli.table.start_time").to_string(),
    ]);
    for task in tasks {
        table.add_row(vec![
            task["id"].as_i64().unwrap_or(0).to_string(),
            task_status_label(&task["status"]),
            truncate_middle(task["paths"].as_str().unwrap_or(""), 48),
            task["total_cnt"].as_i64().unwrap_or(0).to_string(),
            task["content_indexed_failed_cnt"]
                .as_i64()
                .unwrap_or(0)
                .to_string(),
            format_duration(task["duration"].as_i64().unwrap_or(0)),
            format_datetime(task["start_time"].as_str()),
        ]);
    }
    crate::outln!("{table}");
}

fn render_index_start(data: &Value) {
    if let Some(msg) = data["message"].as_str() {
        crate::outln!("{}", msg);
    }
    if let Some(paths) = data["paths"].as_array() {
        for p in paths {
            crate::outln!("  {}", p.as_str().unwrap_or(""));
        }
    }
    if let Some(tip) = data["tip"].as_str() {
        crate::outln!("\n{}", tip);
    }
}

fn render_index_list(data: &Value) {
    let files = data["files"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if files.is_empty() {
        crate::outln!("{}", t!("cli.table.no_results"));
        return;
    }
    let mut table = new_table(vec![
        t!("cli.table.id").to_string(),
        t!("cli.table.name").to_string(),
        t!("cli.table.category").to_string(),
        t!("cli.table.path").to_string(),
    ]);
    for f in files {
        table.add_row(vec![
            f["id"].as_i64().unwrap_or(0).to_string(),
            f["name"].as_str().unwrap_or("").to_string(),
            category_label(f["category"].as_i64().unwrap_or(0)),
            f["path"].as_str().unwrap_or("").to_string(),
        ]);
    }
    crate::outln!("{table}");
    crate::outln!(
        "{}",
        t!(
            "cli.table.page_info",
            page = data["page"].as_i64().unwrap_or(0),
            size = data["page_size"].as_i64().unwrap_or(0)
        )
    );
}

fn render_file(data: &Value) {
    crate::outln!("{}: {}", label("id"), data["id"].as_i64().unwrap_or(0));
    crate::outln!("{}: {}", label("name"), data["name"].as_str().unwrap_or(""));
    crate::outln!(
        "{}: {}",
        label("category"),
        category_label(data["category"].as_i64().unwrap_or(0))
    );
    crate::outln!("{}: {}", label("path"), data["path"].as_str().unwrap_or(""));
    crate::outln!(
        "{}: {}",
        label("file_ext"),
        data["file_ext"].as_str().unwrap_or("")
    );
    crate::outln!(
        "{}: {}",
        label("file_size"),
        human_size(data["file_size"].as_i64().unwrap_or(0))
    );
    crate::outln!(
        "{}: {}",
        label("content_index_status"),
        index_status_label(
            &data["content_index_status"],
            &data["content_index_status_msg"]
        )
    );
    crate::outln!(
        "{}: {}",
        label("meta_index_status"),
        index_status_label(&data["meta_index_status"], &data["meta_index_status_msg"])
    );
    if let Some(r) = data["content_ref_path"].as_str() {
        crate::outln!("{}: {}", label("content_ref_path"), r);
    }
    crate::outln!(
        "{}: {}",
        label("file_create_time"),
        data["file_create_time"].as_str().unwrap_or("-")
    );
    crate::outln!(
        "{}: {}",
        label("file_update_time"),
        data["file_update_time"].as_str().unwrap_or("-")
    );

    // Content preview: single-line truncated; full content goes via JSON output.
    let content = data["content"].as_str().unwrap_or("");
    if !content.is_empty() {
        let truncated = content.chars().count() > 200;
        let preview: String = content
            .chars()
            .take(200)
            .collect::<String>()
            .replace(['\n', '\r'], " ");
        crate::outln!(
            "{}: {}{}",
            label("content_preview"),
            preview,
            if truncated { "…" } else { "" }
        );
    }
}

fn render_device_list(data: &Value) {
    let devices = data["devices"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if devices.is_empty() {
        crate::outln!("{}", t!("cli.table.no_results"));
        return;
    }
    let mut table = new_table(vec![
        t!("cli.table.name").to_string(),
        label("ip_address"),
        t!("cli.table.port").to_string(),
        t!("cli.table.version").to_string(),
        label("online_status"),
        label("pairing_status"),
        label("device_id"),
    ]);
    for d in devices {
        table.add_row(vec![
            d["name"].as_str().unwrap_or("").to_string(),
            d["ip_address"].as_str().unwrap_or("").to_string(),
            d["port"].as_i64().unwrap_or(0).to_string(),
            d["version"].as_str().unwrap_or("").to_string(),
            d["online_status"].as_str().unwrap_or("-").to_lowercase(),
            d["pairing_status"].as_str().unwrap_or("-").to_lowercase(),
            d["device_id"].as_str().unwrap_or("").to_string(),
        ]);
    }
    crate::outln!("{table}");
}

fn render_check(data: &Value) {
    for section in ["system", "network", "storage", "cluster", "ai_model"] {
        if let Some(v) = data.get(section) {
            let key = format!("cli.table.section.{section}");
            crate::outln!("[{}]", t!(&key));
            render_value_block(v, 1);
            crate::outln!();
        }
    }
    if let Some(recs) = data["recommendations"].as_array() {
        crate::outln!("[{}]", t!("cli.table.section.recommendations"));
        for r in recs {
            crate::outln!("  - {}", r.as_str().unwrap_or(&r.to_string()));
        }
    }
}

/// Generic key-value rendering: objects line by line, scalar arrays joined with commas, arrays of objects rendered as sub-tables.
/// 通用 key-value 渲染：对象逐行，标量数组逗号连接，对象数组渲染为子表格
fn render_kv(data: &Value) {
    render_value_block(data, 0);
}

fn render_value_block(v: &Value, indent: usize) {
    let pad = "  ".repeat(indent);
    let Some(obj) = v.as_object() else {
        crate::outln!("{pad}{}", scalar(v));
        return;
    };
    for (k, val) in obj {
        match val {
            // Arrays of objects render as sub-tables (e.g. check's network interfaces).
            Value::Array(arr) if !arr.is_empty() && arr[0].is_object() => {
                crate::outln!("{pad}{}:", label(k));
                let headers: Vec<String> = arr[0]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(|h| label(h))
                    .collect();
                let mut table = new_table(headers);
                for item in arr {
                    if let Some(o) = item.as_object() {
                        table.add_row(o.values().map(scalar).collect::<Vec<_>>());
                    }
                }
                let s = table.to_string();
                for line in s.lines() {
                    crate::outln!("{pad}  {line}");
                }
            }
            Value::Array(arr) => {
                crate::outln!(
                    "{pad}{}: {}",
                    label(k),
                    arr.iter().map(scalar).collect::<Vec<_>>().join(", ")
                );
            }
            Value::Object(_) => {
                crate::outln!("{pad}{}:", label(k));
                render_value_block(val, indent + 1);
            }
            _ => crate::outln!("{pad}{}: {}", label(k), scalar(val)),
        }
    }
}

/// Results table shared by search / similar: Score | Name | Category | Path
/// search / similar 共用的结果表格：Score | Name | Category | Path
fn add_results_table(results: &[Value]) {
    let mut table = new_table(vec![
        t!("cli.table.score").to_string(),
        t!("cli.table.name").to_string(),
        t!("cli.table.category").to_string(),
        t!("cli.table.path").to_string(),
    ]);
    for r in results {
        let f = &r["file_info"];
        table.add_row(vec![
            format!("{}%", r["score"].as_i64().unwrap_or(0)),
            f["name"].as_str().unwrap_or("").to_string(),
            category_label(f["category"].as_i64().unwrap_or(0)),
            f["path"].as_str().unwrap_or("").to_string(),
        ]);
    }
    crate::outln!("{table}");
}

fn print_results_summary(data: &Value) {
    let total = data["total"].as_i64().unwrap_or(0);
    let returned = data["returned"].as_i64().unwrap_or_else(|| {
        data["results"]
            .as_array()
            .map(|a| a.len() as i64)
            .unwrap_or(0)
    });
    let elapsed = data["elapsed_ms"].as_i64().unwrap_or(0);
    crate::outln!(
        "{}",
        t!(
            "cli.table.results_summary",
            total = total,
            returned = returned,
            elapsed = elapsed
        )
    );
}

fn new_table(headers: Vec<String>) -> Table {
    let mut table = Table::new();
    table.set_header(headers);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if !std::io::stdout().is_terminal() {
        table.set_width(PIPE_TABLE_WIDTH);
    }
    table
}

/// Translate a field name; falls back to the underscore-replaced original when no translation key exists.
/// 字段名翻译，无对应 key 时回退为下划线替换的原始名
fn label(key: &str) -> String {
    let k = format!("cli.table.field.{key}");
    let translated = t!(&k).to_string();
    if translated == k {
        key.replace('_', " ")
    } else {
        translated
    }
}

fn category_label(category: i64) -> String {
    let key = match category {
        1 => "document",
        2 => "image",
        3 => "audio",
        4 => "video",
        _ => "other",
    };
    let k = format!("cli.table.file_category.{key}");
    t!(&k).to_string()
}

fn task_status_label(v: &Value) -> String {
    let raw = match v.as_str() {
        Some(s) => s.to_lowercase(),
        None => v.to_string().to_lowercase(),
    };
    let key = format!("cli.table.task_status.{raw}");
    let translated = t!(&key).to_string();
    if translated == key { raw } else { translated }
}

fn index_status_label(status: &Value, msg: &Value) -> String {
    let key = match status.as_i64().unwrap_or(0) {
        1 => "waiting",
        2 => "indexing",
        3 => "indexed",
        4 => "failed",
        _ => return status.to_string(),
    };
    let k = format!("cli.table.index_status.{key}");
    let mut label = t!(&k).to_string();
    if let Some(m) = msg.as_str() {
        // "success" is the normal receipt of the status itself, not an error message; skip repeating it.
        if !m.is_empty() && m != "success" {
            label.push_str(&format!(" ({})", truncate_middle(m, 60)));
        }
    }
    label
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => yes_no(*b),
        Value::Null => "-".to_string(),
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(o) => format!("{{{} fields}}", o.len()),
    }
}

fn yes_no(b: bool) -> String {
    if b {
        t!("cli.table.yes").to_string()
    } else {
        t!("cli.table.no").to_string()
    }
}

/// Task timestamps serialize as ISO 8601 (with timezone); tables show a normalized date-time slice.
/// 任务时间戳序列化为 ISO 8601（含时区），表格中截取日期时间部分统一展示
fn format_datetime(s: Option<&str>) -> String {
    match s {
        Some(s) => s.chars().take(19).collect::<String>().replacen('T', " ", 1),
        None => "-".to_string(),
    }
}

fn format_duration(secs: i64) -> String {
    // IndexingTask.duration is in seconds (same source as the GUI's durationInSeconds column).
    if secs >= 60 {
        format!("{:.1}m", secs as f64 / 60.0)
    } else {
        format!("{secs}s")
    }
}

fn human_size(bytes: i64) -> String {
    let b = bytes as f64;
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Middle truncation for over-long fields such as paths, keeping head and tail
/// 超长路径等字段的中间截断，保留首尾
fn truncate_middle(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let head = max.saturating_sub(2) / 2;
    let tail = max.saturating_sub(2) - head;
    let skip_start = count - tail;
    format!(
        "{}…{}",
        s.chars().take(head).collect::<String>(),
        s.chars().skip(skip_start).take(tail).collect::<String>()
    )
}
