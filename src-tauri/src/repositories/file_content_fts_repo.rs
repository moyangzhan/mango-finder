use crate::entities::FtsSearchResult;
use crate::repositories::RepositoryError;
use crate::utils::jieba_util;
use regex::Regex;
use rusqlite::{Result, named_params, params};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub async fn insert(file_id: i64, chunk_id: i64, chunk_text: &str) -> Result<(), RepositoryError> {
    if chunk_text.is_empty() {
        return Ok(());
    }
    let content = jieba_util::tokenize(chunk_text).await;
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("insert into file_content_fts (file_id, chunk_id, content) values (:file_id, :chunk_id, :content)")?;
    let last_insert_rowid = stmt.insert(named_params! {
        ":file_id": file_id,
        ":chunk_id": chunk_id,
        ":content": content,
    })?;
    log::debug!(
        "insert file_content_fts last_insert_rowid: {}",
        last_insert_rowid
    );
    Ok(())
}

pub async fn update(file_id: i64, chunk_id: i64, chunk_text: &str) -> Result<(), RepositoryError> {
    if chunk_text.is_empty() {
        return Ok(());
    }
    let content = jieba_util::tokenize(chunk_text).await;
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt =
        conn.prepare("update file_content_fts set content = :content where file_id = :file_id and chunk_id = :chunk_id")?;
    let affected = stmt.execute(named_params! {
        ":file_id": file_id,
        ":chunk_id": chunk_id,
        ":content": content,
    })?;
    log::debug!("update file_content_fts affected: {}", affected);
    Ok(())
}

pub fn delete_by_file_id(file_id: i64) -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_content_fts where file_id = ?1")?;
    let affected = stmt.execute([file_id])?;
    log::debug!(
        "delete file_content_fts by file id affected: {:?}",
        affected
    );
    Ok(affected)
}

pub fn delete_by_prefix_path(pre_path: &str) -> Result<usize, RepositoryError> {
    if pre_path.is_empty() {
        return Ok(0);
    }
    let pattern = if pre_path.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}%", pre_path)
    } else {
        format!("{}{}%", pre_path, std::path::MAIN_SEPARATOR)
    };
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_content_fts join file_info on file_content_fts.file_id = file_info.id where file_info.path = ?1 or file_info.path like ?2")?;
    let affected = stmt.execute((pre_path, pattern))?;
    log::debug!(
        "delete file_content_fts by prefix path affected: {:?}",
        affected
    );
    Ok(affected)
}

pub fn clear() -> Result<usize, RepositoryError> {
    let conn = crate::db_init_manager::open_db()?;
    let mut stmt = conn.prepare("delete from file_content_fts")?;
    let affected = stmt.execute([])?;
    log::debug!("clear file_content_fts affected: {:?}", affected);
    Ok(affected)
}

/// Multi-word search set to OR operator by default
pub async fn search(query: &str, limit: usize) -> Result<Vec<FtsSearchResult>, RepositoryError> {
    let start = Instant::now();
    let tokens = jieba_util::tokenize(&query).await;
    let keywords: Vec<&str> = tokens
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect();
    let match_query = keywords
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .join(" OR ");
    let conn = crate::db_init_manager::open_db()?;
    let max_rows = if keywords.len() <= 2 { 500 } else { 700 };
    let mut stmt = conn.prepare(
        r#"
        SELECT file_id, chunk_id, content, "rank"
        FROM file_content_fts
        WHERE file_content_fts MATCH ?
        ORDER BY "rank" DESC
        LIMIT ?;
        "#,
    )?;
    let rows = stmt.query_map(params![match_query, max_rows], |row| {
        Ok((
            row.get::<_, i64>("file_id")?,
            row.get::<_, i64>("chunk_id")?,
            row.get::<_, String>("content")?,
            row.get::<_, f64>("rank").unwrap_or(0.0),
        ))
    })?;
    let mut file_map: HashMap<i64, FtsSearchResult> = HashMap::new();
    let kw_len = keywords.len() as f64;
    for row in rows {
        let (file_id, chunk_id, content, rank) = row?;
        let mut matched_keyword = HashSet::new();
        for kw in &keywords {
            if content.contains(kw) {
                matched_keyword.insert(kw.to_string());
            }
        }

        // 1. Keyword coverage (0.0 - 1.0)
        let mut matched_keywords = HashSet::new();
        for kw in &keywords {
            if content.contains(kw) {
                matched_keywords.insert(kw.to_string());
            }
        }
        let coverage = matched_keywords.len() as f64 / kw_len;
        // 2. Convert FTS5 Rank to a 0-1 positive score
        // Lower FTS5 rank is better (usually negative); inverted, higher is better
        // Map via a simple logistic function
        let rank_score = 1.0 / (1.0 + (rank + 1.0).exp());
        // 3. Final 0-100 score
        // Coverage 70% weight (matched words rank first), BM25 rank 30%
        let final_score_f64 = coverage * 70.0 + rank_score * 30.0;
        let final_score = final_score_f64.clamp(0.0, 100.0) as usize;

        // Vue will do this let snippet = make_snippet(&content, &keywords, 40);
        let entry = file_map.entry(file_id).or_insert_with(|| FtsSearchResult {
            file_id,
            chunk_ids: HashSet::new(),
            matched_keywords: HashSet::new(),
            score: 0,
        });
        entry.chunk_ids.insert(chunk_id);
        entry.matched_keywords.extend(matched_keyword.into_iter());

        if final_score > entry.score {
            entry.score = final_score;
        }
    }
    let mut results: Vec<FtsSearchResult> = file_map.into_values().collect();
    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(limit);
    log::debug!("search file_content_fts cost: {:?}", start.elapsed());
    Ok(results)
}

fn extract_matches(content: &str, offsets: &str) -> Vec<String> {
    let mut result = Vec::new();
    let nums: Vec<usize> = offsets
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    for chunk in nums.chunks(4) {
        if chunk.len() == 4 {
            let byte_offset = chunk[2];
            let byte_len = chunk[3];
            if let Some(matched) = content.get(byte_offset..byte_offset + byte_len) {
                result.push(matched.to_string());
            }
        }
    }
    result.sort();
    result.dedup();
    result
}

pub fn make_snippet(text: &str, keywords: &[&str], radius: usize) -> String {
    if text.is_empty() || keywords.is_empty() {
        return text.chars().take(radius * 2).collect();
    }
    let mut first_pos: Option<usize> = None;
    for kw in keywords {
        if let Some(pos) = text.find(kw) {
            first_pos = match first_pos {
                Some(existing) => Some(existing.min(pos)),
                None => Some(pos),
            };
        }
    }
    let pos = match first_pos {
        Some(p) => p,
        None => return text.chars().take(radius * 2).collect(),
    };
    let char_positions: Vec<(usize, char)> = text.char_indices().collect();
    let total_chars = char_positions.len();

    let char_index = char_positions
        .iter()
        .position(|(byte_idx, _)| *byte_idx >= pos)
        .unwrap_or(0);

    let start_char = char_index.saturating_sub(radius);
    let end_char = (char_index + radius).min(total_chars - 1);

    let start_byte = char_positions[start_char].0;
    let end_byte = if end_char + 1 < total_chars {
        char_positions[end_char + 1].0
    } else {
        text.len()
    };

    let mut snippet = text[start_byte..end_byte].to_string();

    // highlight keywords
    for kw in keywords {
        if kw.is_empty() {
            continue;
        }
        // SAFETY: regex::escape() always produces a valid regex pattern as it escapes all special regex metacharacters
        let pattern = match Regex::new(&regex::escape(kw)) {
            Ok(p) => p,
            Err(_) => continue, // Skip invalid patterns (should not happen with escape)
        };
        snippet = pattern
            .replace_all(&snippet, |caps: &regex::Captures| {
                format!("<b>{}</b>", &caps[0])
            })
            .to_string();
    }

    if start_byte > 0 {
        snippet = format!("...{}", snippet);
    }
    if end_byte < text.len() {
        snippet = format!("{}...", snippet);
    }

    snippet
}
