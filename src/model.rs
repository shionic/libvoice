use crate::config::AnalyzerConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SummaryStats {
    pub count: usize,
    pub mean: f32,
    pub std: f32,
    pub median: f32,
    pub min: f32,
    pub max: f32,
    pub p5: f32,
    pub p95: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpectralSummary {
    pub rolloff_hz: SummaryStats,
    pub centroid_hz: SummaryStats,
    pub bandwidth_hz: SummaryStats,
    pub flatness: SummaryStats,
    pub zcr: SummaryStats,
    pub rms: SummaryStats,
    pub hnr_db: SummaryStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JitterMetrics {
    pub sample_count: usize,
    pub local_ratio: f32,
    pub local_absolute_seconds: f32,
    pub rap_ratio: f32,
    pub ppq5_ratio: f32,
    pub ddp_ratio: f32,
    pub local_hz_mean: f32,
    pub local_hz_std: f32,
    pub local_ratio_mean: f32,
    pub local_ratio_std: f32,
    pub direction_change_rate: f32,
    pub rapid_change_ratio: f32,
    pub estimated_vibrato_hz: f32,
    pub estimated_vibrato_extent_cents: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkAnalysis {
    pub chunk_index: usize,
    pub input_samples: usize,
    pub frame_count: usize,
    pub pitch_hz: Option<SummaryStats>,
    pub spectral: Option<SpectralSummary>,
    pub energy: Option<SummaryStats>,
    pub jitter: Option<JitterMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverallAnalysis {
    pub processed_samples: usize,
    pub frame_count: usize,
    pub pitch_hz: Option<SummaryStats>,
    pub spectral: Option<SpectralSummary>,
    pub energy: Option<SummaryStats>,
    pub jitter: Option<JitterMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisReport {
    pub config: AnalyzerConfig,
    pub chunks: Vec<ChunkAnalysis>,
    pub overall: OverallAnalysis,
}

#[derive(Debug, Clone)]
pub(crate) struct FrameFeatures {
    pub(crate) pitch_hz: Option<f32>,
    pub(crate) period_seconds: Option<f32>,
    pub(crate) pitch_clarity: f32,
    pub(crate) spectral_rolloff_hz: f32,
    pub(crate) spectral_centroid_hz: f32,
    pub(crate) spectral_bandwidth_hz: f32,
    pub(crate) spectral_flatness: f32,
    pub(crate) zcr: f32,
    pub(crate) rms: f32,
    pub(crate) hnr_db: f32,
    pub(crate) energy: f32,
}
