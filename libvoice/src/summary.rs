use crate::model::{
    ChunkAnalysis, FrameFeatures, HarmonicStats, HarmonicSummary, OverallAnalysis, SpectralSummary,
};
use crate::stats::{summarize_optional, summarize_required};

pub(crate) fn empty_overall(processed_samples: usize) -> OverallAnalysis {
    OverallAnalysis {
        processed_samples,
        frame_count: 0,
        pitch_hz: None,
        spectral: None,
        harmonics: None,
        energy: None,
        jitter: None,
    }
}

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
        harmonics: summarize_harmonics(frames),
        energy: summarize_required(frames.iter().map(|f| f.energy)),
        jitter: None,
    }
}

pub(crate) fn summarize_overall(
    processed_samples: usize,
    frames: &[FrameFeatures],
    _frame_step_seconds: f32,
) -> OverallAnalysis {
    if frames.is_empty() {
        return empty_overall(processed_samples);
    }

    OverallAnalysis {
        processed_samples,
        frame_count: frames.len(),
        pitch_hz: summarize_optional(summarized_pitch_values(frames).into_iter()),
        spectral: summarize_spectral(frames),
        harmonics: summarize_harmonics(frames),
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
        tilt_db_per_octave: summarize_required(
            frames.iter().map(|f| f.spectral_tilt_db_per_octave),
        )
        .unwrap(),
        zcr: summarize_required(frames.iter().map(|f| f.zcr)).unwrap(),
        rms: summarize_required(frames.iter().map(|f| f.rms)).unwrap(),
        loudness_dbfs: summarize_required(frames.iter().map(|f| f.loudness_dbfs)).unwrap(),
        hnr_db: summarize_required(frames.iter().map(|f| f.hnr_db)).unwrap(),
    })
}

fn summarize_harmonics(frames: &[FrameFeatures]) -> Option<HarmonicSummary> {
    let max_harmonics = frames
        .iter()
        .map(|frame| frame.harmonic_strengths.len())
        .max()
        .unwrap_or(0);
    if max_harmonics == 0 {
        return None;
    }

    let harmonics: Vec<HarmonicStats> = (0..max_harmonics)
        .filter_map(|index| {
            let strength_ratio = summarize_optional(
                frames
                    .iter()
                    .filter_map(|frame| frame.harmonic_strengths.get(index).copied().flatten()),
            )?;
            Some(HarmonicStats {
                harmonic_number: index + 1,
                strength_ratio,
            })
        })
        .collect();

    if harmonics.is_empty() {
        None
    } else {
        let max_frequency_hz = frames
            .iter()
            .filter_map(|frame| {
                frame
                    .pitch_hz
                    .map(|pitch_hz| pitch_hz * frame.harmonic_strengths.len() as f32)
            })
            .fold(0.0_f32, f32::max);

        Some(HarmonicSummary {
            normalized_to_f0: true,
            max_frequency_hz,
            harmonics,
        })
    }
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
    let mut window = [0.0_f32; 5];
    for index in 0..raw.len() {
        let start = index.saturating_sub(radius);
        let end = (index + radius + 1).min(raw.len());
        let mut len = 0usize;
        for &value in &raw[start..end] {
            window[len] = value;
            len += 1;
        }
        window[..len].sort_unstable_by(|a, b| a.total_cmp(b));
        let median = window[len / 2];
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
