use libvoice::{AnalysisReport, AnalyzerConfig, ChunkAnalysis, FrameAnalysis, VoiceAnalyzer};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzerConfigPatch {
    frame_size: Option<usize>,
    hop_size: Option<usize>,
    min_pitch_hz: Option<f32>,
    max_pitch_hz: Option<f32>,
    pitch_clarity_threshold: Option<f32>,
    rolloff_ratio: Option<f32>,
    voiced_rms_threshold: Option<f32>,
    voiced_max_spectral_flatness: Option<f32>,
    voiced_max_zero_crossing_rate: Option<f32>,
    max_formants: Option<usize>,
    formant_max_frequency_hz: Option<f32>,
    formant_max_bandwidth_hz: Option<f32>,
    formant_pre_emphasis_hz: Option<f32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamingChunkResult {
    chunk: ChunkAnalysis,
    frames: Vec<FrameAnalysis>,
}

#[wasm_bindgen(js_name = analyzeMonoF32)]
pub fn analyze_mono_f32(
    sample_rate: u32,
    samples: Vec<f32>,
    options: Option<JsValue>,
    include_frames: bool,
) -> Result<JsValue, JsValue> {
    let config = build_config(sample_rate, options)?;

    if include_frames {
        let mut analyzer = VoiceAnalyzer::new(config.clone());
        let (chunk, frames) = analyzer.process_chunk_with_frames(&samples);
        return to_js_value(&AnalyzeWithFrames {
            report: AnalysisReport {
                config,
                frames: frames.clone(),
                chunks: vec![chunk],
                overall: analyzer.finalize(),
            },
            frames,
        });
    }

    to_js_value(&VoiceAnalyzer::analyze_buffer(config, &samples))
}

#[wasm_bindgen]
pub struct VoiceAnalyzerStream {
    inner: VoiceAnalyzer,
}

#[wasm_bindgen]
impl VoiceAnalyzerStream {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: u32, options: Option<JsValue>) -> Result<VoiceAnalyzerStream, JsValue> {
        let config = build_config(sample_rate, options)?;
        Ok(Self {
            inner: VoiceAnalyzer::new(config),
        })
    }

    #[wasm_bindgen(js_name = config)]
    pub fn config_js(&self) -> Result<JsValue, JsValue> {
        to_js_value(self.inner.config())
    }

    #[wasm_bindgen(js_name = processChunk)]
    pub fn process_chunk(&mut self, samples: Vec<f32>) -> Result<JsValue, JsValue> {
        let (chunk, frames) = self.inner.process_chunk_with_frames(&samples);
        to_js_value(&StreamingChunkResult { chunk, frames })
    }

    pub fn finalize(&self) -> Result<JsValue, JsValue> {
        to_js_value(&self.inner.finalize())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeWithFrames {
    report: AnalysisReport,
    frames: Vec<FrameAnalysis>,
}

fn build_config(sample_rate: u32, options: Option<JsValue>) -> Result<AnalyzerConfig, JsValue> {
    if sample_rate == 0 {
        return Err(js_error("sampleRate must be greater than 0"));
    }

    let mut config = AnalyzerConfig::new(sample_rate);
    let patch = match options {
        Some(value) if !value.is_undefined() && !value.is_null() => {
            serde_wasm_bindgen::from_value::<AnalyzerConfigPatch>(value)
                .map_err(|error| js_error(format!("invalid analyzer options: {error}")))?
        }
        _ => AnalyzerConfigPatch::default(),
    };

    if let Some(frame_size) = patch.frame_size {
        if frame_size < 8 {
            return Err(js_error("frameSize must be at least 8"));
        }
        config.frame_size = frame_size;
    }
    if let Some(hop_size) = patch.hop_size {
        if hop_size == 0 {
            return Err(js_error("hopSize must be greater than 0"));
        }
        config.hop_size = hop_size;
    }
    if let Some(min_pitch_hz) = patch.min_pitch_hz {
        config.min_pitch_hz = min_pitch_hz;
    }
    if let Some(max_pitch_hz) = patch.max_pitch_hz {
        config.max_pitch_hz = max_pitch_hz;
    }
    if let Some(pitch_clarity_threshold) = patch.pitch_clarity_threshold {
        config.pitch_clarity_threshold = pitch_clarity_threshold;
    }
    if let Some(rolloff_ratio) = patch.rolloff_ratio {
        config.rolloff_ratio = rolloff_ratio;
    }
    if let Some(voiced_rms_threshold) = patch.voiced_rms_threshold {
        config.voiced_rms_threshold = voiced_rms_threshold;
    }
    if let Some(voiced_max_spectral_flatness) = patch.voiced_max_spectral_flatness {
        config.voiced_max_spectral_flatness = voiced_max_spectral_flatness;
    }
    if let Some(voiced_max_zero_crossing_rate) = patch.voiced_max_zero_crossing_rate {
        config.voiced_max_zero_crossing_rate = voiced_max_zero_crossing_rate;
    }
    if let Some(max_formants) = patch.max_formants {
        config.max_formants = max_formants;
    }
    if let Some(formant_max_frequency_hz) = patch.formant_max_frequency_hz {
        config.formant_max_frequency_hz = formant_max_frequency_hz;
    }
    if let Some(formant_max_bandwidth_hz) = patch.formant_max_bandwidth_hz {
        config.formant_max_bandwidth_hz = formant_max_bandwidth_hz;
    }
    if let Some(formant_pre_emphasis_hz) = patch.formant_pre_emphasis_hz {
        config.formant_pre_emphasis_hz = formant_pre_emphasis_hz;
    }

    if config.min_pitch_hz <= 0.0 || config.max_pitch_hz <= 0.0 {
        return Err(js_error("pitch bounds must be greater than 0"));
    }
    if config.min_pitch_hz >= config.max_pitch_hz {
        return Err(js_error("minPitchHz must be smaller than maxPitchHz"));
    }
    if config.hop_size > config.frame_size {
        return Err(js_error("hopSize must be less than or equal to frameSize"));
    }
    if config.max_formants == 0 {
        return Err(js_error("maxFormants must be greater than 0"));
    }

    Ok(config)
}

fn to_js_value<T>(value: &T) -> Result<JsValue, JsValue>
where
    T: Serialize,
{
    serde_wasm_bindgen::to_value(value)
        .map_err(|error| js_error(format!("failed to serialize result: {error}")))
}

fn js_error(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}
