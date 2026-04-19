use core::f32::consts::PI;

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
        let frame_len = frame.len();
        if frame_len < 3 {
            return None;
        }

        self.centered.resize(frame_len, 0.0);
        let centered = &mut self.centered[..frame_len];
        let reduced_sum: f32 = frame.iter().copied().sum();

        let mean = reduced_sum / frame_len as f32;
        for (dst, &sample) in centered.iter_mut().zip(frame.iter()) {
            *dst = sample - mean;
        }

        let min_lag = (sample_rate as f32 / max_pitch_hz).floor().max(1.0) as usize;
        let max_lag = (sample_rate as f32 / min_pitch_hz).ceil() as usize;
        if frame_len <= max_lag + 1 {
            return None;
        }

        let upper_lag = max_lag.min(frame_len - 1);
        self.difference.resize(upper_lag + 1, 0.0);
        self.cmndf.resize(upper_lag + 1, 1.0);
        self.difference[0] = 0.0;
        self.cmndf[0] = 1.0;

        let centered = &self.centered[..frame_len];
        for lag in 1..=upper_lag {
            self.difference[lag] =
                squared_difference_sum(&centered[..frame_len - lag], &centered[lag..]);
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

        let hz = sample_rate as f32 / refined_lag;
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

pub(crate) fn estimate_hnr_db(periodicity: f32) -> f32 {
    if periodicity <= 0.0 {
        return 0.0;
    }
    let harmonicity = periodicity.clamp(1.0e-6, 0.999);
    10.0 * (harmonicity / (1.0 - harmonicity)).log10()
}

pub(crate) fn estimate_loudness_dbfs(rms: f32) -> f32 {
    20.0 * rms.max(1.0e-12).log10()
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

    let overlap = &signal[..signal.len() - lag];
    let shifted = &signal[lag..];
    let (dot, energy_a, energy_b) = correlation_sums(overlap, shifted);

    if energy_a <= 1.0e-12 || energy_b <= 1.0e-12 {
        0.0
    } else {
        (dot / (energy_a.sqrt() * energy_b.sqrt())).clamp(0.0, 1.0)
    }
}

fn squared_difference_sum(left: &[f32], right: &[f32]) -> f32 {
    debug_assert_eq!(left.len(), right.len());

    let mut sum0 = 0.0_f32;
    let mut sum1 = 0.0_f32;
    let mut sum2 = 0.0_f32;
    let mut sum3 = 0.0_f32;

    let mut left_chunks = left.chunks_exact(4);
    let mut right_chunks = right.chunks_exact(4);

    for (left_chunk, right_chunk) in left_chunks.by_ref().zip(right_chunks.by_ref()) {
        let delta0 = left_chunk[0] - right_chunk[0];
        let delta1 = left_chunk[1] - right_chunk[1];
        let delta2 = left_chunk[2] - right_chunk[2];
        let delta3 = left_chunk[3] - right_chunk[3];
        sum0 += delta0 * delta0;
        sum1 += delta1 * delta1;
        sum2 += delta2 * delta2;
        sum3 += delta3 * delta3;
    }

    let mut sum = (sum0 + sum1) + (sum2 + sum3);
    for (&a, &b) in left_chunks
        .remainder()
        .iter()
        .zip(right_chunks.remainder().iter())
    {
        let delta = a - b;
        sum += delta * delta;
    }

    sum
}

fn correlation_sums(left: &[f32], right: &[f32]) -> (f32, f32, f32) {
    debug_assert_eq!(left.len(), right.len());

    let mut dot0 = 0.0_f32;
    let mut dot1 = 0.0_f32;
    let mut dot2 = 0.0_f32;
    let mut dot3 = 0.0_f32;
    let mut energy_a0 = 0.0_f32;
    let mut energy_a1 = 0.0_f32;
    let mut energy_a2 = 0.0_f32;
    let mut energy_a3 = 0.0_f32;
    let mut energy_b0 = 0.0_f32;
    let mut energy_b1 = 0.0_f32;
    let mut energy_b2 = 0.0_f32;
    let mut energy_b3 = 0.0_f32;

    let mut left_chunks = left.chunks_exact(4);
    let mut right_chunks = right.chunks_exact(4);

    for (left_chunk, right_chunk) in left_chunks.by_ref().zip(right_chunks.by_ref()) {
        let a0 = left_chunk[0];
        let a1 = left_chunk[1];
        let a2 = left_chunk[2];
        let a3 = left_chunk[3];
        let b0 = right_chunk[0];
        let b1 = right_chunk[1];
        let b2 = right_chunk[2];
        let b3 = right_chunk[3];

        dot0 += a0 * b0;
        dot1 += a1 * b1;
        dot2 += a2 * b2;
        dot3 += a3 * b3;

        energy_a0 += a0 * a0;
        energy_a1 += a1 * a1;
        energy_a2 += a2 * a2;
        energy_a3 += a3 * a3;

        energy_b0 += b0 * b0;
        energy_b1 += b1 * b1;
        energy_b2 += b2 * b2;
        energy_b3 += b3 * b3;
    }

    let mut dot = (dot0 + dot1) + (dot2 + dot3);
    let mut energy_a = (energy_a0 + energy_a1) + (energy_a2 + energy_a3);
    let mut energy_b = (energy_b0 + energy_b1) + (energy_b2 + energy_b3);

    for (&a, &b) in left_chunks
        .remainder()
        .iter()
        .zip(right_chunks.remainder().iter())
    {
        dot += a * b;
        energy_a += a * a;
        energy_b += b * b;
    }

    (dot, energy_a, energy_b)
}
