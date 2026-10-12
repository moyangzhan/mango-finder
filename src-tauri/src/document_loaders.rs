pub mod anytomd_loader;
pub mod odp;
pub mod ods;
pub mod odt;
pub mod open_doc;
pub mod pdfplumber_loader;

use crate::global::EXTRACTED_IMAGES_PATH;
use std::path::PathBuf;

pub(crate) const BUCKET_SIZE: i64 = 1000;

/// Remove all extracted images for a file ID.
/// Deletes files matching `{id}_*` in the appropriate bucket directory.
pub(crate) fn cleanup_extracted_images(file_id: i64) {
    let images_dir = get_images_dir(file_id);
    if !images_dir.exists() {
        return;
    }
    let prefix = format!("{}_", file_id);
    if let Ok(entries) = std::fs::read_dir(&images_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

/// Get the extracted_images bucket directory for a file ID.
/// Structure: `extracted_images/{bucket}/` where bucket = (id - 1) / 1000
pub(crate) fn get_images_dir(file_id: i64) -> PathBuf {
    let extracted_path = EXTRACTED_IMAGES_PATH.get().cloned().unwrap_or_default();
    let bucket = if file_id > 0 {
        (file_id - 1) / BUCKET_SIZE
    } else {
        0
    };
    PathBuf::from(extracted_path).join(format!("{:04}", bucket))
}

/// Generate an image filename with file ID prefix: `{id}_{suffix}`
pub(crate) fn image_filename(file_id: i64, suffix: &str) -> String {
    format!("{}_{}", file_id, suffix)
}

/// Generate a human-readable, filesystem-safe name: `{id}_{name_without_ext}`
fn sanitize_fs_name(file_id: i64, file_name: &str) -> String {
    const MAX_NAME_LEN: usize = 80;
    // Strip extension (e.g. "report.pdf" → "report")
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    let sanitized: String = stem
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let truncated = if sanitized.len() > MAX_NAME_LEN {
        let end = sanitized
            .char_indices()
            .take_while(|(i, _)| *i < MAX_NAME_LEN)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(MAX_NAME_LEN);
        &sanitized[..end]
    } else {
        &sanitized
    };
    format!("{}_{}", file_id, truncated)
}

/// Generate a human-readable markdown relative path: `parsed_documents/{bucket}/{id}_{name}.md`
pub(crate) fn md_relative_path(file_id: i64, file_name: &str) -> String {
    let bucket = if file_id > 0 {
        (file_id - 1) / BUCKET_SIZE
    } else {
        0
    };
    format!(
        "parsed_documents/{:04}/{}.md",
        bucket,
        sanitize_fs_name(file_id, file_name)
    )
}

/// Truncate content in-place to at most `limit` chars.
pub(crate) fn truncate_to_char_limit(content: &mut String, limit: usize) {
    if content.len() > limit * 3 {
        if let Some((byte_pos, _)) = content.char_indices().nth(limit) {
            content.truncate(byte_pos);
        }
    }
}

/// Remove extracted images left over by external (un-indexed) document queries.
/// 清理外部文档查询遗留的抽取图片。
///
/// External queries parse with file_id = 0, so their images land in bucket 0000
/// with a `0_` prefix and are never referenced by any index row. Real indexed
/// files use ids >= 1 in the same bucket and are not affected.
pub fn cleanup_external_query_images() {
    let dir = get_images_dir(0);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("0_") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
