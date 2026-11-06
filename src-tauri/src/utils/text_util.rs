use crate::global::{DOCUMENT_CHUNK_OVERLAP, DOCUMENT_CHUNK_SIZE};
use std::error::Error;
use text_splitter::{ChunkConfig, TextSplitter};
use tokenizers::Tokenizer;

pub fn collapse_newlines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_newline = false;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\r' {
            // Check if "\r\n" is present
            if let Some('\n') = chars.peek() {
                chars.next(); // Skip '\n'
            }

            if !last_was_newline {
                result.push('\n'); // Add a newline if the previous character was not a newline '\n'
                last_was_newline = true;
            }
        } else if ch == '\n' {
            if !last_was_newline {
                result.push('\n');
                last_was_newline = true;
            }
        } else {
            result.push(ch);
            last_was_newline = false;
        }
    }

    result
}

pub fn split_text(text: &str, tokenizer: &Tokenizer) -> Result<Vec<String>, Box<dyn Error>> {
    let config = ChunkConfig::new(DOCUMENT_CHUNK_SIZE)
        .with_sizer(&tokenizer)
        .with_overlap(DOCUMENT_CHUNK_OVERLAP)?;
    let splitter = TextSplitter::new(config);
    Ok(splitter.chunks(text).map(|x| x.to_string()).collect())
}

/// Character-based chunking fallback for backends without a local tokenizer
/// (remote / self-hosted embedding, or the lite package with no model files).
/// The default ChunkConfig sizer is already char-based.
///
/// 无本地 tokenizer 的后端(远程/自托管 embedding,或无模型文件的 lite 包)
/// 使用的按字符分块回退。ChunkConfig 默认 sizer 即按字符。
pub fn split_text_by_chars(text: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let config = ChunkConfig::new(DOCUMENT_CHUNK_SIZE).with_overlap(DOCUMENT_CHUNK_OVERLAP)?;
    let splitter = TextSplitter::new(config);
    Ok(splitter.chunks(text).map(|x| x.to_string()).collect())
}
