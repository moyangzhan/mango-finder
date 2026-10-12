use crate::audio_parser::AudioParser;
use crate::entities::{AiModel, FileInfo};
use crate::enums::{FileCategory, FileParserMode, ModelPlatformName, ModelType};
use crate::errors::AppError;
use crate::global::{ACTIVE_MODEL_PLATFORM, INDEXER_SETTING};
use crate::model_platform_services::openai::OpenAi;
use crate::model_platform_services::openai_compatible_service::OpenAiCompatibleService;
use crate::model_platform_services::siliconflow::SiliconFlow;
use crate::repositories::ai_model_repo;
use crate::traits::audio_analyzer::AudioAnalyzer;
use crate::traits::indexing_template::IndexingTemplate;

pub struct AudioIndexer {
    category: FileCategory,
    remote_model: Option<AiModel>,
    remote_service: Option<Box<dyn AudioAnalyzer>>,
    local_parser: Option<AudioParser>,
}

impl<'a> AudioIndexer {
    pub async fn new() -> Result<AudioIndexer, AppError> {
        let mut local_parser_option: Option<AudioParser> = None;
        let mut remote_service_option: Option<Box<dyn AudioAnalyzer>> = None;
        let mut remote_model_option: Option<AiModel> = None;

        let audio_parser_mode = INDEXER_SETTING.read().await.audio_parser_mode.clone();

        match audio_parser_mode {
            FileParserMode::Local => {
                local_parser_option = Some(AudioParser::new()?);
            }
            FileParserMode::SelfHosted => {
                // Self-hosted platforms (Ollama/vLLM) do not support ASR
                return Err(AppError::UnsupportedAudioAnalyze(
                    "Self-hosted platforms (Ollama/vLLM) do not support ASR".to_string(),
                ));
            }
            FileParserMode::Remote => {
                let (platform_name, base_url) = {
                    let active_platform = crate::read_lock!(ACTIVE_MODEL_PLATFORM);
                    (
                        active_platform.name.clone(),
                        active_platform.base_url.clone(),
                    )
                };
                if let Ok(Some(ai_model)) =
                    ai_model_repo::get_one_by_type(platform_name.as_str(), ModelType::Asr.into())
                {
                    let platform_service: Box<dyn AudioAnalyzer> =
                        match ModelPlatformName::from(platform_name.as_str()) {
                            ModelPlatformName::OpenAi => Box::new(OpenAi::new().await),
                            ModelPlatformName::SiliconFlow => Box::new(SiliconFlow::new().await),
                            ModelPlatformName::DashScope | ModelPlatformName::DeepSeek => {
                                log::warn!(
                                    "DeepSeek and DashScope do not support audio analysis yet."
                                );
                                return Err(AppError::UnsupportedAudioAnalyze(
                                    "Deepseek and Dashscope".to_string(),
                                ));
                            }
                            _ => Box::new(
                                OpenAiCompatibleService::new(&platform_name, &base_url).await,
                            ),
                        };

                    remote_service_option = Some(platform_service);
                    remote_model_option = Some(ai_model);
                }
            }
        }

        if remote_service_option.is_some() || local_parser_option.is_some() {
            return Ok(Self {
                category: FileCategory::Audio,
                remote_model: remote_model_option,
                remote_service: remote_service_option,
                local_parser: local_parser_option,
            });
        }
        let asr: &str = ModelType::Asr.into();
        Err(AppError::AiModelNotFound(format!("model type:{}", asr)))
    }
}

impl IndexingTemplate for AudioIndexer {
    fn category(&self) -> &FileCategory {
        &self.category
    }

    async fn load_content(&self, file_info: &FileInfo) -> String {
        match &self.local_parser {
            Some(parser) => parser.parse(&file_info.path).await.unwrap_or_else(|e| {
                log::error!("audio parser error:{}", e);
                String::new()
            }),
            None => {
                let model = match &self.remote_model {
                    Some(m) => m,
                    None => {
                        log::warn!("No AI model configured for audio analysis");
                        return String::new();
                    }
                };
                match &self.remote_service {
                    Some(service) => service
                        .analyze_audio(model, &file_info.path)
                        .await
                        .unwrap_or_else(|e| {
                            log::error!("Error analyzing audio: {}", e);
                            String::new()
                        }),
                    None => {
                        log::warn!("No remote service configured for audio analysis");
                        String::new()
                    }
                }
            }
        }
    }
}
