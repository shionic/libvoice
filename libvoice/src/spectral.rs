use crate::config::AnalyzerConfig;
use crate::harmonic::HarmonicAnalyzer;
use crate::model::FrameFeatures;
use crate::signal::{PitchAnalyzer, estimate_hnr_db, estimate_loudness_dbfs, zero_crossing_rate};
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex32;
use std::sync::Arc;

const TILT_MIN_FREQUENCY_HZ: f32 = 80.0;
const TILT_MAX_FREQUENCY_HZ: f32 = 5_000.0;
const TILT_PEAK_FLOOR_DB: f32 = 40.0;

pub(crate) struct FrameAnalyzer {
    config: AnalyzerConfig,
    fft: Arc<dyn RealToComplex<f32>>,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex32>,
    magnitudes: Vec<f32>,
    pitch_analyzer: PitchAnalyzer,
    harmonic_analyzer: HarmonicAnalyzer,
    window: Vec<f32>,
    bin_hz: f32,
}

impl FrameAnalyzer {
    pub(crate) fn new(config: AnalyzerConfig, window: Vec<f32>) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(config.frame_size);
        let fft_input = fft.make_input_vec();
        let fft_output = fft.make_output_vec();
        let bin_hz = config.sample_rate as f32 / config.frame_size as f32;

        Self {
            config,
            fft,
            fft_input,
            fft_output,
            magnitudes: Vec::new(),
            pitch_analyzer: PitchAnalyzer::new(),
            harmonic_analyzer: HarmonicAnalyzer::new(),
            window,
            bin_hz,
        }
    }

    pub(crate) fn analyze(&mut self, frame: &[f32]) -> FrameFeatures {
        debug_assert_eq!(frame.len(), self.config.frame_size);

        let mut energy_sum = 0.0_f32;
        let mut trailing_energy_sum = 0.0_f32;
        let trailing_start = frame.len() / 2;
        for ((slot, sample), window) in self
            .fft_input
            .iter_mut()
            .zip(frame.iter().copied())
            .zip(self.window.iter().copied())
        {
            *slot = sample * window;
            energy_sum += sample * sample;
        }
        for sample in frame.iter().copied().skip(trailing_start) {
            trailing_energy_sum += sample * sample;
        }

        self.fft
            .process(&mut self.fft_input, &mut self.fft_output)
            .expect("fft processing must succeed for pre-allocated buffers");

        let energy = energy_sum / frame.len() as f32;
        let rms = energy.sqrt();
        let trailing_rms = if trailing_start < frame.len() {
            (trailing_energy_sum / (frame.len() - trailing_start) as f32).sqrt()
        } else {
            rms
        };
        let zcr = zero_crossing_rate(frame);

        self.magnitudes.resize(self.fft_output.len(), 0.0);
        let mut magnitude_sum = 0.0_f32;
        let mut weighted_sum = 0.0_f32;
        let mut power_sum = 0.0_f32;
        let mut log_sum = 0.0_f32;
        let mut rolloff_hz = 0.0_f32;
        for (index, bin) in self.fft_output.iter().enumerate() {
            let magnitude = (bin.re.mul_add(bin.re, bin.im * bin.im))
                .sqrt()
                .max(1.0e-12);
            self.magnitudes[index] = magnitude;
            let power = magnitude * magnitude;
            let hz = index as f32 * self.bin_hz;
            magnitude_sum += magnitude;
            power_sum += power;
            weighted_sum += hz * magnitude;
            log_sum += power.ln();
        }

        let centroid = if magnitude_sum > 0.0 {
            weighted_sum / magnitude_sum
        } else {
            0.0
        };

        let mut bandwidth_sum = 0.0_f32;
        let threshold = power_sum * self.config.rolloff_ratio.clamp(0.0, 1.0);
        let mut cumulative = 0.0_f32;
        for (index, magnitude) in self.magnitudes.iter().copied().enumerate() {
            let hz = index as f32 * self.bin_hz;
            let diff = hz - centroid;
            let power = magnitude * magnitude;
            bandwidth_sum += magnitude * diff * diff;
            cumulative += power;
            if rolloff_hz == 0.0 && cumulative >= threshold {
                rolloff_hz = hz;
            }
        }

        let flatness = if power_sum > 0.0 && !self.fft_output.is_empty() {
            (log_sum / self.fft_output.len() as f32).exp()
                / (power_sum / self.fft_output.len() as f32)
        } else {
            0.0
        };

        let bandwidth = if magnitude_sum > 0.0 {
            (bandwidth_sum / magnitude_sum).sqrt()
        } else {
            0.0
        };

        let spectral_tilt_db_per_octave =
            estimate_spectral_tilt_db_per_octave(&self.magnitudes, self.bin_hz);

        let pitch = self.pitch_analyzer.estimate_pitch_hz(
            frame,
            self.config.sample_rate,
            self.config.min_pitch_hz,
            self.config.max_pitch_hz,
            self.config.pitch_clarity_threshold,
        );
        let pitch_hz = pitch.map(|estimate| estimate.hz);
        let loudness_dbfs = estimate_loudness_dbfs(rms);
        let hnr_db = estimate_hnr_db(pitch.map(|estimate| estimate.periodicity).unwrap_or(0.0));
        let harmonic_strengths = self.harmonic_analyzer.estimate(
            &self.magnitudes,
            self.bin_hz,
            pitch_hz,
            self.config.max_harmonic_frequency_hz,
            self.config.harmonic_min_strength_ratio,
        );
        FrameFeatures {
            pitch_hz,
            pitch_clarity: pitch.map(|estimate| estimate.clarity).unwrap_or(0.0),
            spectral_rolloff_hz: if rolloff_hz > 0.0 {
                rolloff_hz
            } else {
                self.fft_output.len().saturating_sub(1) as f32 * self.bin_hz
            },
            spectral_centroid_hz: centroid,
            spectral_bandwidth_hz: bandwidth,
            spectral_flatness: flatness,
            spectral_tilt_db_per_octave,
            zcr,
            rms,
            loudness_dbfs,
            hnr_db,
            energy,
            harmonic_strengths,
            trailing_rms,
        }
    }

    pub(crate) fn magnitudes(&self) -> &[f32] {
        &self.magnitudes
    }
}

fn estimate_spectral_tilt_db_per_octave(magnitudes: &[f32], bin_hz: f32) -> f32 {
    if magnitudes.len() < 3 || bin_hz <= 0.0 {
        return 0.0;
    }

    let nyquist_hz = (magnitudes.len().saturating_sub(1)) as f32 * bin_hz;
    let max_frequency_hz = TILT_MAX_FREQUENCY_HZ.min(nyquist_hz);
    if max_frequency_hz <= TILT_MIN_FREQUENCY_HZ {
        return 0.0;
    }

    let mut peak_power = 0.0_f32;
    for (index, magnitude) in magnitudes.iter().copied().enumerate().skip(1) {
        let hz = index as f32 * bin_hz;
        if hz < TILT_MIN_FREQUENCY_HZ || hz > max_frequency_hz {
            continue;
        }
        peak_power = peak_power.max(magnitude * magnitude);
    }

    if peak_power <= 1.0e-12 {
        return 0.0;
    }

    let power_floor = peak_power * 10.0_f32.powf(-TILT_PEAK_FLOOR_DB / 10.0);
    let mut weight_sum = 0.0_f32;
    let mut x_mean = 0.0_f32;
    let mut y_mean = 0.0_f32;
    let mut selected_bins = 0usize;

    for (index, magnitude) in magnitudes.iter().copied().enumerate().skip(1) {
        let hz = index as f32 * bin_hz;
        if hz < TILT_MIN_FREQUENCY_HZ || hz > max_frequency_hz {
            continue;
        }

        let power = magnitude * magnitude;
        if power < power_floor {
            continue;
        }

        let weight = power;
        let x = hz.log2();
        let y = 20.0 * magnitude.max(1.0e-12).log10();
        weight_sum += weight;
        x_mean += weight * x;
        y_mean += weight * y;
        selected_bins += 1;
    }

    if selected_bins < 3 || weight_sum <= 1.0e-12 {
        return 0.0;
    }

    x_mean /= weight_sum;
    y_mean /= weight_sum;

    let mut covariance = 0.0_f32;
    let mut variance = 0.0_f32;
    for (index, magnitude) in magnitudes.iter().copied().enumerate().skip(1) {
        let hz = index as f32 * bin_hz;
        if hz < TILT_MIN_FREQUENCY_HZ || hz > max_frequency_hz {
            continue;
        }

        let power = magnitude * magnitude;
        if power < power_floor {
            continue;
        }

        let x = hz.log2();
        let y = 20.0 * magnitude.max(1.0e-12).log10();
        let centered_x = x - x_mean;
        covariance += power * centered_x * (y - y_mean);
        variance += power * centered_x * centered_x;
    }

    if variance <= 1.0e-6 {
        0.0
    } else {
        covariance / variance
    }
}

#[cfg(test)]
mod tests {
    use super::estimate_spectral_tilt_db_per_octave;

    #[test]
    fn flat_spectrum_has_near_zero_tilt() {
        let magnitudes = vec![1.0_f32; 513];
        let tilt = estimate_spectral_tilt_db_per_octave(&magnitudes, 15.625);
        assert!(tilt.abs() < 0.1, "tilt={tilt}");
    }

    #[test]
    fn decaying_spectrum_has_negative_tilt() {
        let magnitudes = (0..513)
            .map(|index| {
                let hz = index as f32 * 15.625;
                if hz <= 0.0 {
                    1.0
                } else {
                    1.0 / hz.sqrt()
                }
            })
            .collect::<Vec<_>>();
        let tilt = estimate_spectral_tilt_db_per_octave(&magnitudes, 15.625);
        assert!(tilt < -1.0, "tilt={tilt}");
    }
}
