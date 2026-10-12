use crate::entities::FileInfo;
use crate::errors::AppError;
use crate::structs::search_result::SearchResult;
use async_trait::async_trait;

/// Trait for detecting similar files
/// 检测相似文件的 Trait
#[async_trait]
pub trait SimilarityDetector: Send {
    async fn find_similars(
        &self,
        file_info: &FileInfo,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AppError>;
}
