use crate::config::AnalyzerConfig;
use crate::formant::{FormantAnalyzer, FormantTracker};
use crate::model::FrameFeatures;
use crate::signal::{PitchAnalyzer, estimate_hnr_db, estimate_loudness_dbfs, zero_crossing_rate};
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex32;
use std::sync::Arc;

pub(crate) struct FrameAnalyzer {
    config: AnalyzerConfig,
    fft: Arc<dyn RealToComplex<f32>>,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex32>,
    magnitudes: Vec<f32>,
    pitch_analyzer: PitchAnalyzer,
    formant_analyzer: FormantAnalyzer,
    formant_tracker: FormantTracker,
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
            formant_analyzer: FormantAnalyzer::new(),
            formant_tracker: FormantTracker::new(),
            window,
            bin_hz,
        }
    }

    pub(crate) fn analyze(&mut self, frame: &[f32]) -> FrameFeatures {
        debug_assert_eq!(frame.len(), self.config.frame_size);

        let mut energy_sum = 0.0_f32;
        for ((slot, sample), window) in self
            .fft_input
            .iter_mut()
            .zip(frame.iter().copied())
            .zip(self.window.iter().copied())
        {
            *slot = sample * window;
            energy_sum += sample * sample;
        }

        self.fft
            .process(&mut self.fft_input, &mut self.fft_output)
            .expect("fft processing must succeed for pre-allocated buffers");

        let energy = energy_sum / frame.len() as f32;
        let rms = energy.sqrt();
        let zcr = zero_crossing_rate(frame);

        self.magnitudes.resize(self.fft_output.len(), 0.0);
        let mut magnitude_sum = 0.0_f32;
        let mut weighted_sum = 0.0_f32;
        let mut power_sum = 0.0_f32;
        let mut log_sum = 0.0_f32;
        let mut rolloff_hz = 0.0_f32;
        let mut tilt_count = 0usize;
        let mut sum_log_hz = 0.0_f32;
        let mut sum_db = 0.0_f32;
        let mut sum_log_hz_sq = 0.0_f32;
        let mut sum_log_hz_db = 0.0_f32;

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

            if index > 0 && hz > 0.0 {
                let log_hz = hz.log2();
                let db = 20.0 * magnitude.log10();
                tilt_count += 1;
                sum_log_hz += log_hz;
                sum_db += db;
                sum_log_hz_sq += log_hz * log_hz;
                sum_log_hz_db += log_hz * db;
            }
        }

        let centroid = if magnitude_sum > 0.0 {
            weighted_sum / magnitude_sum
        } else {
            0.0
        };

        let mut bandwidth_sum = 0.0_f32;
        let threshold = magnitude_sum * self.config.rolloff_ratio.clamp(0.0, 1.0);
        let mut cumulative = 0.0_f32;
        for (index, magnitude) in self.magnitudes.iter().copied().enumerate() {
            let hz = index as f32 * self.bin_hz;
            let diff = hz - centroid;
            bandwidth_sum += magnitude * diff * diff;
            cumulative += magnitude;
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

        let spectral_tilt_db_per_octave = if tilt_count >= 2 {
            let count = tilt_count as f32;
            let denominator = count * sum_log_hz_sq - sum_log_hz * sum_log_hz;
            if denominator.abs() > 1.0e-6 {
                (count * sum_log_hz_db - sum_log_hz * sum_db) / denominator
            } else {
                0.0
            }
        } else {
            0.0
        };

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
        let detected_formants = self.formant_analyzer.estimate(frame, &self.config);
        let formants = self
            .formant_tracker
            .track(&detected_formants, self.config.max_formants);
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
            formants,
        }
    }
}
