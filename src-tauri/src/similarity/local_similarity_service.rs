use crate::entities::FileInfo;
use crate::enums::FileCategory;
use crate::errors::AppError;
use crate::similarity::{
    audio_similarity::AudioSimilarityDetector, document_similarity::DocumentSimilarityDetector,
    image_similarity::ImageSimilarityDetector,
};
use crate::structs::search_result::SearchResult;
use crate::traits::similarity_detector::SimilarityDetector;

/// Find similar files based on file category
pub async fn find_similars_by_file_id(
    file_info: &FileInfo,
    limit: usize,
) -> Result<Vec<SearchResult>, AppError> {
    let category = FileCategory::from(file_info.category);

    match category {
        FileCategory::Document => {
            let detector = DocumentSimilarityDetector;
            detector.find_similars(file_info, limit).await
        }
        FileCategory::Image => {
            let detector = ImageSimilarityDetector;
            detector.find_similars(file_info, limit).await
        }
        FileCategory::Audio => {
            let detector = AudioSimilarityDetector;
            detector.find_similars(file_info, limit).await
        }
        _ => {
            // For unsupported categories, return empty results
            Ok(Vec::new())
        }
    }
}
