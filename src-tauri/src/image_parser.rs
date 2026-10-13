use crate::entities::AiModel;
use crate::enums::{FileParserMode, ModelPlatformName, ModelType};
use crate::errors::AppError;
use crate::global::{ACTIVE_MODEL_PLATFORM, ACTIVE_SELF_HOSTED_PLATFORM, INDEXER_SETTING};
use crate::local_image_parser::build_local_analysis_text;
use crate::model_platform_services::dashscope::DashScope;
use crate::model_platform_services::openai::OpenAi;
use crate::model_platform_services::openai_compatible_service::OpenAiCompatibleService;
use crate::model_platform_services::siliconflow::SiliconFlow;
use crate::repositories::ai_model_repo;
use crate::self_hosted_services::ollama::Ollama;
use crate::self_hosted_services::vllm::Vllm;
use crate::traits::image_analyzer::ImageAnalyzer;
use crate::traits::self_hosted_image_analyzer::SelfHostedImageAnalyzer;
use crate::utils::app_util::get_vision_0_path;
use anyhow::Result;
use std::path::Path;

/// Image parser chosen by `image_parser_mode`; shared by the indexing and external-file pipelines so both caption identically.
/// 按 image_parser_mode 选出的图片解析器;索引与外部文件管线共用,保证同源打标。
pub struct ImageParser {
    is_local: bool,
    self_hosted_model: Option<AiModel>,
    self_hosted_service: Option<Box<dyn SelfHostedImageAnalyzer>>,
    remote_model: Option<AiModel>,
    remote_service: Option<Box<dyn ImageAnalyzer>>,
}

impl ImageParser {
    pub async fn new() -> Result<Self, AppError> {
        let mut analyzer = Self {
            is_local: false,
            self_hosted_model: None,
            self_hosted_service: None,
            remote_model: None,
            remote_service: None,
        };
        match INDEXER_SETTING.read().await.image_parser_mode.clone() {
            FileParserMode::Local => {
                let model_path = get_vision_0_path();
                if !Path::new(&model_path).exists() {
                    return Err(AppError::ImageParserInitError(format!(
                        "BLIP vision model not found at: {}",
                        model_path
                    )));
                }
                analyzer.is_local = true;
            }
            FileParserMode::SelfHosted => {
                let platform_name = crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM).name.clone();
                let vision: &str = ModelType::Vision.into();
                let ai_model =
                    ai_model_repo::get_one_by_type(&platform_name, ModelType::Vision.into())
                        .ok()
                        .flatten()
                        .ok_or_else(|| {
                            AppError::AiModelNotFound(format!("model type:{}", vision))
                        })?;
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
                analyzer.self_hosted_model = Some(ai_model);
                analyzer.self_hosted_service = Some(service);
            }
            FileParserMode::Remote => {
                let (platform_name, base_url) = {
                    let active_platform = crate::read_lock!(ACTIVE_MODEL_PLATFORM);
                    (
                        active_platform.name.clone(),
                        active_platform.base_url.clone(),
                    )
                };
                let vision: &str = ModelType::Vision.into();
                let ai_model = ai_model_repo::get_one_by_type(
                    platform_name.as_str(),
                    ModelType::Vision.into(),
                )
                .ok()
                .flatten()
                .ok_or_else(|| AppError::AiModelNotFound(format!("model type:{}", vision)))?;
                let service: Box<dyn ImageAnalyzer> =
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
                analyzer.remote_model = Some(ai_model);
                analyzer.remote_service = Some(service);
            }
        }
        Ok(analyzer)
    }

    /// Local never fails (BLIP/OCR degrade to empty); remote errors propagate.
    /// Local 不返回错误(BLIP/OCR 失败降级为空串);远程错误上抛。
    pub async fn analyze(&self, path: &str) -> Result<String, String> {
        if self.is_local {
            let p = path.to_string();
            return Ok(tokio::task::spawn_blocking(move || {
                build_local_analysis_text(Path::new(&p))
            })
            .await
            .unwrap_or_default());
        }
        if let Some(service) = &self.self_hosted_service {
            let model = self
                .self_hosted_model
                .as_ref()
                .ok_or("No vision model configured")?;
            return service
                .analyze_image(model, path)
                .await
                .map_err(|e| e.to_string());
        }
        if let Some(service) = &self.remote_service {
            let model = self
                .remote_model
                .as_ref()
                .ok_or("No vision model configured")?;
            return service
                .analyze_image(model, path)
                .await
                .map_err(|e| e.to_string());
        }
        Err("No image parser available".to_string())
    }
}
