use crate::model::{
    ChunkAnalysis, FormantStats, FormantSummary, FrameFeatures, OverallAnalysis, SpectralSummary,
};
use crate::stats::{summarize_optional, summarize_required};

pub(crate) fn summarize_chunk(
    chunk_index: usize,
    input_samples: usize,
    frames: &[FrameFeatures],
    _frame_step_seconds: f32,
) -> ChunkAnalysis {
    ChunkAnalysis {
        chunk_index,
        input_samples,
        frame_count: frames.len(),
        pitch_hz: summarize_optional(summarized_pitch_values(frames).into_iter()),
        spectral: summarize_spectral(frames),
        formants: summarize_formants(frames),
        energy: summarize_required(frames.iter().map(|f| f.energy)),
        jitter: None,
    }
}

pub(crate) fn summarize_overall(
    processed_samples: usize,
    frames: &[FrameFeatures],
    _frame_step_seconds: f32,
) -> OverallAnalysis {
    OverallAnalysis {
        processed_samples,
        frame_count: frames.len(),
        pitch_hz: summarize_optional(summarized_pitch_values(frames).into_iter()),
        spectral: summarize_spectral(frames),
        formants: summarize_formants(frames),
        energy: summarize_required(frames.iter().map(|f| f.energy)),
        jitter: None,
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

fn summarize_formants(frames: &[FrameFeatures]) -> Option<FormantSummary> {
    let f1 = summarize_formant_slot(frames, 0);
    let f2 = summarize_formant_slot(frames, 1);
    let f3 = summarize_formant_slot(frames, 2);
    let f4 = summarize_formant_slot(frames, 3);

    if f1.is_none() && f2.is_none() && f3.is_none() && f4.is_none() {
        None
    } else {
        Some(FormantSummary { f1, f2, f3, f4 })
    }
}

fn summarize_formant_slot(frames: &[FrameFeatures], index: usize) -> Option<FormantStats> {
    let frequency_hz = summarize_optional(frames.iter().filter_map(|frame| {
        frame
            .formants
            .get(index)
            .map(|formant| formant.frequency_hz)
    }))?;
    let bandwidth_hz = summarize_optional(frames.iter().filter_map(|frame| {
        frame
            .formants
            .get(index)
            .map(|formant| formant.bandwidth_hz)
    }))?;

    Some(FormantStats {
        frequency_hz,
        bandwidth_hz,
    })
}

fn summarized_pitch_values(frames: &[FrameFeatures]) -> Vec<f32> {
    median_smooth_pitch_contour(&repair_pitch_outliers(raw_pitch_contour(frames)), 2)
}

fn raw_pitch_contour(frames: &[FrameFeatures]) -> Vec<f32> {
    frames.iter().filter_map(|f| f.pitch_hz).collect()
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
