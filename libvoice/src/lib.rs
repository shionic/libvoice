mod analyzer;
mod config;
mod formant;
mod model;
mod signal;
mod spectral;
mod stats;
mod summary;

pub use analyzer::VoiceAnalyzer;
pub use config::AnalyzerConfig;
pub use model::{
    AnalysisReport, ChunkAnalysis, FormantStats, FormantSummary, FrameAnalysis, JitterMetrics,
    OverallAnalysis, SpectralSummary, SummaryStats,
};

#[cfg(test)]
mod tests {
    use super::{AnalyzerConfig, VoiceAnalyzer};
    use core::f32::consts::PI;

    fn synth_sine(sample_rate: u32, frequency_hz: f32, seconds: f32) -> Vec<f32> {
        let total = (sample_rate as f32 * seconds) as usize;
        (0..total)
            .map(|index| {
                let t = index as f32 / sample_rate as f32;
                (2.0 * PI * frequency_hz * t).sin() * 0.5
            })
            .collect()
    }

    #[test]
    fn analyzes_full_buffer() {
        let sample_rate = 16_000;
        let samples = synth_sine(sample_rate, 220.0, 1.0);
        let report = VoiceAnalyzer::analyze_buffer(AnalyzerConfig::new(sample_rate), &samples);

        assert_eq!(report.chunks.len(), 1);
        assert!(report.overall.frame_count > 0);
        let pitch = report.overall.pitch_hz.unwrap();
        assert!(
            (pitch.mean - 220.0).abs() < 15.0,
            "pitch mean = {}",
            pitch.mean
        );
        let energy = report.overall.energy.unwrap();
        assert!(energy.mean > 0.01);
    }

    #[test]
    fn streaming_matches_single_pass_frame_count() {
        let sample_rate = 16_000;
        let samples = synth_sine(sample_rate, 180.0, 1.0);
        let config = AnalyzerConfig::new(sample_rate);

        let single = VoiceAnalyzer::analyze_buffer(config.clone(), &samples);

        let mut streaming = VoiceAnalyzer::new(config);
        let mut chunks = Vec::new();
        for piece in samples.chunks(700) {
            chunks.push(streaming.process_chunk(piece));
        }
        let overall = streaming.finalize();

        assert!(chunks.iter().any(|chunk| chunk.frame_count > 0));
        assert_eq!(single.overall.frame_count, overall.frame_count);
    }
}
