pub mod anytomd_loader;
pub mod odp;
pub mod ods;
pub mod odt;
pub mod open_doc;
pub mod pdfplumber_loader;

use crate::global::EXTRACTED_IMAGES_PATH;
use std::path::{Path, PathBuf};

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

/// Unique temp dir per external (file_id = 0) parse: images isolated per query (no cross-query name collisions), deleted afterwards.
/// 外部解析(file_id=0)每次查询独立临时目录:图片按查询隔离(无重名冲突),事后删除。
pub(crate) fn temp_images_dir() -> PathBuf {
    let base = EXTRACTED_IMAGES_PATH.get().cloned().unwrap_or_default();
    PathBuf::from(base).join(format!("tmp_{}", uuid::Uuid::new_v4().simple()))
}

/// RAII guard: removes the whole dir on drop, covering every exit path (early error returns and panics included).
/// RAII 守卫:drop 时整目录删除,覆盖所有退出路径(含错误提前返回与 panic)。
pub(crate) struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        // NotFound is the expected race with the stale-dir sweep; other failures leave a dir the sweep will catch later
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            log::debug!("remove temp images dir {:?} failed: {}", self.0.display(), e);
        }
    }
}

/// Resolve the images dir for one parse: indexed parses (id > 0) use their persistent bucket; external parses get a temp dir guarded for auto-removal.
/// 解析所用图片目录:索引解析(id>0)用持久桶;外部解析返回临时目录并附守卫。
pub(crate) fn images_dir_for(file_id: i64) -> (PathBuf, Option<TempDirGuard>) {
    if file_id > 0 {
        (get_images_dir(file_id), None)
    } else {
        let dir = temp_images_dir();
        let guard = TempDirGuard(dir.clone());
        (dir, Some(guard))
    }
}

/// A temp dir idle this long is a crash leftover (a live parse runs minutes).
/// 临时目录闲置该时长即视为崩溃残留(进行中的解析最多数分钟)。
const STALE_TEMP_DIR_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Sweep orphaned temp dirs left by crashed processes.
/// 清扫崩溃进程遗留的孤儿临时目录。
pub fn cleanup_stale_temp_images_dirs() {
    let base = EXTRACTED_IMAGES_PATH.get().cloned().unwrap_or_default();
    if base.is_empty() {
        return;
    }
    sweep_stale_temp_dirs_in(std::path::Path::new(&base), STALE_TEMP_DIR_AGE);
}

/// Core sweep: remove `tmp_*` dirs whose mtime is older than `max_age`; bucket dirs (`0000`-`9999`) are never touched.
/// The age gate makes it safe next to a live query (its dir was just written).
/// 清扫核心:删除 mtime 早于 max_age 的 tmp_* 目录;桶目录(`0000`-`9999`)不受影响。
/// 时间阈值保证不会误删进行中查询的目录(其目录刚被写入)。
fn sweep_stale_temp_dirs_in(base: &Path, max_age: std::time::Duration) {
    let cutoff = std::time::SystemTime::now() - max_age;
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("tmp_") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t < cutoff)
            .unwrap_or(false);
        if stale {
            if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                log::warn!(
                    "remove stale temp dir {:?} failed: {}",
                    entry.path().display(),
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_removes_only_stale_tmp_dirs() {
        let base = std::env::temp_dir().join(format!(
            "mango_tmp_sweep_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(base.join("tmp_old")).unwrap();
        std::fs::create_dir_all(base.join("tmp_fresh")).unwrap();
        std::fs::create_dir_all(base.join("0001")).unwrap();

        // Zero age = everything counts as stale; only tmp_* dirs are removed
        sweep_stale_temp_dirs_in(&base, std::time::Duration::ZERO);
        assert!(!base.join("tmp_old").exists());
        assert!(!base.join("tmp_fresh").exists());
        assert!(base.join("0001").exists());

        // Large age = nothing stale, bucket dirs kept
        sweep_stale_temp_dirs_in(&base, std::time::Duration::from_secs(365 * 24 * 3600));
        assert!(base.join("0001").exists());

        std::fs::remove_dir_all(&base).unwrap();
    }
}
