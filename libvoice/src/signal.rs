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
