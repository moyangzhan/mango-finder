use crate::audio_parser::AudioParser;
use crate::entities::FileInfo;
use crate::enums::FileCategory;
use crate::errors::AppError;
use crate::traits::indexing_template::IndexingTemplate;

pub struct AudioIndexer {
    category: FileCategory,
    parser: AudioParser,
}

impl AudioIndexer {
    pub async fn new() -> Result<AudioIndexer, AppError> {
        Ok(Self {
            category: FileCategory::Audio,
            parser: AudioParser::new().await?,
        })
    }
}

impl IndexingTemplate for AudioIndexer {
    fn category(&self) -> &FileCategory {
        &self.category
    }

    async fn load_content(&self, file_info: &FileInfo) -> String {
        // Batch semantics: a failed file logs and yields empty, not an abort
        self.parser
            .transcribe(&file_info.path, None)
            .await
            .unwrap_or_else(|e| {
                log::error!("audio parse error:{}", e);
                String::new()
            })
    }
}
