use core::f32::consts::PI;
use rustfft::num_complex::Complex64;

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
        let mut reduced_sum = 0.0_f32;
        for (index, chunk) in frame.chunks_exact(downsample).take(reduced_len).enumerate() {
            let sample = chunk.iter().copied().sum::<f32>() / downsample as f32;
            self.centered[index] = sample;
            reduced_sum += sample;
        }

        let mean = reduced_sum / reduced_len as f32;
        for sample in &mut self.centered[..reduced_len] {
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

pub(crate) fn estimate_formants(
    frame: &[f32],
    sample_rate: u32,
    pitch_hz: Option<f32>,
) -> [Option<f32>; 4] {
    const TARGET_SAMPLE_RATE: u32 = 16_000;
    const LPC_ORDER: usize = 12;
    const PRE_EMPHASIS: f32 = 0.94;
    const MIN_FORMANT_HZ: f32 = 200.0;
    const MAX_FORMANT_HZ: f32 = 5_000.0;
    const MIN_BANDWIDTH_HZ: f32 = 20.0;
    const MAX_BANDWIDTH_HZ: f32 = 800.0;

    if frame.len() < LPC_ORDER + 2 || sample_rate == 0 {
        return [None, None, None, None];
    }

    let downsample = (sample_rate / TARGET_SAMPLE_RATE).max(1) as usize;
    let reduced_len = frame.len() / downsample;
    if reduced_len <= LPC_ORDER + 1 {
        return [None, None, None, None];
    }

    let effective_sample_rate = sample_rate / downsample as u32;
    let mut reduced = Vec::with_capacity(reduced_len);
    for chunk in frame.chunks_exact(downsample).take(reduced_len) {
        reduced.push(chunk.iter().copied().sum::<f32>() / downsample as f32);
    }

    apply_pre_emphasis(&mut reduced, PRE_EMPHASIS);
    apply_hamming_window(&mut reduced);

    let coefficients = match lpc_coefficients(&reduced, LPC_ORDER) {
        Some(coefficients) => coefficients,
        None => return [None, None, None, None],
    };

    let polynomial = coefficients
        .iter()
        .rev()
        .map(|&coefficient| coefficient as f64)
        .collect::<Vec<_>>();
    let mut roots = polynomial_roots(&polynomial);
    roots.retain(|root| root.im >= 0.01);

    let max_formant_hz = MAX_FORMANT_HZ.min(effective_sample_rate as f32 * 0.5);
    let pitch_guard_hz = pitch_hz
        .map(|pitch| (pitch * 2.0).max(MIN_FORMANT_HZ))
        .unwrap_or(MIN_FORMANT_HZ);
    let mut formants = roots
        .into_iter()
        .filter_map(|root| {
            let radius = root.norm() as f32;
            if !radius.is_finite() || radius <= 1.0e-6 {
                return None;
            }

            let frequency_hz = root.arg() as f32 * effective_sample_rate as f32 / (2.0 * PI);
            let bandwidth_hz = -(effective_sample_rate as f32 / PI) * radius.ln();

            let valid_frequency = frequency_hz > pitch_guard_hz && frequency_hz < max_formant_hz;
            let valid_bandwidth =
                bandwidth_hz > MIN_BANDWIDTH_HZ && bandwidth_hz < MAX_BANDWIDTH_HZ;
            if valid_frequency && valid_bandwidth {
                Some(frequency_hz)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    formants.sort_by(|left, right| left.total_cmp(right));

    let mut detected = [None, None, None, None];
    for (slot, hz) in detected.iter_mut().zip(formants.into_iter()) {
        *slot = Some(hz);
    }
    detected
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

fn apply_hamming_window(signal: &mut [f32]) {
    let len = signal.len();
    if len < 2 {
        return;
    }

    for (index, sample) in signal.iter_mut().enumerate() {
        let phase = 2.0 * PI * index as f32 / (len - 1) as f32;
        let window = 0.54 - 0.46 * phase.cos();
        *sample *= window;
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

    let mut matrix = vec![vec![0.0_f64; order]; order];
    let mut rhs = vec![0.0_f64; order];
    for row in 0..order {
        rhs[row] = -autocorrelation[row + 1];
        for col in 0..order {
            let lag = row.abs_diff(col);
            matrix[row][col] = autocorrelation[lag];
        }
    }

    let solution = solve_linear_system(matrix, rhs)?;
    let mut lpc = Vec::with_capacity(order + 1);
    lpc.push(1.0);
    lpc.extend(solution.into_iter().map(|coefficient| coefficient as f32));
    Some(lpc)
}

fn solve_linear_system(mut matrix: Vec<Vec<f64>>, mut rhs: Vec<f64>) -> Option<Vec<f64>> {
    let n = rhs.len();
    for pivot in 0..n {
        let mut pivot_row = pivot;
        let mut pivot_value = matrix[pivot][pivot].abs();
        for row in (pivot + 1)..n {
            let candidate = matrix[row][pivot].abs();
            if candidate > pivot_value {
                pivot_row = row;
                pivot_value = candidate;
            }
        }

        if !pivot_value.is_finite() || pivot_value <= 1.0e-12 {
            return None;
        }

        if pivot_row != pivot {
            matrix.swap(pivot, pivot_row);
            rhs.swap(pivot, pivot_row);
        }

        let pivot_scale = matrix[pivot][pivot];
        for col in pivot..n {
            matrix[pivot][col] /= pivot_scale;
        }
        rhs[pivot] /= pivot_scale;

        for row in 0..n {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot];
            if factor.abs() <= 1.0e-12 {
                continue;
            }
            for col in pivot..n {
                matrix[row][col] -= factor * matrix[pivot][col];
            }
            rhs[row] -= factor * rhs[pivot];
        }
    }

    if rhs.iter().all(|value| value.is_finite()) {
        Some(rhs)
    } else {
        None
    }
}

fn polynomial_roots(coefficients: &[f64]) -> Vec<Complex64> {
    let degree = coefficients.len().saturating_sub(1);
    if degree == 0 {
        return Vec::new();
    }

    let mut roots = (0..degree)
        .map(|index| {
            let angle = 2.0 * std::f64::consts::PI * index as f64 / degree as f64;
            Complex64::new(angle.cos(), angle.sin())
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
