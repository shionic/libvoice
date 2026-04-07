use crate::model::{ChunkAnalysis, FrameFeatures, JitterMetrics, OverallAnalysis, SpectralSummary};
use crate::stats::{summarize_optional, summarize_required};

pub(crate) fn summarize_chunk(
    chunk_index: usize,
    input_samples: usize,
    frames: &[FrameFeatures],
    frame_step_seconds: f32,
) -> ChunkAnalysis {
    ChunkAnalysis {
        chunk_index,
        input_samples,
        frame_count: frames.len(),
        pitch_hz: summarize_optional(summarized_pitch_values(frames).into_iter()),
        spectral: summarize_spectral(frames),
        energy: summarize_required(frames.iter().map(|f| f.energy)),
        jitter: summarize_jitter(frames, frame_step_seconds),
    }
}

pub(crate) fn summarize_overall(
    processed_samples: usize,
    frames: &[FrameFeatures],
    frame_step_seconds: f32,
) -> OverallAnalysis {
    OverallAnalysis {
        processed_samples,
        frame_count: frames.len(),
        pitch_hz: summarize_optional(summarized_pitch_values(frames).into_iter()),
        spectral: summarize_spectral(frames),
        energy: summarize_required(frames.iter().map(|f| f.energy)),
        jitter: summarize_jitter(frames, frame_step_seconds),
    }
}

fn summarize_spectral(frames: &[FrameFeatures]) -> Option<SpectralSummary> {
    if frames.is_empty() {
        return None;
    }

    Some(SpectralSummary {
        rolloff_hz: summarize_required(frames.iter().map(|f| f.spectral_rolloff_hz)).unwrap(),
        centroid_hz: summarize_required(frames.iter().map(|f| f.spectral_centroid_hz)).unwrap(),
        bandwidth_hz: summarize_required(frames.iter().map(|f| f.spectral_bandwidth_hz)).unwrap(),
        flatness: summarize_required(frames.iter().map(|f| f.spectral_flatness)).unwrap(),
        zcr: summarize_required(frames.iter().map(|f| f.zcr)).unwrap(),
        rms: summarize_required(frames.iter().map(|f| f.rms)).unwrap(),
        hnr_db: summarize_required(frames.iter().map(|f| f.hnr_db)).unwrap(),
    })
}

fn summarize_jitter(frames: &[FrameFeatures], frame_step_seconds: f32) -> Option<JitterMetrics> {
    let segments = stable_pitch_segments(frames);
    let valid_segments: Vec<&[f32]> = segments
        .iter()
        .map(Vec::as_slice)
        .filter(|segment| segment.len() >= 3)
        .collect();
    if valid_segments.is_empty() {
        return None;
    }

    let mut hz_diffs = Vec::new();
    let mut period_ratio_diffs = Vec::new();
    let mut signed_period_diffs = Vec::new();
    let period_segments = stable_period_segments(frames);
    let valid_period_segments: Vec<&[f32]> = period_segments
        .iter()
        .map(Vec::as_slice)
        .filter(|segment| segment.len() >= 3)
        .collect();

    for segment in &valid_segments {
        for pair in segment.windows(2) {
            hz_diffs.push((pair[1] - pair[0]).abs());
        }
    }
    for segment in &valid_period_segments {
        for pair in segment.windows(2) {
            let delta = pair[1] - pair[0];
            period_ratio_diffs.push(delta.abs() / pair[1].max(pair[0]).max(1.0e-6));
            signed_period_diffs.push(delta);
        }
    }

    if hz_diffs.is_empty() || period_ratio_diffs.is_empty() {
        return None;
    }

    let diff_mean = mean(&hz_diffs);
    let diff_std = stddev(&hz_diffs, diff_mean);
    let ratio_mean = mean(&period_ratio_diffs);
    let ratio_std = stddev(&period_ratio_diffs, ratio_mean);
    let all_periods: Vec<f32> = valid_period_segments
        .iter()
        .flat_map(|segment| segment.iter().copied())
        .collect();
    let mean_period = mean(&all_periods);
    let local_absolute_seconds = mean_abs_successive_period_delta_segments(&valid_period_segments);
    let local_ratio = if mean_period > 0.0 {
        local_absolute_seconds / mean_period
    } else {
        0.0
    };
    let rap_ratio = averaged_period_perturbation(&valid_period_segments, 3, mean_period);
    let ppq5_ratio = averaged_period_perturbation(&valid_period_segments, 5, mean_period);
    let ddp_ratio = rap_ratio * 3.0;

    let direction_changes = signed_period_diffs
        .windows(2)
        .filter(|pair| pair[0].signum() != pair[1].signum() && pair[0] != 0.0 && pair[1] != 0.0)
        .count();
    let direction_change_rate =
        direction_changes as f32 / signed_period_diffs.len().saturating_sub(1).max(1) as f32;

    let robust_threshold = median(&hz_diffs) + 3.0 * median_absolute_deviation(&hz_diffs);
    let rapid_change_ratio = hz_diffs
        .iter()
        .filter(|delta| **delta > robust_threshold.max(diff_mean + diff_std))
        .count() as f32
        / hz_diffs.len() as f32;

    let reference_segment = valid_segments
        .iter()
        .max_by_key(|segment| segment.len())
        .copied()
        .unwrap();
    let (estimated_vibrato_hz, estimated_vibrato_extent_cents) =
        estimate_pitch_modulation(reference_segment, frame_step_seconds);

    Some(JitterMetrics {
        sample_count: period_ratio_diffs.len(),
        local_ratio,
        local_absolute_seconds,
        rap_ratio,
        ppq5_ratio,
        ddp_ratio,
        local_hz_mean: diff_mean,
        local_hz_std: diff_std,
        local_ratio_mean: ratio_mean,
        local_ratio_std: ratio_std,
        direction_change_rate,
        rapid_change_ratio,
        estimated_vibrato_hz,
        estimated_vibrato_extent_cents,
    })
}

fn summarized_pitch_values(frames: &[FrameFeatures]) -> Vec<f32> {
    median_smooth_pitch_contour(&repair_pitch_outliers(raw_pitch_contour(frames)), 2)
}

fn stable_pitch_segments(frames: &[FrameFeatures]) -> Vec<Vec<f32>> {
    let contour = repair_pitch_outliers(raw_pitch_contour(frames));
    if contour.len() < 3 {
        return if contour.is_empty() {
            Vec::new()
        } else {
            vec![contour]
        };
    }

    let mut segments = Vec::new();
    let mut run_start = 0usize;

    for index in 1..contour.len() {
        let prev = contour[index - 1];
        let current = contour[index];
        let jump_ratio = (current - prev).abs() / prev.max(current).max(1.0);
        if jump_ratio > 0.12 || is_octave_like_jump(prev, current) {
            let run = contour[run_start..index].to_vec();
            if run.len() >= 3 {
                segments.push(run);
            }
            run_start = index;
        }
    }

    let tail = contour[run_start..].to_vec();
    if tail.len() >= 3 {
        segments.push(tail);
    }

    segments
}

fn raw_pitch_contour(frames: &[FrameFeatures]) -> Vec<f32> {
    frames.iter().filter_map(|f| f.pitch_hz).collect()
}

fn stable_period_segments(frames: &[FrameFeatures]) -> Vec<Vec<f32>> {
    let pitch_segments = stable_pitch_segments(frames);
    let mut period_segments = Vec::new();
    let mut frame_index = 0usize;

    for segment in pitch_segments {
        let mut periods = Vec::with_capacity(segment.len());
        while frame_index < frames.len() && periods.len() < segment.len() {
            if let Some(period) = frames[frame_index].period_seconds {
                periods.push(period);
            }
            frame_index += 1;
        }
        if periods.len() >= 3 {
            period_segments.push(periods);
        }
    }

    period_segments
}

fn median_smooth_pitch_contour(raw: &[f32], radius: usize) -> Vec<f32> {
    if raw.len() < 3 {
        return raw.to_vec();
    }

    let mut smoothed = Vec::with_capacity(raw.len());
    for index in 0..raw.len() {
        let start = index.saturating_sub(radius);
        let end = (index + radius + 1).min(raw.len());
        let mut window = raw[start..end].to_vec();
        window.sort_by(|a, b| a.total_cmp(b));
        let median = window[window.len() / 2];
        smoothed.push(median);
    }

    smoothed
}

fn repair_pitch_outliers(mut contour: Vec<f32>) -> Vec<f32> {
    if contour.len() < 3 {
        return contour;
    }

    for index in 1..contour.len().saturating_sub(1) {
        let prev = contour[index - 1];
        let current = contour[index];
        let next = contour[index + 1];
        let prev_jump = (current - prev).abs() / prev.max(current).max(1.0);
        let next_jump = (current - next).abs() / current.max(next).max(1.0);
        let bridge_jump = (next - prev).abs() / next.max(prev).max(1.0);

        if prev_jump > 0.18 && next_jump > 0.18 && bridge_jump < 0.08 {
            contour[index] = 0.5 * (prev + next);
        }
    }

    contour
}

fn estimate_pitch_modulation(contour: &[f32], frame_step_seconds: f32) -> (f32, f32) {
    if contour.len() < 16 || frame_step_seconds <= 0.0 {
        return (0.0, 0.0);
    }

    let log_pitch: Vec<f32> = contour.iter().map(|pitch| pitch.log2()).collect();
    let smoothed = moving_average(&log_pitch, 4);
    let detrended: Vec<f32> = log_pitch
        .iter()
        .zip(smoothed.iter())
        .map(|(value, trend)| value - trend)
        .collect();
    let variance = detrended.iter().map(|value| value * value).sum::<f32>() / detrended.len() as f32;
    if variance <= 1.0e-8 {
        return (0.0, 0.0);
    }

    let min_hz = 3.0_f32;
    let max_hz = 10.0_f32;
    let min_lag = ((1.0 / (max_hz * frame_step_seconds)).round() as usize).max(1);
    let max_lag = ((1.0 / (min_hz * frame_step_seconds)).round() as usize)
        .min(detrended.len().saturating_sub(2));
    if min_lag >= max_lag {
        return (0.0, 0.0);
    }

    let mut best_lag = 0usize;
    let mut best_corr = -1.0_f32;
    for lag in min_lag..=max_lag {
        let mut dot = 0.0_f32;
        let mut norm_a = 0.0_f32;
        let mut norm_b = 0.0_f32;
        for index in 0..(detrended.len() - lag) {
            let a = detrended[index];
            let b = detrended[index + lag];
            dot += a * b;
            norm_a += a * a;
            norm_b += b * b;
        }
        let corr = if norm_a > 0.0 && norm_b > 0.0 {
            dot / (norm_a.sqrt() * norm_b.sqrt())
        } else {
            0.0
        };
        if corr > best_corr {
            best_corr = corr;
            best_lag = lag;
        }
    }

    if best_lag == 0 || best_corr < 0.25 {
        return (0.0, 0.0);
    }

    let vibrato_hz = 1.0 / (best_lag as f32 * frame_step_seconds);
    let rms_cents = (detrended.iter().map(|value| value * value).sum::<f32>() / detrended.len() as f32)
        .sqrt()
        * 1200.0;
    let extent_cents = (rms_cents * 2.0_f32.sqrt()).max(0.0);
    (vibrato_hz, extent_cents)
}

fn moving_average(values: &[f32], radius: usize) -> Vec<f32> {
    let mut averaged = Vec::with_capacity(values.len());
    for index in 0..values.len() {
        let start = index.saturating_sub(radius);
        let end = (index + radius + 1).min(values.len());
        averaged.push(mean(&values[start..end]));
    }
    averaged
}

fn is_octave_like_jump(left: f32, right: f32) -> bool {
    let ratio = left.max(right) / left.min(right).max(1.0);
    ratio > 1.85
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn stddev(values: &[f32], mean: f32) -> f32 {
    (values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / values.len() as f32)
        .sqrt()
}

fn median(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted[sorted.len() / 2]
}

fn median_absolute_deviation(values: &[f32]) -> f32 {
    let center = median(values);
    let deviations: Vec<f32> = values.iter().map(|value| (value - center).abs()).collect();
    median(&deviations)
}

fn mean_abs_successive_period_delta_segments(segments: &[&[f32]]) -> f32 {
    let mut total = 0.0_f32;
    let mut count = 0usize;
    for segment in segments {
        if segment.len() < 2 {
            continue;
        }
        for pair in segment.windows(2) {
            total += (pair[1] - pair[0]).abs();
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

fn averaged_period_perturbation(segments: &[&[f32]], window: usize, mean_period: f32) -> f32 {
    if window < 3 || mean_period <= 0.0 {
        return 0.0;
    }

    let radius = window / 2;
    let mut deltas = Vec::new();
    for segment in segments {
        if segment.len() < window {
            continue;
        }
        for center in radius..(segment.len() - radius) {
            let mut neighbor_sum = 0.0_f32;
            let mut neighbor_count = 0usize;
            for (index, period) in segment[center - radius..=center + radius].iter().enumerate() {
                if index == radius {
                    continue;
                }
                neighbor_sum += *period;
                neighbor_count += 1;
            }
            if neighbor_count == 0 {
                continue;
            }
            let local_mean = neighbor_sum / neighbor_count as f32;
            deltas.push((segment[center] - local_mean).abs());
        }
    }

    if deltas.is_empty() {
        0.0
    } else {
        mean(&deltas) / mean_period
    }
}
