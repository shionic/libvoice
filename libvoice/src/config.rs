use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalyzerConfig {
    pub sample_rate: u32,
    pub frame_size: usize,
    pub hop_size: usize,
    pub min_pitch_hz: f32,
    pub max_pitch_hz: f32,
    pub pitch_clarity_threshold: f32,
    pub rolloff_ratio: f32,
    pub voiced_rms_threshold: f32,
    pub voiced_max_spectral_flatness: f32,
    pub voiced_max_zero_crossing_rate: f32,
    pub max_formants: usize,
    pub formant_max_frequency_hz: f32,
    pub formant_max_bandwidth_hz: f32,
    pub formant_pre_emphasis_hz: f32,
}

impl AnalyzerConfig {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            frame_size: 2048,
            hop_size: 512,
            min_pitch_hz: 60.0,
            max_pitch_hz: 500.0,
            pitch_clarity_threshold: 0.60,
            rolloff_ratio: 0.85,
            voiced_rms_threshold: 0.015,
            voiced_max_spectral_flatness: 0.45,
            voiced_max_zero_crossing_rate: 0.25,
            max_formants: 4,
            formant_max_frequency_hz: ((sample_rate as f32 * 0.5) - 50.0)
                .min(5_500.0)
                .max(1_500.0),
            formant_max_bandwidth_hz: 700.0,
            formant_pre_emphasis_hz: 50.0,
        }
    }

    pub fn frame_step_seconds(&self) -> f32 {
        self.hop_size as f32 / self.sample_rate as f32
    }

    pub(crate) fn lpc_order(&self) -> usize {
        (self.max_formants.saturating_mul(2) + 2).max(8)
    }
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self::new(16_000)
    }
}
