use crate::entities::FileInfo;
use crate::enums::FileCategory;
use crate::errors::AppError;
use crate::image_parser::ImageParser;
use crate::traits::indexing_template::IndexingTemplate;

pub struct ImageIndexer {
    category: FileCategory,
    parser: ImageParser,
}

impl ImageIndexer {
    pub async fn new() -> Result<ImageIndexer, AppError> {
        Ok(Self {
            category: FileCategory::Image,
            parser: ImageParser::new().await?,
        })
    }
}

impl IndexingTemplate for ImageIndexer {
    fn category(&self) -> &FileCategory {
        &self.category
    }

    async fn load_content(&self, file_info: &FileInfo) -> String {
        // Batch semantics: a failed file logs and yields empty, not an abort
        self.parser
            .analyze(&file_info.path)
            .await
            .unwrap_or_else(|e| {
                log::error!("image parse error:{}", e);
                String::new()
            })
    }
}
