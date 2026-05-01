use crate::model::{
    ChunkAnalysis, FrameFeatures, HarmonicStats, HarmonicSummary, OverallAnalysis, SpectralSummary,
    SummaryStats,
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

pub(crate) struct IncrementalSummarizer {
    raw_pitches: Vec<f32>,
    rolloffs: Vec<f32>,
    centroids: Vec<f32>,
    bandwidths: Vec<f32>,
    flatness: Vec<f32>,
    tilts: Vec<f32>,
    zcrs: Vec<f32>,
    rmss: Vec<f32>,
    loudness: Vec<f32>,
    hnrs: Vec<f32>,
    energies: Vec<f32>,
    harmonic_max_frequencies: Vec<f32>,
    harmonic_strengths: Vec<Vec<f32>>,
}

impl IncrementalSummarizer {
    pub(crate) fn new() -> Self {
        Self {
            raw_pitches: Vec::new(),
            rolloffs: Vec::new(),
            centroids: Vec::new(),
            bandwidths: Vec::new(),
            flatness: Vec::new(),
            tilts: Vec::new(),
            zcrs: Vec::new(),
            rmss: Vec::new(),
            loudness: Vec::new(),
            hnrs: Vec::new(),
            energies: Vec::new(),
            harmonic_max_frequencies: Vec::new(),
            harmonic_strengths: Vec::new(),
        }
    }

    pub(crate) fn add_frame(&mut self, features: &FrameFeatures) {
        if let Some(pitch) = features.pitch_hz {
            self.raw_pitches.push(pitch);
        }
        insert_sorted(&mut self.rolloffs, features.spectral_rolloff_hz);
        insert_sorted(&mut self.centroids, features.spectral_centroid_hz);
        insert_sorted(&mut self.bandwidths, features.spectral_bandwidth_hz);
        insert_sorted(&mut self.flatness, features.spectral_flatness);
        insert_sorted(&mut self.tilts, features.spectral_tilt_db_per_octave);
        insert_sorted(&mut self.zcrs, features.zcr);
        insert_sorted(&mut self.rmss, features.rms);
        insert_sorted(&mut self.loudness, features.loudness_dbfs);
        insert_sorted(&mut self.hnrs, features.hnr_db);
        insert_sorted(&mut self.energies, features.energy);

        if let Some(f0_hz) = features.pitch_hz {
            self.harmonic_max_frequencies
                .push(f0_hz * features.harmonic_strengths.len() as f32);
        }

        for (i, strength) in features.harmonic_strengths.iter().enumerate() {
            if let Some(s) = strength {
                if i >= self.harmonic_strengths.len() {
                    self.harmonic_strengths.resize(i + 1, Vec::new());
                }
                insert_sorted(&mut self.harmonic_strengths[i], *s);
            }
        }
    }

    pub(crate) fn summarize(&self, processed_samples: usize, frame_count: usize) -> OverallAnalysis {
        if frame_count == 0 {
            return empty_overall(processed_samples);
        }

        let pitch_hz = if self.raw_pitches.is_empty() {
            None
        } else {
            let smoothed = median_smooth_pitch_contour(&repair_pitch_outliers(self.raw_pitches.clone()), 2);
            summarize_optional(smoothed.into_iter())
        };

        OverallAnalysis {
            processed_samples,
            frame_count,
            pitch_hz,
            spectral: Some(SpectralSummary {
                rolloff_hz: summarize_sorted(&self.rolloffs).unwrap(),
                centroid_hz: summarize_sorted(&self.centroids).unwrap(),
                bandwidth_hz: summarize_sorted(&self.bandwidths).unwrap(),
                flatness: summarize_sorted(&self.flatness).unwrap(),
                tilt_db_per_octave: summarize_sorted(&self.tilts).unwrap(),
                zcr: summarize_sorted(&self.zcrs).unwrap(),
                rms: summarize_sorted(&self.rmss).unwrap(),
                loudness_dbfs: summarize_sorted(&self.loudness).unwrap(),
                hnr_db: summarize_sorted(&self.hnrs).unwrap(),
            }),
            harmonics: self.summarize_harmonics(),
            energy: summarize_sorted(&self.energies),
            jitter: None,
        }
    }

    fn summarize_harmonics(&self) -> Option<HarmonicSummary> {
        if self.harmonic_strengths.is_empty() {
            return None;
        }

        let harmonics: Vec<HarmonicStats> = self
            .harmonic_strengths
            .iter()
            .enumerate()
            .filter_map(|(i, strengths)| {
                let stats = summarize_sorted(strengths)?;
                Some(HarmonicStats {
                    harmonic_number: i + 1,
                    strength_ratio: stats,
                })
            })
            .collect();

        if harmonics.is_empty() {
            None
        } else {
            Some(HarmonicSummary {
                normalized_to_f0: true,
                max_frequency_hz: self
                    .harmonic_max_frequencies
                    .iter()
                    .copied()
                    .fold(0.0, f32::max),
                harmonics,
            })
        }
    }
}

fn insert_sorted(vec: &mut Vec<f32>, value: f32) {
    if !value.is_finite() {
        return;
    }
    let pos = vec.binary_search_by(|a| a.total_cmp(&value)).unwrap_or_else(|e| e);
    vec.insert(pos, value);
}

fn summarize_sorted(values: &[f32]) -> Option<SummaryStats> {
    if values.is_empty() {
        return None;
    }

    use crate::stats::percentile_sorted_ref;
    let count = values.len();
    let mean = values.iter().sum::<f32>() / count as f32;
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / (count as f32 - 1.0).max(1.0);

    Some(SummaryStats {
        count,
        mean,
        std: variance.sqrt(),
        median: percentile_sorted_ref(values, 0.5),
        min: values[0],
        max: values[count - 1],
        p5: percentile_sorted_ref(values, 0.05),
        p95: percentile_sorted_ref(values, 0.95),
    })
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
    let mut window = vec![0.0_f32; radius * 2 + 1];
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
