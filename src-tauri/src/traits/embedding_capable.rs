use crate::entities::{AiModel, ModelPlatform, SelfHostedPlatform};
use crate::errors::AppError;
use crate::structs::embed_result::EmbedResult;
use crate::structs::proxy_setting::ProxyInfo;
use crate::traits::with_platform_config::WithPlatformConfig;
use crate::traits::with_self_hosted_config::WithSelfHostedConfig;
use crate::utils::llm_client_util::{create_client, create_client_for_self_hosted};
use async_openai::types::{CreateEmbeddingRequestArgs, EmbeddingInput};
use std::sync::Arc;

/// Remote /v1/embeddings channel for model platforms (OpenAI-compatible protocol).
/// Output is dense-only: remote embedding endpoints never return a sparse
/// component, so `EmbedResult.sparse` stays empty and search runs dense-only.
///
/// 模型平台的远程 /v1/embeddings 通道(OpenAI 兼容协议)。
/// 输出仅有 dense:远程 embedding 端点不返回 sparse 分量,
/// EmbedResult.sparse 保持为空,检索走纯 dense 模式。
pub trait EmbeddingCapable: WithPlatformConfig + Sync {
    async fn embed_texts(&self, model: &AiModel, texts: &[String]) -> Result<Vec<EmbedResult>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let client = create_client(self.platform(), self.proxy())?;
        let request = CreateEmbeddingRequestArgs::default()
            .model(model.name.clone())
            .input(EmbeddingInput::StringArray(texts.to_vec()))
            .build()
            .map_err(AppError::OpenAiError)?;
        let response = client.embeddings().create(request).await?;
        // Response order follows request order; sort by index defensively
        let mut data = response.data;
        data.sort_by_key(|item| item.index);
        let results: Vec<EmbedResult> = data
            .into_iter()
            .map(|item| EmbedResult {
                dense: item.embedding,
                sparse: Default::default(),
            })
            .collect();
        if results.len() != texts.len() {
            return Err(AppError::EmbeddingError(format!(
                "embedding endpoint returned {} vectors for {} inputs",
                results.len(),
                texts.len()
            )));
        }
        Ok(results)
    }
}

/// Self-hosted /v1/embeddings channel (Ollama/vLLM speak the same protocol).
pub trait SelfHostedEmbeddingCapable: WithSelfHostedConfig + Sync {
    async fn embed_texts_self_hosted(
        &self,
        model: &AiModel,
        texts: &[String],
    ) -> Result<Vec<EmbedResult>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let client = create_client_for_self_hosted(self.platform())?;
        let request = CreateEmbeddingRequestArgs::default()
            .model(model.name.clone())
            .input(EmbeddingInput::StringArray(texts.to_vec()))
            .build()
            .map_err(AppError::OpenAiError)?;
        let response = client.embeddings().create(request).await?;
        let mut data = response.data;
        data.sort_by_key(|item| item.index);
        let results: Vec<EmbedResult> = data
            .into_iter()
            .map(|item| EmbedResult {
                dense: item.embedding,
                sparse: Default::default(),
            })
            .collect();
        if results.len() != texts.len() {
            return Err(AppError::EmbeddingError(format!(
                "embedding endpoint returned {} vectors for {} inputs",
                results.len(),
                texts.len()
            )));
        }
        Ok(results)
    }
}

/// Generic remote embedding client: any OpenAI-compatible model platform.
/// One client covers all platforms, so no per-platform dispatch is needed.
///
/// 通用远程 embedding 客户端:任意 OpenAI 兼容模型平台。
/// 单一客户端覆盖全部平台,无需按平台分发。
pub struct RemoteEmbeddingClient {
    platform: ModelPlatform,
    proxy: ProxyInfo,
}

impl RemoteEmbeddingClient {
    pub fn new(platform: ModelPlatform, proxy: ProxyInfo) -> Self {
        RemoteEmbeddingClient { platform, proxy }
    }
}

impl WithPlatformConfig for RemoteEmbeddingClient {
    fn platform(&self) -> &ModelPlatform {
        &self.platform
    }
    fn proxy(&self) -> &ProxyInfo {
        &self.proxy
    }
}

impl EmbeddingCapable for RemoteEmbeddingClient {}

/// Self-hosted embedding client (Ollama / vLLM via the OpenAI-compatible protocol).
pub struct SelfHostedEmbeddingClient {
    platform: Arc<SelfHostedPlatform>,
}

impl SelfHostedEmbeddingClient {
    pub fn new(platform: SelfHostedPlatform) -> Self {
        SelfHostedEmbeddingClient {
            platform: Arc::new(platform),
        }
    }
}

impl WithSelfHostedConfig for SelfHostedEmbeddingClient {
    fn platform(&self) -> &SelfHostedPlatform {
        &self.platform
    }
}

impl SelfHostedEmbeddingCapable for SelfHostedEmbeddingClient {}
