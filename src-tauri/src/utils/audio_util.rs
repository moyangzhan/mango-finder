use crate::errors::AppError;
use crate::global::SUPPORTED_AUDIO_EXTS;
use crate::structs::file_metadata::AudioType;
use crate::utils::base64_util::file_to_data_uri;
use anyhow::{Result, anyhow};
use std::fs::File;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_codecs;
use symphonia::default::get_probe;

pub fn is_supported_audio_ext(ext: &str) -> bool {
    SUPPORTED_AUDIO_EXTS.contains(&ext)
}

/// Determine if a file is an audio file
/// Returns Ok(true) for an audio, Ok(false) for non-audio, and Err for file operation failure
pub fn is_supported_audio_file(path: &str) -> Result<bool, AppError> {
    if let Some(kind) = infer::get_from_path(path)? {
        let ext = kind.extension();
        if SUPPORTED_AUDIO_EXTS.contains(&ext) {
            return Ok(true);
        }
    }
    // No audio format matched
    Ok(false)
}

/// Convert audio file to Data URI
pub fn audio_to_data_uri(path: &str) -> Result<String, AppError> {
    let supported = is_supported_audio_file(path)?;
    if !supported {
        return Err(AppError::UnsupportedAudioFormat(path.to_string()));
    }
    let data_uri = file_to_data_uri(path)?;
    Ok(data_uri)
}

/// Load audio file and convert to f32 samples at 16kHz for whisper.cpp
pub fn load_audio_to_f32(path: &str) -> Result<Vec<f32>> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let probed = get_probe().format(
        &Default::default(),
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;

    let track = format.default_track().ok_or(anyhow!("no audio track"))?;
    let source_rate = track
        .codec_params
        .sample_rate
        .ok_or(anyhow!("Unknown rate"))?;
    let target_rate = 16000;
    let resample_ratio = source_rate as f32 / target_rate as f32;
    let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut audio_samples: Vec<f32> = Vec::new();
    let mut next_sample_pos: f32 = 0.0;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(_) => break,
        };
        let decoded = decoder.decode(&packet)?;
        let mut current_frame = Vec::new();

        match decoded {
            AudioBufferRef::F32(buf) => {
                for i in 0..buf.frames() {
                    let sum: f32 = (0..buf.spec().channels.count())
                        .map(|ch| buf.chan(ch)[i])
                        .sum();
                    current_frame.push(sum / buf.spec().channels.count() as f32);
                }
            }
            _ => {
                let spec = decoded.spec();
                let channels = spec.channels.count();
                let mut sbuf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                sbuf.copy_interleaved_ref(decoded);
                let samples = sbuf.samples();
                for i in 0..(samples.len() / channels) {
                    let sum: f32 = (0..channels).map(|ch| samples[i * channels + ch]).sum();
                    current_frame.push(sum / channels as f32);
                }
            }
        }

        // Resample to 16kHz
        let frame_data = current_frame;
        let frame_len = frame_data.len() as f32;
        let mut i = next_sample_pos;

        while i < frame_len {
            let idx = i as usize;
            let next_idx = (idx + 1).min((frame_len - 1.0) as usize);
            let frac = i - idx as f32;
            let interpolated_sample = frame_data[idx] * (1.0 - frac) + frame_data[next_idx] * frac;
            audio_samples.push(interpolated_sample);
            i += resample_ratio;
        }
        next_sample_pos = i - frame_len;
    }

    Ok(audio_samples)
}

/// Audio features for type classification
/// 用于类型分类的音频特征
#[derive(Debug, Clone)]
pub struct AudioFeatures {
    /// Zero crossing rate - speech typically has higher ZCR
    /// 过零率 - 语音通常有更高的过零率
    pub zero_crossing_rate: f32,
    /// Energy variance - music typically has lower variance (more stable)
    /// 能量方差 - 音乐通常有较低的方差（更稳定）
    pub energy_variance: f32,
    /// Average energy
    /// 平均能量
    pub avg_energy: f32,
    /// Spectral centroid mean - music typically has higher values
    /// 频谱质心均值 - 音乐通常有更高的值
    pub spectral_centroid: f32,
    /// RMS energy
    /// RMS 能量
    pub rms_energy: f32,
}

/// Extract audio features from samples
/// 从采样数据中提取音频特征
pub fn extract_audio_features(samples: &[f32]) -> AudioFeatures {
    if samples.is_empty() {
        return AudioFeatures {
            zero_crossing_rate: 0.0,
            energy_variance: 0.0,
            avg_energy: 0.0,
            spectral_centroid: 0.0,
            rms_energy: 0.0,
        };
    }

    // Calculate zero crossing rate
    // 计算过零率
    let mut zero_crossings = 0usize;
    for i in 1..samples.len() {
        if (samples[i - 1] >= 0.0 && samples[i] < 0.0)
            || (samples[i - 1] < 0.0 && samples[i] >= 0.0)
        {
            zero_crossings += 1;
        }
    }
    let zero_crossing_rate = zero_crossings as f32 / samples.len() as f32;

    // Calculate energy statistics
    // 计算能量统计
    let energies: Vec<f32> = samples.iter().map(|&s| s * s).collect();
    let avg_energy = energies.iter().sum::<f32>() / energies.len() as f32;

    // Calculate energy variance
    // 计算能量方差
    let energy_variance = if energies.len() > 1 {
        let mean = avg_energy;
        let variance: f32 =
            energies.iter().map(|&e| (e - mean).powi(2)).sum::<f32>() / (energies.len() - 1) as f32;
        variance
    } else {
        0.0
    };

    // Calculate RMS energy
    // 计算 RMS 能量
    let rms_energy = (energies.iter().sum::<f32>() / energies.len() as f32).sqrt();

    // Simple spectral centroid approximation using zero crossings
    // 使用过零率近似频谱质心（简化版本）
    let spectral_centroid = zero_crossing_rate * 16000.0 / 2.0; // Approximate based on sample rate

    AudioFeatures {
        zero_crossing_rate,
        energy_variance,
        avg_energy,
        spectral_centroid,
        rms_energy,
    }
}

/// Detect audio type based on audio features
/// 根据音频特征检测音频类型
///
/// Analysis logic:
/// - Music: Lower ZCR, lower energy variance (consistent beat), higher spectral centroid
/// - Speech: Higher ZCR (more transitions), higher energy variance (pauses between words)
///
/// 分析逻辑：
/// - 音乐：较低的过零率，较低的能量方差（稳定的节拍），较高的频谱质心
/// - 语音：较高的过零率（更多过渡），较高的能量方差（词语之间的停顿）
pub fn detect_audio_type_by_features(samples: &[f32]) -> AudioType {
    if samples.len() < 16000 {
        // Less than 1 second of audio
        // 少于1秒的音频
        return AudioType::Unknown;
    }

    let features = extract_audio_features(samples);

    // Analyze in segments to detect patterns
    // 分段分析以检测模式
    let segment_size = 16000; // 1 second segments
    let mut segment_zcrs: Vec<f32> = Vec::new();
    let mut segment_energies: Vec<f32> = Vec::new();

    for i in (0..samples.len()).step_by(segment_size) {
        let end = (i + segment_size).min(samples.len());
        let segment = &samples[i..end];
        if segment.len() >= 8000 {
            // At least 0.5 second
            let seg_features = extract_audio_features(segment);
            segment_zcrs.push(seg_features.zero_crossing_rate);
            segment_energies.push(seg_features.avg_energy);
        }
    }

    if segment_zcrs.len() < 2 {
        return AudioType::Unknown;
    }

    // Calculate variance of segment energies
    // 计算分段能量的方差
    let mean_energy: f32 = segment_energies.iter().sum::<f32>() / segment_energies.len() as f32;
    let energy_var: f32 = segment_energies
        .iter()
        .map(|&e| (e - mean_energy).powi(2))
        .sum::<f32>()
        / segment_energies.len() as f32;

    // Classification based on features
    // 基于特征分类
    //
    // Music characteristics:
    // - More consistent energy (lower energy variance across segments)
    // - Lower ZCR variance (more uniform sound)
    // - Higher average energy
    //
    // Speech characteristics:
    // - Variable energy (pauses between words/sentences)
    // - Higher ZCR (more transitions between voiced/unvoiced sounds)
    // - Higher ZCR variance (silence vs speech)

    let energy_cv = if mean_energy > 0.0001 {
        (energy_var.sqrt()) / mean_energy
    } else {
        0.0
    };

    // Music: energy coefficient of variation < 0.5, ZCR < 0.15
    // 音乐：能量变异系数 < 0.5，过零率 < 0.15
    if energy_cv < 0.5 && features.zero_crossing_rate < 0.15 {
        return AudioType::Music;
    }

    // Speech: energy CV > 0.7 or ZCR > 0.2
    // 语音：能量变异系数 > 0.7 或过零率 > 0.2
    if energy_cv > 0.7 || features.zero_crossing_rate > 0.2 {
        return AudioType::Speech;
    }

    // Mixed: between music and speech characteristics
    // 混合：介于音乐和语音特征之间
    if energy_cv > 0.3 && energy_cv < 0.7 && features.zero_crossing_rate > 0.1 {
        return AudioType::Mixed;
    }

    AudioType::Unknown
}

/// Detect audio type from file path
/// 从文件路径检测音频类型
pub fn detect_audio_type_from_file(path: &str) -> AudioType {
    match load_audio_to_f32(path) {
        Ok(samples) => detect_audio_type_by_features(&samples),
        Err(_) => AudioType::Unknown,
    }
}

/// Music fingerprint for similarity comparison
/// 用于相似性比较的音乐指纹
#[derive(Debug, Clone)]
pub struct MusicFingerprint {
    /// Spectral centroid histogram (10 bins)
    /// 频谱质心直方图（10个分箱）
    pub spectral_histogram: [f32; 10],
    /// Energy distribution across frequency bands (8 bands)
    /// 各频段的能量分布（8个频段）
    pub energy_bands: [f32; 8],
    /// Average zero crossing rate
    /// 平均过零率
    pub avg_zcr: f32,
    /// Tempo estimate (beats per minute approximation)
    /// 节拍估计（BPM近似值）
    pub tempo_estimate: f32,
}

impl MusicFingerprint {
    /// Convert fingerprint to bytes for database storage
    /// 将指纹转换为字节数组用于数据库存储
    /// Total: 20 f32 values = 80 bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(80);
        for &v in &self.spectral_histogram {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &self.energy_bands {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&self.avg_zcr.to_le_bytes());
        bytes.extend_from_slice(&self.tempo_estimate.to_le_bytes());
        bytes
    }

    /// Convert bytes from database to fingerprint
    /// 将数据库字节数组转换为指纹
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 80 {
            return None;
        }
        let mut spectral_histogram = [0.0f32; 10];
        let mut energy_bands = [0.0f32; 8];
        for i in 0..10 {
            spectral_histogram[i] = f32::from_le_bytes(bytes[i * 4..(i + 1) * 4].try_into().ok()?);
        }
        for i in 0..8 {
            energy_bands[i] =
                f32::from_le_bytes(bytes[40 + i * 4..40 + (i + 1) * 4].try_into().ok()?);
        }
        let avg_zcr = f32::from_le_bytes(bytes[72..76].try_into().ok()?);
        let tempo_estimate = f32::from_le_bytes(bytes[76..80].try_into().ok()?);
        Some(MusicFingerprint {
            spectral_histogram,
            energy_bands,
            avg_zcr,
            tempo_estimate,
        })
    }
}

/// Extract music fingerprint from audio samples
/// 从音频采样中提取音乐指纹
pub fn extract_music_fingerprint(samples: &[f32]) -> MusicFingerprint {
    let segment_size = 16000; // 1 second segments
    let mut spectral_sums = [0.0f32; 10];
    let mut energy_bands = [0.0f32; 8];
    let mut zcrs: Vec<f32> = Vec::new();
    let mut segment_count = 0usize;

    // Analyze in segments
    for i in (0..samples.len()).step_by(segment_size) {
        let end = (i + segment_size).min(samples.len());
        let segment = &samples[i..end];
        if segment.len() < 8000 {
            continue;
        }

        let features = extract_audio_features(segment);
        zcrs.push(features.zero_crossing_rate);

        // Build spectral histogram based on ZCR (simplified approximation)
        let spectral_bin = ((features.spectral_centroid / 4000.0).min(9.0) as usize).min(9);
        spectral_sums[spectral_bin] += 1.0;

        // Energy band distribution (simplified)
        let energy_total = features.avg_energy;
        for band in 0..8 {
            // Simulate frequency band energy distribution
            let band_factor = ((band + 1) as f32 / 8.0).powi(2);
            energy_bands[band] += energy_total * band_factor;
        }

        segment_count += 1;
    }

    // Normalize
    if segment_count > 0 {
        for i in 0..10 {
            spectral_sums[i] /= segment_count as f32;
        }
        for i in 0..8 {
            energy_bands[i] /= segment_count as f32;
        }
    }

    // Normalize energy bands
    let energy_sum: f32 = energy_bands.iter().sum();
    if energy_sum > 0.0 {
        for band in &mut energy_bands {
            *band /= energy_sum;
        }
    }

    // Normalize spectral histogram
    let spectral_sum: f32 = spectral_sums.iter().sum();
    if spectral_sum > 0.0 {
        for bin in &mut spectral_sums {
            *bin /= spectral_sum;
        }
    }

    let avg_zcr = if !zcrs.is_empty() {
        zcrs.iter().sum::<f32>() / zcrs.len() as f32
    } else {
        0.0
    };

    // Estimate tempo based on energy fluctuations
    let tempo_estimate = estimate_tempo(samples);

    MusicFingerprint {
        spectral_histogram: spectral_sums,
        energy_bands,
        avg_zcr,
        tempo_estimate,
    }
}

/// Estimate tempo from audio samples (simplified BPM estimation)
/// 从音频采样估计节拍（简化的BPM估计）
fn estimate_tempo(samples: &[f32]) -> f32 {
    if samples.len() < 32000 {
        return 0.0;
    }

    // Use energy envelope to detect beats
    let window_size = 1600; // 100ms at 16kHz
    let hop_size = 800; // 50ms hop
    let mut energy_envelope: Vec<f32> = Vec::new();

    for i in (0..samples.len() - window_size).step_by(hop_size) {
        let window = &samples[i..i + window_size];
        let energy: f32 = window.iter().map(|&s| s * s).sum::<f32>() / window_size as f32;
        energy_envelope.push(energy);
    }

    if energy_envelope.len() < 10 {
        return 0.0;
    }

    // Find peaks in energy envelope
    let mut peaks: Vec<usize> = Vec::new();
    let threshold = energy_envelope.iter().sum::<f32>() / energy_envelope.len() as f32 * 1.5;

    for i in 1..energy_envelope.len() - 1 {
        if energy_envelope[i] > threshold
            && energy_envelope[i] > energy_envelope[i - 1]
            && energy_envelope[i] > energy_envelope[i + 1]
        {
            peaks.push(i);
        }
    }

    if peaks.len() < 2 {
        return 0.0;
    }

    // Calculate average interval between peaks
    let mut intervals: Vec<f32> = Vec::new();
    for i in 1..peaks.len() {
        let interval_samples = (peaks[i] - peaks[i - 1]) * hop_size;
        let interval_seconds = interval_samples as f32 / 16000.0;
        intervals.push(interval_seconds);
    }

    if intervals.is_empty() {
        return 0.0;
    }

    // Filter out unrealistic intervals (too short or too long)
    let filtered_intervals: Vec<f32> = intervals
        .into_iter()
        .filter(|&i| i > 0.2 && i < 2.0) // Between 30 and 300 BPM
        .collect();

    if filtered_intervals.is_empty() {
        return 0.0;
    }

    let avg_interval: f32 =
        filtered_intervals.iter().sum::<f32>() / filtered_intervals.len() as f32;
    let bpm = 60.0 / avg_interval;

    bpm.clamp(30.0, 200.0)
}

/// Calculate similarity score between two music fingerprints
/// 计算两个音乐指纹之间的相似度分数
pub fn compare_music_fingerprints(fp1: &MusicFingerprint, fp2: &MusicFingerprint) -> f32 {
    // Compare spectral histograms using cosine similarity
    let mut spectral_dot = 0.0f32;
    let mut spectral_norm1 = 0.0f32;
    let mut spectral_norm2 = 0.0f32;

    for i in 0..10 {
        spectral_dot += fp1.spectral_histogram[i] * fp2.spectral_histogram[i];
        spectral_norm1 += fp1.spectral_histogram[i].powi(2);
        spectral_norm2 += fp2.spectral_histogram[i].powi(2);
    }

    let spectral_sim = if spectral_norm1 > 0.0 && spectral_norm2 > 0.0 {
        spectral_dot / (spectral_norm1.sqrt() * spectral_norm2.sqrt())
    } else {
        0.0
    };

    // Compare energy bands using cosine similarity
    let mut energy_dot = 0.0f32;
    let mut energy_norm1 = 0.0f32;
    let mut energy_norm2 = 0.0f32;

    for i in 0..8 {
        energy_dot += fp1.energy_bands[i] * fp2.energy_bands[i];
        energy_norm1 += fp1.energy_bands[i].powi(2);
        energy_norm2 += fp2.energy_bands[i].powi(2);
    }

    let energy_sim = if energy_norm1 > 0.0 && energy_norm2 > 0.0 {
        energy_dot / (energy_norm1.sqrt() * energy_norm2.sqrt())
    } else {
        0.0
    };

    // Compare ZCR
    let zcr_diff = (fp1.avg_zcr - fp2.avg_zcr).abs();
    let zcr_sim = 1.0 - (zcr_diff / 0.3).min(1.0);

    // Compare tempo
    let tempo_diff = (fp1.tempo_estimate - fp2.tempo_estimate).abs();
    let tempo_sim = if fp1.tempo_estimate > 0.0 && fp2.tempo_estimate > 0.0 {
        1.0 - (tempo_diff / 50.0).min(1.0) // 50 BPM difference = 0 similarity
    } else {
        0.5 // Unknown tempo gets neutral score
    };

    // Weighted combination
    let similarity = spectral_sim * 0.3 + energy_sim * 0.3 + zcr_sim * 0.2 + tempo_sim * 0.2;

    // Convert to 0-100 scale
    (similarity * 100.0).clamp(0.0, 100.0) as f32
}

/// Extract music fingerprint from file path
/// 从文件路径提取音乐指纹
pub fn extract_music_fingerprint_from_file(path: &str) -> Option<MusicFingerprint> {
    match load_audio_to_f32(path) {
        Ok(samples) => Some(extract_music_fingerprint(&samples)),
        Err(_) => None,
    }
}

/// Detect audio type using audio features (primary) and transcription (secondary)
/// 使用音频特征（主要）和转录内容（辅助）检测音频类型
pub fn detect_audio_type(transcription: &str, file_path: &str) -> AudioType {
    // First, try to detect based on audio features
    // 首先尝试基于音频特征检测
    let feature_based_type = detect_audio_type_from_file(file_path);

    // If feature detection gives a clear result, use it
    // 如果特征检测给出明确结果，使用它
    if feature_based_type != AudioType::Unknown {
        return feature_based_type;
    }

    // Fallback: use transcription content analysis
    // 回退：使用转录内容分析
    let transcription = transcription.trim();

    // If transcription is empty or very short, likely music
    // 如果转录为空或非常短，可能是音乐
    if transcription.is_empty() || transcription.len() < 20 {
        return AudioType::Music;
    }

    // Count words in transcription
    // 统计转录文本中的词数
    let word_count = transcription.split_whitespace().count();

    // If transcription has substantial content (more than 50 words), likely speech
    // 如果转录有实质性内容（超过50个词），可能是语音
    if word_count > 50 {
        return AudioType::Speech;
    }

    // Default to unknown
    // 默认为未知类型
    AudioType::Unknown
}
