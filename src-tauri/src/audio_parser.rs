use crate::entities::AiModel;
use crate::enums::{FileParserMode, ModelPlatformName, ModelType};
use crate::errors::AppError;
use crate::global::{ACTIVE_MODEL_PLATFORM, INDEXER_SETTING};
use crate::local_audio_parser::LocalAudioParser;
use crate::model_platform_services::openai::OpenAi;
use crate::model_platform_services::openai_compatible_service::OpenAiCompatibleService;
use crate::model_platform_services::siliconflow::SiliconFlow;
use crate::repositories::ai_model_repo;
use crate::traits::audio_analyzer::AudioAnalyzer;

/// Audio parser chosen by `audio_parser_mode`; shared by the indexing and external-file pipelines so both transcribe identically.
/// 按 audio_parser_mode 选出的音频解析器;索引与外部文件管线共用,保证同源转写。
pub struct AudioParser {
    local: Option<LocalAudioParser>,
    remote_model: Option<AiModel>,
    remote_service: Option<Box<dyn AudioAnalyzer>>,
}

impl AudioParser {
    pub async fn new() -> Result<Self, AppError> {
        match INDEXER_SETTING.read().await.audio_parser_mode.clone() {
            FileParserMode::Local => Ok(Self {
                local: Some(LocalAudioParser::new()?),
                remote_model: None,
                remote_service: None,
            }),
            // Self-hosted platforms (Ollama/vLLM) do not support ASR
            FileParserMode::SelfHosted => Err(AppError::UnsupportedAudioAnalyze(
                "Self-hosted platforms (Ollama/vLLM) do not support ASR".to_string(),
            )),
            FileParserMode::Remote => {
                let (platform_name, base_url) = {
                    let active_platform = crate::read_lock!(ACTIVE_MODEL_PLATFORM);
                    (
                        active_platform.name.clone(),
                        active_platform.base_url.clone(),
                    )
                };
                let asr: &str = ModelType::Asr.into();
                let ai_model =
                    ai_model_repo::get_one_by_type(platform_name.as_str(), ModelType::Asr.into())
                        .ok()
                        .flatten()
                        .ok_or_else(|| AppError::AiModelNotFound(format!("model type:{}", asr)))?;
                let service: Box<dyn AudioAnalyzer> =
                    match ModelPlatformName::from(platform_name.as_str()) {
                        ModelPlatformName::OpenAi => Box::new(OpenAi::new().await),
                        ModelPlatformName::SiliconFlow => Box::new(SiliconFlow::new().await),
                        ModelPlatformName::DashScope | ModelPlatformName::DeepSeek => {
                            log::warn!("DeepSeek and DashScope do not support audio analysis yet.");
                            return Err(AppError::UnsupportedAudioAnalyze(
                                "Deepseek and Dashscope".to_string(),
                            ));
                        }
                        _ => {
                            Box::new(OpenAiCompatibleService::new(&platform_name, &base_url).await)
                        }
                    };
                Ok(Self {
                    local: None,
                    remote_model: Some(ai_model),
                    remote_service: Some(service),
                })
            }
        }
    }

    /// Local reuses pre-decoded samples when given; remote ASR reads the file.
    /// Local 复用已解码采样点(若提供);远程 ASR 读文件。
    pub async fn transcribe(&self, path: &str, samples: Option<&[f32]>) -> Result<String, String> {
        if let Some(parser) = &self.local {
            let result = match samples {
                Some(s) => parser.parse_from_samples(s).await,
                None => parser.parse(path).await,
            };
            return result.map_err(|e| e.to_string());
        }
        let model = self
            .remote_model
            .as_ref()
            .ok_or("No ASR model configured")?;
        let service = self
            .remote_service
            .as_ref()
            .ok_or("No ASR service configured")?;
        service
            .analyze_audio(model, path)
            .await
            .map_err(|e| e.to_string())
    }
}
