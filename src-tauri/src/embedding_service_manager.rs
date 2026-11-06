use crate::embedding_service::EmbeddingService;
use crate::entities::AiModel;
use crate::enums::FileParserMode;
use crate::errors::AppError;
use crate::global::{ACTIVE_SELF_HOSTED_PLATFORM, INDEXER_SETTING, PROXY};
use crate::repositories::{ai_model_repo, embedding_collection_repo, model_platform_repo};
use crate::structs::embed_result::EmbedResult;
use crate::structs::embedding_identity::EmbeddingIdentity;
use crate::traits::embedding_capable::{
    EmbeddingCapable, RemoteEmbeddingClient, SelfHostedEmbeddingCapable,
    SelfHostedEmbeddingClient,
};
use crate::utils::text_util;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::time::Duration;

static EMBEDDING_SERVICE_MANAGER: OnceLock<AsyncRwLock<EmbeddingServiceManager>> = OnceLock::new();
pub static EMBEDDING_SERVICE_CREATING: AtomicBool = AtomicBool::new(false);
pub static SERVICE_DURATION: Duration = Duration::from_secs(60 * 30);

pub fn get_manager() -> &'static AsyncRwLock<EmbeddingServiceManager> {
    EMBEDDING_SERVICE_MANAGER.get_or_init(|| AsyncRwLock::new(EmbeddingServiceManager::default()))
}

pub struct EmbeddingServiceManager {
    pub service: Option<EmbeddingService>,
    pub last_used: Instant,
}

impl Default for EmbeddingServiceManager {
    fn default() -> Self {
        Self {
            service: None,
            last_used: Instant::now(),
        }
    }
}

impl EmbeddingServiceManager {
    pub async fn warmup(&mut self) -> Result<(), AppError> {
        if self.service.is_none() && !EMBEDDING_SERVICE_CREATING.load(Ordering::SeqCst) {
            EMBEDDING_SERVICE_CREATING.store(true, Ordering::SeqCst);
            match EmbeddingService::new().await {
                Ok(service) => {
                    self.service = Some(service);
                    EMBEDDING_SERVICE_CREATING.store(false, Ordering::SeqCst);
                }
                Err(e) => {
                    log::error!("EmbeddingService::new failed: {:?}", e);
                    EMBEDDING_SERVICE_CREATING.store(false, Ordering::SeqCst);
                    return Err(e);
                }
            }
        }
        self.update_last_used();
        Ok(())
    }

    pub async fn service(&mut self) -> Result<&EmbeddingService, AppError> {
        self.warmup().await?;
        return self.service.as_ref().ok_or(AppError::EmbeddingError(
            "Embedding service is none".to_string(),
        ));
    }

    pub fn remove_if_expired(&mut self) {
        if self.last_used.elapsed() > SERVICE_DURATION {
            self.service = None;
        }
    }

    pub fn clear(&mut self) {
        self.service = None;
    }

    pub async fn embed(&mut self, text: &str) -> Result<EmbedResult, AppError> {
        let service = self.service().await?;
        let result = service.embed(text);
        self.update_last_used();
        result
    }

    fn update_last_used(&mut self) {
        self.last_used = Instant::now();
    }
}

// ---------------------------------------------------------------------------
// Embedding dispatcher: routes the vector category through its own backend
// setting (local / self-hosted / remote), independent of the parser-category
// platforms. The active identity (platform+model+dim+has_sparse) is read from
// the config table - the single source of truth shared with the repo layer,
// so GUI, CLI and the mfd daemon can never disagree.
//
// 嵌入分发器:向量类目按自身后端设置(local/selfhosted/remote)分发,
// 与解析类目平台无关。激活身份从 config 表直读 —— 与 repo 层同源的
// 单一事实源,GUI、CLI 与 mfd daemon 不会出现分歧。
// ---------------------------------------------------------------------------

/// Current embedding category backend mode.
pub async fn current_mode() -> FileParserMode {
    INDEXER_SETTING.read().await.embedding_parser_mode.clone()
}

/// Whether the active identity matches the current mode's platform expectation.
fn identity_matches_mode(identity: &EmbeddingIdentity, mode: &FileParserMode) -> bool {
    match mode {
        FileParserMode::Local => identity.is_local(),
        FileParserMode::SelfHosted => {
            !identity.is_local() && identity.platform == crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM).name
        }
        FileParserMode::Remote => {
            // Remote identities use a model-platform name that is neither the
            // local marker nor the active self-hosted platform name
            !identity.is_local()
                && identity.platform != crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM).name
        }
    }
}

/// Post-embed guardrail: response dim must equal the active collection dim.
/// Catches "same name, different model" server-side drift explicitly instead
/// of letting vec0 fail (or worse, silently mixing spaces).
fn assert_dim(results: &[EmbedResult], identity: &EmbeddingIdentity) -> Result<(), AppError> {
    for result in results {
        if result.dense.len() != identity.dim {
            return Err(AppError::EmbeddingSizeMismatch(format!(
                "{} · {} returned dim {} but the active collection expects {}; the model behind the name likely changed",
                identity.model, identity.platform, result.dense.len(), identity.dim
            )));
        }
    }
    Ok(())
}

/// Load the ai_model row for the active identity's platform+model.
fn load_identity_model(identity: &EmbeddingIdentity) -> Result<AiModel, AppError> {
    ai_model_repo::get_one(&identity.platform, &identity.model)
        .map_err(|e| AppError::InternalError(format!("Failed to load embedding model: {}", e)))?
        .ok_or(AppError::AiModelNotFound(format!(
            "{} · {}",
            identity.model, identity.platform
        )))
}

/// Embedding entry point used by indexing, search and similarity.
/// Embeds through the backend selected for the vector category.
pub async fn embed(text: &str) -> Result<EmbedResult, AppError> {
    if text.is_empty() {
        return Err(AppError::EmbeddingError("empty text".to_string()));
    }
    let mut results = embed_batch(&[text.to_string()]).await?;
    results.pop().ok_or(AppError::EmbeddingError(
        "embedding backend returned no result".to_string(),
    ))
}

/// Batch embedding: remote backends get one /v1/embeddings call per batch,
/// the local ONNX session loops.
pub async fn embed_batch(texts: &[String]) -> Result<Vec<EmbedResult>, AppError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let mode = current_mode().await;
    let identity = embedding_collection_repo::active_identity();
    if !identity_matches_mode(&identity, &mode) {
        return Err(AppError::EmbeddingError(format!(
            "embedding backend setting ({}) does not match the active vector collection identity ({} · {}); switch the vector backend in settings first",
            mode_text(&mode), identity.model, identity.platform
        )));
    }
    let results = match mode {
        FileParserMode::Local => {
            let mut manager = get_manager().write().await;
            let mut results = Vec::with_capacity(texts.len());
            for text in texts {
                results.push(manager.embed(text).await?);
            }
            results
        }
        FileParserMode::SelfHosted => {
            let model = load_identity_model(&identity)?;
            let platform = crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM).clone();
            let client = SelfHostedEmbeddingClient::new(platform);
            client.embed_texts_self_hosted(&model, texts).await?
        }
        FileParserMode::Remote => {
            let model = load_identity_model(&identity)?;
            let platform = model_platform_repo::get_one(&identity.platform)
                .map_err(|e| AppError::ModelPlatformNotFound(format!("{}: {}", identity.platform, e)))?;
            let proxy = PROXY.read().await.clone();
            let client = RemoteEmbeddingClient::new(platform, proxy);
            client.embed_texts(&model, texts).await?
        }
    };
    assert_dim(&results, &identity)?;
    Ok(results)
}

/// Chunk text for embedding. Local mode chunks with the bundled tokenizer
/// (when the model files exist); every other backend falls back to character
/// chunking, which keeps lite (no local model files) fully functional.
pub async fn split_chunks(content: &str) -> Result<Vec<String>, AppError> {
    let mode = current_mode().await;
    if mode == FileParserMode::Local {
        let mut manager = get_manager().write().await;
        if manager.warmup().await.is_ok() {
            if let Some(service) = manager.service.as_ref() {
                if let Ok(chunks) = text_util::split_text(content, &service.tokenizer) {
                    return Ok(chunks);
                }
            }
        }
    }
    text_util::split_text_by_chars(content)
        .map_err(|e| AppError::DocumentSplitterError(e.to_string()))
}

fn mode_text(mode: &FileParserMode) -> &'static str {
    match mode {
        FileParserMode::Local => "local",
        FileParserMode::SelfHosted => "selfhosted",
        FileParserMode::Remote => "remote",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structs::sparse_vector::SparseVector;

    fn identity(platform: &str) -> EmbeddingIdentity {
        EmbeddingIdentity {
            platform: platform.to_string(),
            model: "m".to_string(),
            dim: 8,
            has_sparse: false,
        }
    }

    #[test]
    fn mode_identity_matrix() {
        let local = EmbeddingIdentity::local();
        assert!(identity_matches_mode(&local, &FileParserMode::Local));
        assert!(!identity_matches_mode(&identity("openai"), &FileParserMode::Local));

        // Self-hosted: identity platform must equal the active self-hosted name
        *crate::write_lock!(crate::global::ACTIVE_SELF_HOSTED_PLATFORM) =
            crate::entities::SelfHostedPlatform {
                name: "ollama".to_string(),
                ..Default::default()
            };
        assert!(identity_matches_mode(&identity("ollama"), &FileParserMode::SelfHosted));
        assert!(!identity_matches_mode(&identity("vllm"), &FileParserMode::SelfHosted));
        assert!(!identity_matches_mode(&local, &FileParserMode::SelfHosted));

        // Remote: neither the local marker nor the active self-hosted platform
        assert!(identity_matches_mode(&identity("openai"), &FileParserMode::Remote));
        assert!(!identity_matches_mode(&local, &FileParserMode::Remote));
        assert!(!identity_matches_mode(&identity("ollama"), &FileParserMode::Remote));
    }

    #[test]
    fn dim_guardrail_rejects_drift() {
        let id = identity("openai");
        let ok = EmbedResult {
            dense: vec![0.1; 8],
            sparse: SparseVector::default(),
        };
        assert!(assert_dim(std::slice::from_ref(&ok), &id).is_ok());
        let drifted = EmbedResult {
            dense: vec![0.1; 7],
            sparse: SparseVector::default(),
        };
        assert!(assert_dim(std::slice::from_ref(&drifted), &id).is_err());
    }
}

/// Sentinel text used to probe a candidate backend's dim before creating its
/// collection (dim is a property of the collection, discovered at runtime -
/// never hand-entered).
const PROBE_SENTINEL: &str = "mango finder embedding probe";

/// Probe a candidate backend identity: embeds the sentinel once with the given
/// platform+model and reads dim (and has_sparse) from the response. Used by the
/// settings-page switch flow before a collection is created.
pub async fn probe_identity(
    mode: &FileParserMode,
    platform: &str,
    model: &str,
) -> Result<EmbeddingIdentity, AppError> {
    match mode {
        FileParserMode::Local => {
            // Known statics for the bundled backend; probing the ONNX session
            // would load it needlessly
            Ok(EmbeddingIdentity::local())
        }
        FileParserMode::SelfHosted => {
            // The dispatcher only accepts the ACTIVE self-hosted platform name;
            // probing a different one would build an identity every later embed
            // rejects
            let active_name =
                crate::read_lock!(crate::global::ACTIVE_SELF_HOSTED_PLATFORM).name.clone();
            if platform != active_name {
                return Err(AppError::EmbeddingError(format!(
                    "platform '{platform}' is not the active self-hosted platform '{active_name}'"
                )));
            }
            let self_hosted = crate::repositories::self_hosted_platform_repo::get_one(platform)
                .map_err(|e| AppError::ModelPlatformNotFound(format!("{platform}: {e}")))?;
            let ai_model = ai_model_repo::get_one(platform, model)
                .map_err(|e| AppError::InternalError(e.to_string()))?
                .ok_or(AppError::AiModelNotFound(format!("{model} · {platform}")))?;
            let client = SelfHostedEmbeddingClient::new(self_hosted);
            let results = client
                .embed_texts_self_hosted(&ai_model, &[PROBE_SENTINEL.to_string()])
                .await?;
            let dim = results
                .first()
                .map(|r| r.dense.len())
                .ok_or(AppError::EmbeddingError("empty probe response".to_string()))?;
            Ok(EmbeddingIdentity {
                platform: platform.to_string(),
                model: model.to_string(),
                dim,
                has_sparse: false,
            })
        }
        FileParserMode::Remote => {
            let platform_row = model_platform_repo::get_one(platform)
                .map_err(|e| AppError::ModelPlatformNotFound(format!("{platform}: {e}")))?;
            let ai_model = ai_model_repo::get_one(platform, model)
                .map_err(|e| AppError::InternalError(e.to_string()))?
                .ok_or(AppError::AiModelNotFound(format!("{model} · {platform}")))?;
            let proxy = PROXY.read().await.clone();
            let client = RemoteEmbeddingClient::new(platform_row, proxy);
            let results = client
                .embed_texts(&ai_model, &[PROBE_SENTINEL.to_string()])
                .await?;
            let dim = results
                .first()
                .map(|r| r.dense.len())
                .ok_or(AppError::EmbeddingError("empty probe response".to_string()))?;
            Ok(EmbeddingIdentity {
                platform: platform.to_string(),
                model: model.to_string(),
                dim,
                has_sparse: false,
            })
        }
    }
}
