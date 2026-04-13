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

    pub fn apply_high_pitch_mode(&mut self) {
        self.max_pitch_hz = 1_200.0;
        self.max_harmonic_frequency_hz = self.max_harmonic_frequency_hz.max(
            recommended_high_pitch_harmonic_cap_hz(
                self.sample_rate,
                self.frame_size,
                self.max_pitch_hz,
            ),
        );
        self.voiced_max_zero_crossing_rate = self
            .voiced_max_zero_crossing_rate
            .max(recommended_voiced_max_zero_crossing_rate(
                self.sample_rate,
                self.max_pitch_hz,
            ));
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

fn recommended_voiced_max_zero_crossing_rate(sample_rate: u32, max_pitch_hz: f32) -> f32 {
    if sample_rate == 0 || max_pitch_hz <= 0.0 {
        return 0.25;
    }

    (((2.0 * max_pitch_hz / sample_rate as f32) * 1.8) + 0.03).clamp(0.25, 0.40)
}

fn recommended_high_pitch_harmonic_cap_hz(
    sample_rate: u32,
    frame_size: usize,
    max_pitch_hz: f32,
) -> f32 {
    if sample_rate == 0 || frame_size == 0 || max_pitch_hz <= 0.0 {
        return 5_000.0;
    }

    let nyquist_hz = sample_rate as f32 * 0.5;
    let bin_hz = sample_rate as f32 / frame_size as f32;
    let desired_cap_hz = (max_pitch_hz * 6.0).max(5_000.0);
    desired_cap_hz.min((nyquist_hz - 2.0 * bin_hz).max(max_pitch_hz))
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self::new(16_000)
    }
}
