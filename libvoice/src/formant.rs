use crate::config::AnalyzerConfig;
use crate::model::FormantFrame;
use crate::signal::fill_downsampled;
use rustfft::num_complex::Complex32;
use std::f32::consts::PI;

const TARGET_FORMANT_SAMPLE_RATE: u32 = 11_000;

#[derive(Debug, Default)]
pub(crate) struct FormantAnalyzer {
    reduced: Vec<f32>,
    autocorrelation: Vec<f32>,
    lpc: Vec<f32>,
    roots: Vec<Complex32>,
    formants: Vec<FormantFrame>,
}

impl FormantAnalyzer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn estimate(&mut self, frame: &[f32], config: &AnalyzerConfig) -> Vec<FormantFrame> {
        let downsample = (config.sample_rate / TARGET_FORMANT_SAMPLE_RATE).max(1) as usize;
        let reduced_len = frame.len() / downsample;
        let order = config.lpc_order();
        if reduced_len <= order + 2 {
            return Vec::new();
        }

        self.reduced.resize(reduced_len, 0.0);
        let reduced = &mut self.reduced[..reduced_len];
        fill_downsampled(frame, downsample, reduced);

        let mean = reduced.iter().copied().sum::<f32>() / reduced_len as f32;
        for sample in reduced.iter_mut() {
            *sample -= mean;
        }

        let pre_emphasis =
            (-2.0 * PI * config.formant_pre_emphasis_hz / config.sample_rate as f32).exp();
        let mut previous = 0.0_f32;
        for sample in reduced.iter_mut() {
            let current = *sample;
            *sample = current - pre_emphasis * previous;
            previous = current;
        }

        apply_hamming_window(reduced);

        self.autocorrelation.resize(order + 1, 0.0);
        for lag in 0..=order {
            let mut value = 0.0_f32;
            for index in 0..(reduced_len - lag) {
                value += reduced[index] * reduced[index + lag];
            }
            self.autocorrelation[lag] = value;
        }

        let residual = levinson_durbin(&self.autocorrelation, order, &mut self.lpc);
        if residual <= 1.0e-9 {
            return Vec::new();
        }

        self.roots.clear();
        find_polynomial_roots(&self.lpc, &mut self.roots);

        let effective_sample_rate = config.sample_rate / downsample as u32;
        let nyquist = effective_sample_rate as f32 * 0.5;
        let max_frequency = config.formant_max_frequency_hz.min(nyquist - 50.0).max(0.0);

        self.formants.clear();
        self.formants.extend(self.roots.iter().filter_map(|root| {
            if root.im <= 0.0 {
                return None;
            }

            let radius = root.norm();
            if !(0.0..1.0).contains(&radius) {
                return None;
            }

            let frequency_hz = root.arg() * effective_sample_rate as f32 / (2.0 * PI);
            if frequency_hz < 90.0 || frequency_hz > max_frequency {
                return None;
            }

            let bandwidth_hz = -(effective_sample_rate as f32 / PI) * radius.ln();
            if !bandwidth_hz.is_finite()
                || bandwidth_hz <= 0.0
                || bandwidth_hz > config.formant_max_bandwidth_hz
            {
                return None;
            }

            Some(FormantFrame {
                frequency_hz,
                bandwidth_hz,
            })
        }));

        self.formants
            .sort_by(|left, right| left.frequency_hz.total_cmp(&right.frequency_hz));
        self.formants.truncate(config.max_formants);
        self.formants.clone()
    }
}

fn apply_hamming_window(samples: &mut [f32]) {
    let denom = (samples.len().saturating_sub(1)).max(1) as f32;
    for (index, sample) in samples.iter_mut().enumerate() {
        let window = 0.54 - 0.46 * (2.0 * PI * index as f32 / denom).cos();
        *sample *= window;
    }
}

fn levinson_durbin(autocorrelation: &[f32], order: usize, lpc: &mut Vec<f32>) -> f32 {
    lpc.clear();
    lpc.resize(order + 1, 0.0);
    lpc[0] = 1.0;

    if autocorrelation.len() <= order || autocorrelation[0] <= 1.0e-9 {
        return 0.0;
    }

    let mut error = autocorrelation[0];
    let mut next = vec![0.0_f32; order + 1];
    next[0] = 1.0;

    for i in 1..=order {
        let mut reflection = autocorrelation[i];
        for j in 1..i {
            reflection += lpc[j] * autocorrelation[i - j];
        }
        reflection = -reflection / error.max(1.0e-9);

        next[..=i].copy_from_slice(&lpc[..=i]);
        next[i] = reflection;
        for j in 1..i {
            next[j] = lpc[j] + reflection * lpc[i - j];
        }
        lpc[..=i].copy_from_slice(&next[..=i]);

        error *= 1.0 - reflection * reflection;
        if error <= 1.0e-9 {
            return 0.0;
        }
    }

    error
}

fn find_polynomial_roots(coefficients: &[f32], output: &mut Vec<Complex32>) {
    let degree = coefficients.len().saturating_sub(1);
    output.clear();
    if degree == 0 {
        return;
    }

    output.extend((0..degree).map(|index| {
        let angle = 2.0 * PI * index as f32 / degree as f32;
        Complex32::new(angle.cos(), angle.sin())
    }));

    for _ in 0..128 {
        let mut converged = true;
        for index in 0..degree {
            let root = output[index];
            let mut denominator = Complex32::new(1.0, 0.0);
            for (other_index, other) in output.iter().copied().enumerate() {
                if other_index != index {
                    denominator *= root - other;
                }
            }

            if denominator.norm() <= 1.0e-12 {
                converged = false;
                continue;
            }

            let correction = evaluate_polynomial(coefficients, root) / denominator;
            output[index] -= correction;
            if correction.norm() > 1.0e-5 {
                converged = false;
            }
        }

        if converged {
            break;
        }
    }
}

fn evaluate_polynomial(coefficients: &[f32], x: Complex32) -> Complex32 {
    coefficients
        .iter()
        .copied()
        .fold(Complex32::new(0.0, 0.0), |acc, coeff| {
            acc * x + Complex32::new(coeff, 0.0)
        })
}
