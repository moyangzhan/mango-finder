use crate::entities::{AiModel, FileInfo};
use crate::enums::{FileCategory, FileParserMode, ModelPlatformName, ModelType};
use crate::errors::AppError;
use crate::global::{ACTIVE_MODEL_PLATFORM, ACTIVE_SELF_HOSTED_PLATFORM, INDEXER_SETTING};
use crate::image_parser;
use crate::utils::app_util::get_vision_0_path;
use crate::model_platform_services::dashscope::DashScope;
use crate::model_platform_services::openai::OpenAi;
use crate::model_platform_services::openai_compatible_service::OpenAiCompatibleService;
use crate::model_platform_services::siliconflow::SiliconFlow;
use crate::repositories::ai_model_repo;
use crate::self_hosted_services::ollama::Ollama;
use crate::self_hosted_services::vllm::Vllm;
use crate::traits::image_analyzer::ImageAnalyzer;
use crate::traits::indexing_template::IndexingTemplate;
use crate::traits::self_hosted_image_analyzer::SelfHostedImageAnalyzer;

pub struct ImageIndexer {
    category: FileCategory,
    remote_model: Option<AiModel>,
    remote_service: Option<Box<dyn ImageAnalyzer>>,
    self_hosted_model: Option<AiModel>,
    self_hosted_service: Option<Box<dyn SelfHostedImageAnalyzer>>,
    is_local_mode: bool,
}

impl ImageIndexer {
    pub async fn new() -> Result<ImageIndexer, AppError> {
        let mut is_local_mode = false;
        let mut remote_service_option: Option<Box<dyn ImageAnalyzer>> = None;
        let mut remote_model_option: Option<AiModel> = None;
        let mut self_hosted_service_option: Option<Box<dyn SelfHostedImageAnalyzer>> = None;
        let mut self_hosted_model_option: Option<AiModel> = None;

        let image_parser_mode = INDEXER_SETTING.read().await.image_parser_mode.clone();

        match image_parser_mode {
            FileParserMode::Local => {
                let model_path = get_vision_0_path();
                if !std::path::Path::new(&model_path).exists() {
                    return Err(AppError::ImageParserInitError(format!(
                        "BLIP vision model not found at: {}", model_path
                    )));
                }
                is_local_mode = true;
            }
            FileParserMode::SelfHosted => {
                // Get active self-hosted platform
                let platform_name = crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM).name.clone();

                // Get vision model for self-hosted platform
                if let Ok(Some(ai_model)) =
                    ai_model_repo::get_one_by_type(&platform_name, ModelType::Vision.into())
                {
                    let service: Box<dyn SelfHostedImageAnalyzer> = match platform_name.as_str() {
                        "ollama" => Box::new(Ollama::new().await.map_err(|e| {
                            AppError::InternalError(format!("Failed to init Ollama: {}", e))
                        })?),
                        "vllm" => Box::new(Vllm::new().await.map_err(|e| {
                            AppError::InternalError(format!("Failed to init vLLM: {}", e))
                        })?),
                        _ => {
                            return Err(AppError::UnsupportedImageAnalyze(format!(
                                "Unknown self-hosted platform: {}",
                                platform_name
                            )));
                        }
                    };
                    self_hosted_service_option = Some(service);
                    self_hosted_model_option = Some(ai_model);
                }
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
                    ai_model_repo::get_one_by_type(platform_name.as_str(), ModelType::Vision.into())
                {
                    let platform_service: Box<dyn ImageAnalyzer> =
                        match ModelPlatformName::from(platform_name.as_str()) {
                            ModelPlatformName::OpenAi => Box::new(OpenAi::new().await),
                            ModelPlatformName::SiliconFlow => Box::new(SiliconFlow::new().await),
                            ModelPlatformName::DashScope => Box::new(DashScope::new().await),
                            ModelPlatformName::DeepSeek => {
                                log::warn!("DeepSeek do not support image analysis yet.");
                                return Err(AppError::UnsupportedImageAnalyze(
                                    "Deepseek model platforms".to_string(),
                                ));
                            }
                            _ => {
                                Box::new(OpenAiCompatibleService::new(&platform_name, &base_url).await)
                            }
                        };
                    remote_service_option = Some(platform_service);
                    remote_model_option = Some(ai_model);
                }
            }
        }

        if is_local_mode
            || remote_service_option.is_some()
            || self_hosted_service_option.is_some()
        {
            return Ok(Self {
                category: FileCategory::Image,
                remote_model: remote_model_option,
                remote_service: remote_service_option,
                self_hosted_model: self_hosted_model_option,
                self_hosted_service: self_hosted_service_option,
                is_local_mode,
            });
        }
        let vision: &str = ModelType::Vision.into();
        Err(AppError::AiModelNotFound(format!("model type:{}", vision)))
    }
}

impl IndexingTemplate for ImageIndexer {
    fn category(&self) -> &FileCategory {
        &self.category
    }
    async fn load_content(&self, file_info: &FileInfo) -> String {
        // 1. Try local parser first
        if self.is_local_mode {
            let path = file_info.path.clone();
            return tokio::task::spawn_blocking(move || {
                image_parser::build_local_analysis_text(std::path::Path::new(&path))
            })
            .await
            .unwrap_or_default();
        }

        // 2. Try self-hosted service
        if let Some(service) = &self.self_hosted_service {
            let model = match &self.self_hosted_model {
                Some(m) => m,
                None => {
                    log::warn!("No AI model configured for self-hosted image analysis");
                    return String::new();
                }
            };
            return service
                .analyze_image(model, &file_info.path)
                .await
                .unwrap_or_else(|e| {
                    log::error!("Error analyzing image with self-hosted service: {}", e);
                    String::new()
                });
        }

        // 3. Try remote service
        if let Some(service) = &self.remote_service {
            let model = match &self.remote_model {
                Some(m) => m,
                None => {
                    log::warn!("No AI model configured for image analysis");
                    return String::new();
                }
            };
            return service
                .analyze_image(model, &file_info.path)
                .await
                .unwrap_or_else(|e| {
                    log::error!("Error analyzing image: {}", e);
                    String::new()
                });
        }

        log::warn!("No image parser available");
        String::new()
    }
}
