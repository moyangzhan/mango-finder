use crate::utils::app_util;
use crate::utils::audio_util;
use anyhow::Result;
use log::error;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Local transcription engine backed by whisper.cpp.
/// Supports multiple audio formats and auto-detects language.
///
/// 基于 whisper.cpp 的本地转写引擎。
/// 支持多种音频格式，自动检测语言。
pub struct LocalAudioParser {
    ctx: Arc<AsyncMutex<WhisperContext>>,
}

impl LocalAudioParser {
    pub fn new() -> Result<Self> {
        let model_path = app_util::get_whisper_model_path();
        let ctx_params = WhisperContextParameters::default();

        let ctx = WhisperContext::new_with_params(&model_path, ctx_params).map_err(|e| {
            error!("Failed to load whisper model from {}: {:?}", model_path, e);
            anyhow::anyhow!("Failed to load whisper model: {:?}", e)
        })?;

        Ok(Self {
            ctx: Arc::new(AsyncMutex::new(ctx)),
        })
    }

    /// Transcribe audio file to text.
    ///
    /// # Arguments
    /// - `audio_file`: Path to the audio file
    ///
    /// # Returns
    /// Transcribed text content
    ///
    /// 将音频文件转录为文本。
    ///
    /// # 参数
    /// - `audio_file`: 音频文件路径
    ///
    /// # 返回值
    /// 转录后的文本内容
    pub async fn parse(&self, audio_file: &str) -> Result<String> {
        let samples = audio_util::load_audio_to_f32(audio_file)?;
        self.parse_from_samples(&samples).await
    }

    /// Transcribe from pre-loaded audio samples (avoids re-decoding the file)
    /// 从已加载的音频采样点转录(避免重复解码文件)
    pub async fn parse_from_samples(&self, audio_samples: &[f32]) -> Result<String> {
        let mut full_transcript = String::new();

        let ctx = self.ctx.lock().await;
        let mut state = ctx.create_state().map_err(|e| {
            error!("Failed to create whisper state: {:?}", e);
            anyhow::anyhow!("Failed to create whisper state: {:?}", e)
        })?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_progress(false);
        params.set_print_timestamps(false);

        // Whisper 推理是重 CPU 计算,放 block_in_place 避免 tokio worker 被独占
        // Whisper inference is heavy CPU; block_in_place avoids starving the worker
        let (state, num_segments) = tokio::task::block_in_place(|| {
            state.full(params, audio_samples).map_err(|e| {
                error!("Whisper transcription failed: {:?}", e);
                anyhow::anyhow!("Whisper transcription failed: {:?}", e)
            })?;
            let n = state.full_n_segments();
            Ok::<_, anyhow::Error>((state, n))
        })?;

        for i in 0..num_segments {
            let segment = state.get_segment(i).ok_or_else(|| {
                error!("Failed to get segment {}", i);
                anyhow::anyhow!("Failed to get segment {}", i)
            })?;

            let text = segment.to_str().map_err(|e| {
                error!("Failed to get segment text: {:?}", e);
                anyhow::anyhow!("Failed to get segment text: {:?}", e)
            })?;

            full_transcript.push_str(text);
        }

        Ok(full_transcript)
    }
}
