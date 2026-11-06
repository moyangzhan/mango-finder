use serde::{Deserialize, Serialize};

/// Identity of an embedding backend: platform + model name.
/// Compatibility of vector collections is judged by this pair only
/// (same pair = same vector space; dim/has_sparse are probed attributes,
/// recorded for display and guardrails, not part of identity equality).
///
/// 嵌入后端身份:平台 + 模型名。
/// 向量库兼容性只按此二元组判定(同对 = 同向量空间);
/// dim/has_sparse 是探测所得属性,仅用于展示与护栏断言,不参与身份判等。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingIdentity {
    /// "local" | self-hosted platform name ("ollama"/"vllm") | model platform name ("openai"...)
    pub platform: String,
    /// Model name, e.g. "bge-m3", "text-embedding-3-small"
    pub model: String,
    /// Dense vector dimension of this backend's output
    pub dim: usize,
    /// Whether the backend outputs a sparse vector component (BGE-M3 style)
    pub has_sparse: bool,
}

/// Platform value for the bundled local ONNX backend
pub const LOCAL_PLATFORM: &str = "local";
/// Display model name of the bundled local ONNX embedding model (file: embedding.onnx)
pub const LOCAL_MODEL: &str = "bge-m3";
/// Dense dim of the bundled local model (matches the shipped vec0 tables)
pub const LOCAL_DIM: usize = 1024;

impl EmbeddingIdentity {
    pub fn local() -> Self {
        EmbeddingIdentity {
            platform: LOCAL_PLATFORM.to_string(),
            model: LOCAL_MODEL.to_string(),
            dim: LOCAL_DIM,
            has_sparse: true,
        }
    }

    pub fn is_local(&self) -> bool {
        self.platform == LOCAL_PLATFORM
    }

    /// Identity equality: platform + model pair only.
    /// 身份判等:仅平台 + 模型。
    pub fn same_backend(&self, other: &EmbeddingIdentity) -> bool {
        self.platform == other.platform && self.model == other.model
    }
}

impl Default for EmbeddingIdentity {
    fn default() -> Self {
        EmbeddingIdentity::local()
    }
}
