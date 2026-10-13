//! Mock data generation for testing remote device functionality
//!
//! This module provides mock data generators for testing cross-device search and similarity features without requiring actual remote devices.

use crate::entities::FileInfo;
use crate::enums::{FileCategory, HitType, SimilarityType};
use crate::structs::search_result::{SearchResult, SourceDevice};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Mock device identifier
pub const MOCK_DEVICE_ID: &str = "mock-remote-device-001";
/// Mock device name
pub const MOCK_DEVICE_NAME: &str = "模拟远程设备";

// ============================================ Mock file data
// ============================================

/// Mock document files for similarity testing
pub const MOCK_DOCUMENTS: &[(&str, &str, &str, usize)] = &[
    (
        "项目计划书.docx",
        "docx",
        "/remote/Documents/项目计划书.docx",
        92,
    ),
    ("技术方案.pdf", "pdf", "/remote/Documents/技术方案.pdf", 85),
    (
        "需求分析报告.docx",
        "docx",
        "/remote/Documents/需求分析报告.docx",
        78,
    ),
    ("会议记录.md", "md", "/remote/Documents/会议记录.md", 72),
    ("产品说明.txt", "txt", "/remote/Documents/产品说明.txt", 65),
];

/// Mock image files for similarity testing
pub const MOCK_IMAGES: &[(&str, &str, &str, usize)] = &[
    ("设计稿_v2.png", "png", "/remote/Pictures/设计稿_v2.png", 95),
    (
        "产品截图_新.jpg",
        "jpg",
        "/remote/Pictures/产品截图_新.jpg",
        88,
    ),
    ("UI原型图.png", "png", "/remote/Pictures/UI原型图.png", 82),
    ("界面设计.jpg", "jpg", "/remote/Pictures/界面设计.jpg", 75),
    ("图标素材.png", "png", "/remote/Pictures/图标素材.png", 68),
];

/// Mock audio files for similarity testing
pub const MOCK_AUDIOS: &[(&str, &str, &str, usize)] = &[
    (
        "会议录音_new.mp3",
        "mp3",
        "/remote/Audio/会议录音_new.mp3",
        90,
    ),
    ("产品介绍.m4a", "m4a", "/remote/Audio/产品介绍.m4a", 83),
    ("培训音频.mp3", "mp3", "/remote/Audio/培训音频.mp3", 76),
    ("语音笔记.m4a", "m4a", "/remote/Audio/语音笔记.m4a", 70),
];

/// Mock other files for similarity testing
pub const MOCK_OTHERS: &[(&str, &str, &str, usize)] = &[
    ("相似文件_1.bin", "bin", "/remote/Files/相似文件_1.bin", 60),
    ("相似文件_2.dat", "dat", "/remote/Files/相似文件_2.dat", 55),
];

/// Mock files for search testing (name, ext, base_path, category, score)
pub const MOCK_SEARCH_RESULTS: &[(&str, &str, &str, i64, usize)] = &[
    ("搜索结果_文档.docx", "docx", "/remote/Documents/", 1, 95),
    ("搜索结果_图片.png", "png", "/remote/Pictures/", 2, 88),
    ("搜索结果_音频.mp3", "mp3", "/remote/Audio/", 3, 82),
    ("搜索结果_视频.mp4", "mp4", "/remote/Videos/", 4, 75),
    ("其他相关文件.pdf", "pdf", "/remote/Documents/", 1, 68),
];

// ============================================ Mock data generators
// ============================================

/// Generate mock similar file results for testing
pub fn generate_mock_similar_results(file_info: &FileInfo, limit: usize) -> Vec<SearchResult> {
    let category = FileCategory::from(file_info.category);

    let mock_files: &[(&str, &str, &str, usize)] = match category {
        FileCategory::Document => MOCK_DOCUMENTS,
        FileCategory::Image => MOCK_IMAGES,
        FileCategory::Audio => MOCK_AUDIOS,
        _ => MOCK_OTHERS,
    };

    mock_files
        .iter()
        .take(limit)
        .map(|(name, ext, path, score)| {
            let file_info_mock = FileInfo {
                id: rand_id(),
                name: name.to_string(),
                path: path.to_string(),
                category: file_info.category,
                file_ext: ext.to_string(),
                file_size: 1024 * 100,
                ..Default::default()
            };

            SearchResult {
                score: *score,
                hit_types: vec![HitType::MetaSemantic],
                file_info: file_info_mock,
                matched_keywords: HashSet::new(),
                matched_chunk_ids: HashSet::new(),
                similarity_type: Some(get_similarity_type(category)),
                source_device: Some(SourceDevice {
                    device_id: MOCK_DEVICE_ID.to_string(),
                    device_name: MOCK_DEVICE_NAME.to_string(),
                }),
            }
        })
        .collect()
}

/// Generate mock remote search results for testing
pub fn generate_mock_search_results(query: &str, limit: usize) -> Vec<SearchResult> {
    MOCK_SEARCH_RESULTS
        .iter()
        .take(limit)
        .map(|(name, ext, base_path, category, score)| {
            let file_info_mock = FileInfo {
                id: rand_id(),
                name: name.to_string(),
                path: format!("{}{}", base_path, name),
                category: *category,
                file_ext: ext.to_string(),
                file_size: 1024 * 100,
                ..Default::default()
            };

            let mut keywords = HashSet::new();
            keywords.insert(query.to_string());

            SearchResult {
                score: *score,
                hit_types: vec![HitType::ContentSemantic],
                file_info: file_info_mock,
                matched_keywords: keywords,
                matched_chunk_ids: HashSet::new(),
                similarity_type: None,
                source_device: Some(SourceDevice {
                    device_id: MOCK_DEVICE_ID.to_string(),
                    device_name: MOCK_DEVICE_NAME.to_string(),
                }),
            }
        })
        .collect()
}

// ============================================ Helper functions
// ============================================

/// Get similarity type based on file category
fn get_similarity_type(category: FileCategory) -> SimilarityType {
    match category {
        FileCategory::Image => SimilarityType::ImageSemantic,
        FileCategory::Document => SimilarityType::DocumentSemantic,
        FileCategory::Audio => SimilarityType::AudioTranscription,
        _ => SimilarityType::DocumentSemantic,
    }
}

/// Generate a random ID for mock data
fn rand_id() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_millis() % 1_000_000_000) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_mock_similar_results() {
        let file_info = FileInfo {
            id: 1,
            category: FileCategory::Document as i64,
            ..Default::default()
        };

        let results = generate_mock_similar_results(&file_info, 3);
        assert_eq!(results.len(), 3);
        assert!(results[0].source_device.is_some());
    }

    #[test]
    fn test_generate_mock_search_results() {
        let results = generate_mock_search_results("test query", 5);
        assert_eq!(results.len(), 5);
        assert!(results[0].matched_keywords.contains("test query"));
    }
}
