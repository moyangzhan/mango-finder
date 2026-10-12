use crate::errors::AppError;
use crate::global::EMBEDDING_MODEL_NAME;
use crate::structs::embed_result::EmbedResult;
use crate::utils::app_util::{get_multilingual_embedding_path, get_multilingual_tokenizer_path};
use log::{error, info};
use ort::{
    session::{Session, SessionInputs, builder::GraphOptimizationLevel},
    value::Value,
};
use std::collections::HashMap;
use std::sync::Mutex;
use tokenizers::Tokenizer;

struct ThreadSafeSession {
    session: Mutex<ort::session::Session>,
}

pub struct EmbeddingService {
    session: ThreadSafeSession,
    pub tokenizer: Tokenizer,
}

impl EmbeddingService {
    pub async fn new() -> Result<Self, AppError> {
        info!("Initializing embedding service...");
        let (model_path, tokenizer_path) = {
            let multilingual_embedding_path = get_multilingual_embedding_path();
            (
                multilingual_embedding_path,
                get_multilingual_tokenizer_path(),
            )
        };
        let logical_cores = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(2).max(2))
            .unwrap_or(2);
        info!("using {} threads", logical_cores);
        let session = Session::builder()
            .map_err(|e| AppError::ModelServiceInitError(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| AppError::ModelServiceInitError(e.to_string()))?
            .with_intra_threads(logical_cores)
            .map_err(|e| AppError::ModelServiceInitError(e.to_string()))?
            .commit_from_file(model_path)
            .map_err(|e| {
                error!("Failed to load model: {:?}", e);
                AppError::ModelServiceInitError(e.to_string())
            })?;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| {
            AppError::ModelServiceInitError(format!("Failed to load tokenizer: {}", e))
        })?;
        info!("EmbeddingService::new succeeded");
        Ok(EmbeddingService {
            session: ThreadSafeSession {
                session: Mutex::new(session),
            },
            tokenizer,
        })
    }

    pub async fn model_name() -> &'static str {
        EMBEDDING_MODEL_NAME
    }

    pub fn embed(&self, text: &str) -> Result<EmbedResult, AppError> {
        let encoding = self.tokenizer.encode(text, true)?;
        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&id| id as i64)
            .collect();

        let input_ids_len = input_ids.len();
        let input_tensor = Value::from_array(ndarray::Array::from_shape_vec(
            (1, input_ids_len),
            input_ids.clone(),
        )?)?;
        let attention_tensor = Value::from_array(ndarray::Array::from_shape_vec(
            (1, attention_mask.len()),
            attention_mask,
        )?)?;
        let mut guard = self.session.session.lock().unwrap_or_else(|e| {
            error!("Session mutex poisoned, recovering: {}", e);
            e.into_inner()
        });
        let mut input_val = HashMap::new();
        input_val.insert("input_ids", input_tensor);
        input_val.insert("attention_mask", attention_tensor);
        let needs_token_type = (*guard)
            .inputs()
            .iter()
            .any(|input| input.name() == "token_type_ids");
        if needs_token_type {
            let token_type_ids = vec![0i64; input_ids_len];
            let token_type_tensor = Value::from_array(ndarray::Array::from_shape_vec(
                (1, token_type_ids.len()),
                token_type_ids,
            )?)?;
            input_val.insert("token_type_ids", token_type_tensor);
        }
        let inputs = SessionInputs::from(input_val);
        let outputs: ort::session::SessionOutputs<'_> = (*guard).run(inputs)?;
        Ok(EmbedResult::from_outputs(&outputs, &input_ids).unwrap_or_default())
    }
}

impl Drop for EmbeddingService {
    fn drop(&mut self) {
        info!("Embedding Service is dropped");
    }
}
