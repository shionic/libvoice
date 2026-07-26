use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    message: &'static str,
}

impl ConfigError {
    fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ConfigError {}

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
        self.max_harmonic_frequency_hz =
            self.max_harmonic_frequency_hz
                .max(recommended_high_pitch_harmonic_cap_hz(
                    self.sample_rate,
                    self.frame_size,
                    self.max_pitch_hz,
                ));
        self.voiced_max_zero_crossing_rate =
            self.voiced_max_zero_crossing_rate
                .max(recommended_voiced_max_zero_crossing_rate(
                    self.sample_rate,
                    self.max_pitch_hz,
                ));
    }

    pub fn frame_step_seconds(&self) -> f32 {
        self.hop_size as f32 / self.sample_rate as f32
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.sample_rate == 0 {
            return Err(ConfigError::new("sample_rate must be greater than zero"));
        }
        if self.frame_size < 3 {
            return Err(ConfigError::new("frame_size must be at least 3"));
        }
        if self.hop_size == 0 || self.hop_size > self.frame_size {
            return Err(ConfigError::new(
                "hop_size must be between 1 and frame_size",
            ));
        }
        if !self.min_pitch_hz.is_finite()
            || !self.max_pitch_hz.is_finite()
            || self.min_pitch_hz <= 0.0
            || self.min_pitch_hz >= self.max_pitch_hz
        {
            return Err(ConfigError::new(
                "pitch bounds must be finite, positive, and increasing",
            ));
        }
        if self.max_pitch_hz >= self.sample_rate as f32 * 0.5 {
            return Err(ConfigError::new("max_pitch_hz must be below Nyquist"));
        }
        let required_pitch_frame =
            (self.sample_rate as f32 / self.min_pitch_hz).ceil() as usize + 2;
        if self.frame_size < required_pitch_frame {
            return Err(ConfigError::new("frame_size is too short for min_pitch_hz"));
        }
        for (value, message) in [
            (
                self.pitch_clarity_threshold,
                "pitch_clarity_threshold must be between 0 and 1",
            ),
            (self.rolloff_ratio, "rolloff_ratio must be between 0 and 1"),
            (
                self.voiced_max_spectral_flatness,
                "voiced_max_spectral_flatness must be between 0 and 1",
            ),
            (
                self.voiced_max_zero_crossing_rate,
                "voiced_max_zero_crossing_rate must be between 0 and 1",
            ),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(ConfigError::new(message));
            }
        }
        if !self.voiced_rms_threshold.is_finite() || self.voiced_rms_threshold < 0.0 {
            return Err(ConfigError::new(
                "voiced_rms_threshold must be finite and non-negative",
            ));
        }
        if !self.max_harmonic_frequency_hz.is_finite() || self.max_harmonic_frequency_hz <= 0.0 {
            return Err(ConfigError::new(
                "max_harmonic_frequency_hz must be finite and positive",
            ));
        }
        if !self.harmonic_min_strength_ratio.is_finite() || self.harmonic_min_strength_ratio < 0.0 {
            return Err(ConfigError::new(
                "harmonic_min_strength_ratio must be finite and non-negative",
            ));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::AnalyzerConfig;

    #[test]
    fn rejects_configurations_that_could_hang_or_panic() {
        let mut config = AnalyzerConfig::new(16_000);
        config.hop_size = 0;
        assert!(config.validate().is_err());

        let mut config = AnalyzerConfig::new(16_000);
        config.frame_size = 0;
        assert!(config.validate().is_err());

        let mut config = AnalyzerConfig::new(16_000);
        config.sample_rate = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_non_finite_and_inconsistent_ranges() {
        let mut config = AnalyzerConfig::new(16_000);
        config.min_pitch_hz = f32::NAN;
        assert!(config.validate().is_err());

        let mut config = AnalyzerConfig::new(16_000);
        config.min_pitch_hz = config.max_pitch_hz;
        assert!(config.validate().is_err());

        let mut config = AnalyzerConfig::new(16_000);
        config.rolloff_ratio = f32::INFINITY;
        assert!(config.validate().is_err());
    }
}
