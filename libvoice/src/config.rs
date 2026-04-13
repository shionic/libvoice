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
    pub max_harmonic_frequency_hz: f32,
    pub harmonic_min_strength_ratio: f32,
}

impl AnalyzerConfig {
    pub fn new(sample_rate: u32) -> Self {
        let (frame_size, hop_size) = default_window_sizes(sample_rate);
        Self {
            sample_rate,
            frame_size,
            hop_size,
            min_pitch_hz: 60.0,
            max_pitch_hz: 500.0,
            pitch_clarity_threshold: 0.60,
            rolloff_ratio: 0.85,
            voiced_rms_threshold: 0.015,
            voiced_max_spectral_flatness: 0.45,
            voiced_max_zero_crossing_rate: 0.25,
            max_harmonic_frequency_hz: 5_000.0,
            harmonic_min_strength_ratio: 0.005,
        }
    }

    pub fn frame_step_seconds(&self) -> f32 {
        self.hop_size as f32 / self.sample_rate as f32
    }
}

fn default_window_sizes(sample_rate: u32) -> (usize, usize) {
    let frame_size = if sample_rate >= 44_100 {
        6_144
    } else if sample_rate >= 24_000 {
        4_096
    } else {
        2_048
    };

    (frame_size, frame_size / 4)
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self::new(16_000)
    }
}
