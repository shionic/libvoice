use core::f32::consts::PI;
use rustfft::num_complex::Complex64;

const DEFAULT_FORMANT_CEILING_HZ: f32 = 5_500.0;
const PRAAT_WINDOW_LENGTH_SECONDS: f32 = 0.050;
const PRAAT_PRE_EMPHASIS_FROM_HZ: f32 = 50.0;
const PRAAT_MAX_FORMANTS: usize = 5;
const FORMANT_TRACK_ORDER_MARGIN_HZ: f32 = 25.0;
const FORMANT_SLOT_RANGES_HZ: [(f32, f32); 4] = [
    (150.0, 1_200.0),
    (500.0, 3_500.0),
    (1_200.0, 4_500.0),
    (2_000.0, 5_450.0),
];
const FORMANT_SLOT_TARGETS_HZ: [f32; 4] = [500.0, 1_700.0, 2_800.0, 4_000.0];

#[derive(Debug, Clone, Copy)]
pub(crate) struct PitchEstimate {
    pub(crate) hz: f32,
    pub(crate) clarity: f32,
    pub(crate) periodicity: f32,
}

#[derive(Debug, Default)]
pub(crate) struct PitchAnalyzer {
    centered: Vec<f32>,
    difference: Vec<f32>,
    cmndf: Vec<f32>,
}

#[derive(Debug)]
pub(crate) struct FormantAnalyzer {
    input_sample_rate: u32,
    effective_sample_rate: u32,
    formant_ceiling_hz: f32,
    formant_window_len: usize,
    previous_formants_hz: [Option<f32>; 4],
    analysis: Vec<f32>,
    resampled: Vec<f32>,
    window: Vec<f32>,
}

impl PitchAnalyzer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn estimate_pitch_hz(
        &mut self,
        frame: &[f32],
        sample_rate: u32,
        min_pitch_hz: f32,
        max_pitch_hz: f32,
        clarity_threshold: f32,
    ) -> Option<PitchEstimate> {
        const TARGET_PITCH_SAMPLE_RATE: u32 = 16_000;

        let downsample = (sample_rate / TARGET_PITCH_SAMPLE_RATE).max(1) as usize;
        let reduced_len = frame.len() / downsample;
        if reduced_len < 3 {
            return None;
        }

        self.centered.resize(reduced_len, 0.0);
        let reduced = &mut self.centered[..reduced_len];
        fill_downsampled(frame, downsample, reduced);
        let reduced_sum: f32 = reduced.iter().copied().sum();

        let mean = reduced_sum / reduced_len as f32;
        for sample in reduced {
            *sample -= mean;
        }

        let effective_sample_rate = sample_rate / downsample as u32;
        let min_lag = (effective_sample_rate as f32 / max_pitch_hz)
            .floor()
            .max(1.0) as usize;
        let max_lag = (effective_sample_rate as f32 / min_pitch_hz).ceil() as usize;
        if reduced_len <= max_lag + 1 {
            return None;
        }

        let frame_len = reduced_len;

        let upper_lag = max_lag.min(frame_len - 1);
        self.difference.resize(upper_lag + 1, 0.0);
        self.cmndf.resize(upper_lag + 1, 1.0);
        self.difference[0] = 0.0;
        self.cmndf[0] = 1.0;

        for lag in 1..=upper_lag {
            let mut value = 0.0_f32;
            for index in 0..(frame_len - lag) {
                let delta = self.centered[index] - self.centered[index + lag];
                value += delta * delta;
            }
            self.difference[lag] = value;
        }

        let mut running_sum = 0.0_f32;
        for lag in 1..=upper_lag {
            running_sum += self.difference[lag];
            self.cmndf[lag] = if running_sum > 0.0 {
                self.difference[lag] * lag as f32 / running_sum
            } else {
                1.0
            };
        }

        let yin_threshold = (1.0 - clarity_threshold).clamp(0.05, 0.40);
        let mut best_lag = None;
        for lag in min_lag.max(2)..upper_lag.saturating_sub(1) {
            if self.cmndf[lag] <= yin_threshold
                && self.cmndf[lag] <= self.cmndf[lag - 1]
                && self.cmndf[lag] <= self.cmndf[lag + 1]
            {
                best_lag = Some(lag);
                break;
            }
        }

        let best_lag = best_lag.or_else(|| {
            (min_lag..=upper_lag).min_by(|a, b| self.cmndf[*a].total_cmp(&self.cmndf[*b]))
        })?;

        let clarity = 1.0 - self.cmndf[best_lag];
        if clarity < clarity_threshold {
            return None;
        }

        let refined_lag = parabolic_refine(best_lag, &self.cmndf)
            .clamp(min_lag as f32, upper_lag as f32)
            .max(1.0);
        let boundary_margin = 1.0_f32;
        let near_boundary = refined_lag <= min_lag as f32 + boundary_margin
            || refined_lag >= upper_lag as f32 - boundary_margin;
        if near_boundary && clarity < (clarity_threshold + 0.15).min(0.98) {
            return None;
        }

        let hz = effective_sample_rate as f32 / refined_lag;
        if hz < min_pitch_hz || hz > max_pitch_hz {
            return None;
        }

        let lag_index = refined_lag.round() as usize;
        let periodicity = normalized_autocorrelation(&self.centered, lag_index)
            .min(clarity)
            .max(0.0);

        Some(PitchEstimate {
            hz,
            clarity,
            periodicity,
        })
    }
}

impl FormantAnalyzer {
    pub(crate) fn new(sample_rate: u32, frame_size: usize) -> Self {
        let formant_ceiling_hz = DEFAULT_FORMANT_CEILING_HZ.min(sample_rate as f32 * 0.5 - 50.0);
        let effective_sample_rate = (2.0 * formant_ceiling_hz).round() as u32;
        let analysis_len = ((frame_size as f32 * effective_sample_rate as f32 / sample_rate as f32)
            .round() as usize)
            .max(64);
        let formant_window_len = ((effective_sample_rate as f32 * PRAAT_WINDOW_LENGTH_SECONDS)
            .round() as usize)
            .max(64)
            .min(analysis_len.max(64));

        Self {
            input_sample_rate: sample_rate,
            effective_sample_rate,
            formant_ceiling_hz,
            formant_window_len,
            previous_formants_hz: [None, None, None, None],
            analysis: vec![0.0; frame_size.max(64)],
            resampled: vec![0.0; analysis_len],
            window: gaussian_window(formant_window_len),
        }
    }

    pub(crate) fn estimate_formants(
        &mut self,
        frame: &[f32],
        _pitch_hz: Option<f32>,
    ) -> [Option<f32>; 4] {
        const MIN_FORMANT_HZ: f32 = 50.0;
        let lpc_order = 2 * PRAAT_MAX_FORMANTS;

        if frame.len() < lpc_order + 2 {
            return [None, None, None, None];
        }

        if self.analysis.len() != frame.len() {
            self.analysis.resize(frame.len(), 0.0);
        }
        self.analysis[..frame.len()].copy_from_slice(frame);
        lowpass_biquad_in_place(
            &mut self.analysis[..frame.len()],
            self.input_sample_rate,
            self.formant_ceiling_hz,
        );

        let resampled_len = ((frame.len() as f32 * self.effective_sample_rate as f32
            / self.input_sample_rate as f32)
            .round() as usize)
            .max(64);
        self.resampled.resize(resampled_len, 0.0);
        resample_bandlimited(
            &self.analysis[..frame.len()],
            self.input_sample_rate,
            self.effective_sample_rate,
            &mut self.resampled,
        );

        if self.resampled.len() <= lpc_order + 1 {
            return [None, None, None, None];
        }

        let window_len = self.formant_window_len.min(self.resampled.len());
        if window_len <= lpc_order + 1 {
            return [None, None, None, None];
        }
        let start = (self.resampled.len() - window_len) / 2;
        let end = start + window_len;
        let reduced = &mut self.resampled[start..end];
        if self.window.len() != window_len {
            self.window = gaussian_window(window_len);
        }
        remove_mean(reduced);
        let pre_emphasis = (-2.0 * PI * PRAAT_PRE_EMPHASIS_FROM_HZ
            / self.effective_sample_rate as f32)
            .exp()
            .clamp(0.0, 0.9999);
        apply_pre_emphasis(reduced, pre_emphasis);
        apply_window(reduced, &self.window);

        let coefficients = burg_lpc_coefficients(reduced, lpc_order)
            .or_else(|| lpc_coefficients(reduced, lpc_order));
        let coefficients = match coefficients {
            Some(coefficients) => coefficients,
            None => return [None, None, None, None],
        };

        let max_formant_hz = self.formant_ceiling_hz - 50.0;
        let candidates = lpc_formant_candidates(
            &coefficients,
            self.effective_sample_rate,
            MIN_FORMANT_HZ,
            max_formant_hz,
        );
        let tracked = track_formants(&candidates, self.previous_formants_hz);
        self.previous_formants_hz = tracked;
        tracked
    }
}

pub(crate) fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|index| 0.5 - 0.5 * (2.0 * PI * index as f32 / size as f32).cos())
        .collect()
}

pub(crate) fn zero_crossing_rate(frame: &[f32]) -> f32 {
    if frame.len() < 2 {
        return 0.0;
    }

    let mut crossings = 0usize;
    let mut prev = frame[0];
    for &sample in &frame[1..] {
        if (prev >= 0.0 && sample < 0.0) || (prev < 0.0 && sample >= 0.0) {
            crossings += 1;
        }
        prev = sample;
    }

    crossings as f32 / (frame.len() - 1) as f32
}

pub(crate) fn estimate_hnr_db(periodicity: f32) -> f32 {
    if periodicity <= 0.0 {
        return 0.0;
    }
    let harmonicity = periodicity.clamp(1.0e-6, 0.999);
    10.0 * (harmonicity / (1.0 - harmonicity)).log10()
}

fn parabolic_refine(index: usize, values: &[f32]) -> f32 {
    if index == 0 || index + 1 >= values.len() {
        return index as f32;
    }

    let left = values[index - 1];
    let center = values[index];
    let right = values[index + 1];
    let denominator = left - 2.0 * center + right;
    if denominator.abs() < 1.0e-12 {
        return index as f32;
    }

    index as f32 + 0.5 * (left - right) / denominator
}

fn normalized_autocorrelation(signal: &[f32], lag: usize) -> f32 {
    if lag < 1 || lag >= signal.len().saturating_sub(1) {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut energy_a = 0.0_f32;
    let mut energy_b = 0.0_f32;
    for index in 0..(signal.len() - lag) {
        let a = signal[index];
        let b = signal[index + lag];
        dot += a * b;
        energy_a += a * a;
        energy_b += b * b;
    }

    if energy_a <= 1.0e-12 || energy_b <= 1.0e-12 {
        0.0
    } else {
        (dot / (energy_a.sqrt() * energy_b.sqrt())).clamp(0.0, 1.0)
    }
}

fn remove_mean(signal: &mut [f32]) {
    if signal.is_empty() {
        return;
    }

    let mean = signal.iter().copied().sum::<f32>() / signal.len() as f32;
    for sample in signal {
        *sample -= mean;
    }
}

fn apply_pre_emphasis(signal: &mut [f32], coefficient: f32) {
    if signal.len() < 2 {
        return;
    }

    let mut previous = signal[0];
    for sample in signal.iter_mut().skip(1) {
        let original = *sample;
        *sample = original - coefficient * previous;
        previous = original;
    }
}

fn lowpass_biquad_in_place(signal: &mut [f32], sample_rate: u32, cutoff_hz: f32) {
    let nyquist_hz = sample_rate as f32 * 0.5;
    if signal.len() < 3 || cutoff_hz <= 0.0 || cutoff_hz >= nyquist_hz - 1.0 {
        return;
    }

    let q = 1.0_f32 / 2.0_f32.sqrt();
    let omega = 2.0 * PI * cutoff_hz / sample_rate as f32;
    let alpha = omega.sin() / (2.0 * q);
    let cos_omega = omega.cos();

    let b0 = (1.0 - cos_omega) * 0.5;
    let b1 = 1.0 - cos_omega;
    let b2 = (1.0 - cos_omega) * 0.5;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_omega;
    let a2 = 1.0 - alpha;

    let b0 = b0 / a0;
    let b1 = b1 / a0;
    let b2 = b2 / a0;
    let a1 = a1 / a0;
    let a2 = a2 / a0;

    let mut x1 = 0.0_f32;
    let mut x2 = 0.0_f32;
    let mut y1 = 0.0_f32;
    let mut y2 = 0.0_f32;
    for sample in signal.iter_mut() {
        let x0 = *sample;
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *sample = y0;
    }
}

fn gaussian_window(len: usize) -> Vec<f32> {
    if len < 2 {
        return vec![1.0; len];
    }

    let midpoint = 0.5 * (len - 1) as f32;
    let sigma = 0.23 * (len - 1) as f32;
    (0..len)
        .map(|index| {
            let offset = (index as f32 - midpoint) / sigma.max(1.0);
            (-0.5 * offset * offset).exp()
        })
        .collect()
}

fn apply_window(signal: &mut [f32], window: &[f32]) {
    debug_assert_eq!(signal.len(), window.len());

    for (sample, factor) in signal.iter_mut().zip(window.iter().copied()) {
        *sample *= factor;
    }
}

fn fill_downsampled(frame: &[f32], downsample: usize, output: &mut [f32]) {
    match downsample {
        1 => output.copy_from_slice(&frame[..output.len()]),
        2 => {
            for (chunk, slot) in frame.chunks_exact(2).zip(output.iter_mut()) {
                *slot = (chunk[0] + chunk[1]) * 0.5;
            }
        }
        3 => {
            for (chunk, slot) in frame.chunks_exact(3).zip(output.iter_mut()) {
                *slot = (chunk[0] + chunk[1] + chunk[2]) * (1.0 / 3.0);
            }
        }
        4 => {
            for (chunk, slot) in frame.chunks_exact(4).zip(output.iter_mut()) {
                *slot = (chunk[0] + chunk[1] + chunk[2] + chunk[3]) * 0.25;
            }
        }
        _ => {
            let scale = 1.0 / downsample as f32;
            for (chunk, slot) in frame.chunks_exact(downsample).zip(output.iter_mut()) {
                let sum: f32 = chunk.iter().copied().sum();
                *slot = sum * scale;
            }
        }
    }
}

fn resample_bandlimited(
    input: &[f32],
    input_sample_rate: u32,
    output_sample_rate: u32,
    output: &mut [f32],
) {
    const HALF_TAPS: isize = 16;

    if input.is_empty() || output.is_empty() {
        return;
    }
    if input_sample_rate == output_sample_rate {
        let copy_len = input.len().min(output.len());
        output[..copy_len].copy_from_slice(&input[..copy_len]);
        if output.len() > copy_len {
            for sample in &mut output[copy_len..] {
                *sample = *input.last().unwrap_or(&0.0);
            }
        }
        return;
    }

    let scale = input_sample_rate as f32 / output_sample_rate as f32;
    let cutoff = if output_sample_rate < input_sample_rate {
        output_sample_rate as f32 / input_sample_rate as f32
    } else {
        1.0
    };
    for (index, slot) in output.iter_mut().enumerate() {
        let position = index as f32 * scale;
        let center = position.floor() as isize;
        let mut sum = 0.0_f32;
        let mut weight_sum = 0.0_f32;

        for tap in -HALF_TAPS..=HALF_TAPS {
            let sample_index = center + tap;
            if sample_index < 0 || sample_index >= input.len() as isize {
                continue;
            }

            let distance = position - sample_index as f32;
            let sinc_arg = PI * distance * cutoff;
            let sinc = if sinc_arg.abs() < 1.0e-6 {
                1.0
            } else {
                sinc_arg.sin() / sinc_arg
            };
            let window_phase = (tap + HALF_TAPS) as f32 / (2 * HALF_TAPS) as f32;
            let window = 0.42 - 0.5 * (2.0 * PI * window_phase).cos()
                + 0.08 * (4.0 * PI * window_phase).cos();
            let weight = sinc * window * cutoff;
            sum += input[sample_index as usize] * weight;
            weight_sum += weight;
        }

        *slot = if weight_sum.abs() > 1.0e-9 {
            sum / weight_sum
        } else {
            0.0
        };
    }
}

fn burg_lpc_coefficients(signal: &[f32], order: usize) -> Option<Vec<f32>> {
    if signal.len() <= order + 1 {
        return None;
    }

    let mut forward = signal.to_vec();
    let mut backward = signal.to_vec();
    let mut coefficients = vec![0.0_f64; order + 1];
    coefficients[0] = 1.0;

    let mut error = signal
        .iter()
        .map(|&sample| {
            let sample = sample as f64;
            sample * sample
        })
        .sum::<f64>()
        / signal.len() as f64;
    if !error.is_finite() || error <= 1.0e-12 {
        return None;
    }

    for m in 1..=order {
        let span = signal.len() - m;
        let mut numerator = 0.0_f64;
        let mut denominator = 0.0_f64;
        for n in 0..span {
            let f = forward[n + 1] as f64;
            let b = backward[n] as f64;
            numerator += f * b;
            denominator += f * f + b * b;
        }

        if !denominator.is_finite() || denominator <= 1.0e-12 {
            return None;
        }

        let reflection = -2.0 * numerator / denominator;
        if !reflection.is_finite() || reflection.abs() >= 1.0 {
            return None;
        }

        let previous = coefficients.clone();
        for i in 1..m {
            coefficients[i] = previous[i] + reflection * previous[m - i];
        }
        coefficients[m] = reflection;

        for n in (0..span).rev() {
            let f = forward[n + 1] as f64;
            let b = backward[n] as f64;
            forward[n + 1] = (f + reflection * b) as f32;
            backward[n] = (b + reflection * f) as f32;
        }

        error *= 1.0 - reflection * reflection;
        if !error.is_finite() || error <= 1.0e-12 {
            return None;
        }
    }

    let mut lpc_f32 = Vec::with_capacity(order + 1);
    lpc_f32.extend(
        coefficients
            .into_iter()
            .map(|coefficient| coefficient as f32),
    );
    if lpc_f32.iter().all(|coefficient| coefficient.is_finite()) {
        Some(lpc_f32)
    } else {
        None
    }
}

fn lpc_coefficients(signal: &[f32], order: usize) -> Option<Vec<f32>> {
    let mut autocorrelation = vec![0.0_f64; order + 1];
    for lag in 0..=order {
        let mut sum = 0.0_f64;
        for index in 0..(signal.len() - lag) {
            sum += signal[index] as f64 * signal[index + lag] as f64;
        }
        autocorrelation[lag] = sum;
    }

    if !autocorrelation[0].is_finite() || autocorrelation[0] <= 1.0e-9 {
        return None;
    }

    let mut lpc = vec![0.0_f64; order + 1];
    lpc[0] = 1.0;

    let mut error = autocorrelation[0];
    for i in 1..=order {
        let mut reflection = autocorrelation[i];
        for j in 1..i {
            reflection += lpc[j] * autocorrelation[i - j];
        }

        reflection = -reflection / error;
        if !reflection.is_finite() {
            return None;
        }

        for j in 1..=((i - 1) / 2) {
            let left = lpc[j];
            let right = lpc[i - j];
            lpc[j] = left + reflection * right;
            lpc[i - j] = right + reflection * left;
        }

        if i % 2 == 0 {
            let mid = i / 2;
            lpc[mid] += reflection * lpc[mid];
        }

        lpc[i] = reflection;
        error *= 1.0 - reflection * reflection;
        if !error.is_finite() || error <= 1.0e-12 {
            return None;
        }
    }

    let mut lpc_f32 = Vec::with_capacity(order + 1);
    lpc_f32.extend(lpc.into_iter().map(|coefficient| coefficient as f32));
    if lpc_f32.iter().all(|coefficient| coefficient.is_finite()) {
        Some(lpc_f32)
    } else {
        None
    }
}

fn lpc_envelope_db(coefficients: &[f32], sample_rate: u32, frequency_hz: f32) -> f32 {
    let omega = 2.0 * PI * frequency_hz / sample_rate as f32;
    let mut real = 0.0_f32;
    let mut imag = 0.0_f32;
    for (order, coefficient) in coefficients.iter().copied().enumerate() {
        let phase = -omega * order as f32;
        real += coefficient * phase.cos();
        imag += coefficient * phase.sin();
    }
    let magnitude = (real.mul_add(real, imag * imag)).sqrt().max(1.0e-9);
    -20.0 * magnitude.log10()
}

fn lpc_formant_candidates(
    coefficients: &[f32],
    sample_rate: u32,
    min_formant_hz: f32,
    max_formant_hz: f32,
) -> Vec<(f32, f32)> {
    const MIN_BANDWIDTH_HZ: f32 = 20.0;
    const MAX_BANDWIDTH_HZ: f32 = 800.0;
    const MIN_IMAGINARY: f64 = 0.01;

    let polynomial = coefficients
        .iter()
        .rev()
        .copied()
        .map(f64::from)
        .collect::<Vec<_>>();
    let mut roots = polynomial_roots(&polynomial);
    roots.retain(|root| root.im >= MIN_IMAGINARY);

    let mut candidates = roots
        .into_iter()
        .filter_map(|root| {
            let radius = root.norm() as f32;
            if !radius.is_finite() || radius <= 1.0e-6 || radius >= 1.0 {
                return None;
            }

            let frequency_hz = root.arg() as f32 * sample_rate as f32 / (2.0 * PI);
            let bandwidth_hz = -(sample_rate as f32 / PI) * radius.ln();
            let valid_frequency = frequency_hz >= min_formant_hz && frequency_hz <= max_formant_hz;
            let valid_bandwidth =
                bandwidth_hz >= MIN_BANDWIDTH_HZ && bandwidth_hz <= MAX_BANDWIDTH_HZ;
            if valid_frequency && valid_bandwidth {
                Some((
                    frequency_hz,
                    lpc_envelope_db(coefficients, sample_rate, frequency_hz),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
    candidates
}

fn polynomial_roots(coefficients: &[f64]) -> Vec<Complex64> {
    let degree = coefficients.len().saturating_sub(1);
    if degree == 0 {
        return Vec::new();
    }

    let mut roots = (0..degree)
        .map(|index| {
            let angle = 2.0 * std::f64::consts::PI * index as f64 / degree as f64;
            Complex64::new(0.5 * angle.cos(), 0.5 * angle.sin())
        })
        .collect::<Vec<_>>();

    for _ in 0..80 {
        let mut converged = true;
        for index in 0..degree {
            let root = roots[index];
            let numerator = evaluate_polynomial(coefficients, root);
            let mut denominator = Complex64::new(1.0, 0.0);
            for (other_index, other_root) in roots.iter().copied().enumerate() {
                if index != other_index {
                    denominator *= root - other_root;
                }
            }
            if denominator.norm() <= 1.0e-18 {
                continue;
            }

            let next = root - numerator / denominator;
            if (next - root).norm() > 1.0e-10 {
                converged = false;
            }
            roots[index] = next;
        }

        if converged {
            break;
        }
    }

    roots
}

fn evaluate_polynomial(coefficients: &[f64], x: Complex64) -> Complex64 {
    let mut acc = Complex64::new(0.0, 0.0);
    for &coefficient in coefficients.iter().rev() {
        acc = acc * x + Complex64::new(coefficient, 0.0);
    }
    acc
}

fn track_formants(candidates: &[(f32, f32)], previous: [Option<f32>; 4]) -> [Option<f32>; 4] {
    let mut assigned = [None, None, None, None];
    let mut last_frequency_hz = 0.0_f32;

    for slot_index in 0..4 {
        let (min_hz, max_hz) = FORMANT_SLOT_RANGES_HZ[slot_index];
        let previous_hz = previous[slot_index];
        let mut best = None;
        let mut best_cost = f32::INFINITY;

        for &(frequency_hz, strength_db) in candidates {
            if frequency_hz < min_hz
                || frequency_hz > max_hz
                || frequency_hz <= last_frequency_hz + FORMANT_TRACK_ORDER_MARGIN_HZ
            {
                continue;
            }

            let cost =
                formant_candidate_cost(slot_index, frequency_hz, strength_db, previous_hz, 0.6);
            if cost < best_cost {
                best_cost = cost;
                best = Some(frequency_hz);
            }
        }

        if best.is_none() {
            let relaxed_min_hz = min_hz * 0.85;
            let relaxed_max_hz = max_hz * 1.10;
            for &(frequency_hz, strength_db) in candidates {
                if frequency_hz < relaxed_min_hz
                    || frequency_hz > relaxed_max_hz
                    || frequency_hz <= last_frequency_hz + FORMANT_TRACK_ORDER_MARGIN_HZ
                {
                    continue;
                }

                let cost =
                    formant_candidate_cost(slot_index, frequency_hz, strength_db, previous_hz, 0.8);
                if cost < best_cost {
                    best_cost = cost;
                    best = Some(frequency_hz);
                }
            }
        }

        let chosen = if let Some(frequency_hz) = best {
            Some(frequency_hz)
        } else if let Some(previous_hz) = previous_hz {
            if previous_hz >= min_hz * 0.85
                && previous_hz <= max_hz * 1.10
                && previous_hz > last_frequency_hz + FORMANT_TRACK_ORDER_MARGIN_HZ
            {
                Some(previous_hz)
            } else {
                None
            }
        } else {
            None
        };

        assigned[slot_index] = chosen;
        if let Some(frequency_hz) = chosen {
            last_frequency_hz = frequency_hz;
        }
    }

    assigned
}

fn formant_candidate_cost(
    slot_index: usize,
    frequency_hz: f32,
    strength_db: f32,
    previous_hz: Option<f32>,
    target_weight: f32,
) -> f32 {
    let continuity_cost = previous_hz
        .map(|previous_hz| (frequency_hz / previous_hz).ln().abs() * 4.0)
        .unwrap_or(
            ((frequency_hz - FORMANT_SLOT_TARGETS_HZ[slot_index]).abs()
                / FORMANT_SLOT_TARGETS_HZ[slot_index])
                * target_weight,
        );
    let strength_cost = (-strength_db) * 0.05;
    continuity_cost + strength_cost
}
