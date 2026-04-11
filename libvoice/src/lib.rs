mod analyzer;
mod config;
mod formant;
mod model;
mod signal;
mod spectral;
mod stats;
mod summary;

pub use analyzer::VoiceAnalyzer;
pub use analyzer::AnalysisOutputOptions;
pub use config::AnalyzerConfig;
pub use model::{
    AnalysisReport, ChunkAnalysis, FftSpectrum, FftSpectrumFrame, FormantStats, FormantSummary,
    FrameAnalysis, JitterMetrics, OverallAnalysis, SpectralSummary, SummaryStats,
};

#[cfg(test)]
mod tests {
    use super::{AnalysisOutputOptions, AnalyzerConfig, VoiceAnalyzer};
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
        assert_eq!(report.frames.len(), report.overall.frame_count);
        assert!(report.overall.frame_count > 0);
        let pitch = report.overall.pitch_hz.unwrap();
        assert!(
            (pitch.mean - 220.0).abs() < 15.0,
            "pitch mean = {}",
            pitch.mean
        );
        let energy = report.overall.energy.unwrap();
        assert!(energy.mean > 0.01);
        let loudness = report.overall.spectral.unwrap().loudness_dbfs;
        assert!(loudness.mean.is_finite());
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

    #[test]
    fn can_include_fft_spectrum_output() {
        let sample_rate = 16_000;
        let samples = synth_sine(sample_rate, 220.0, 1.0);
        let report = VoiceAnalyzer::analyze_buffer_with_output_options(
            AnalyzerConfig::new(sample_rate),
            &samples,
            AnalysisOutputOptions { fft_spectrum: true },
        );

        let spectrum = report.fft_spectrum.expect("expected fft spectrum");
        assert_eq!(spectrum.frame_size, 2048);
        assert_eq!(spectrum.hop_size, 512);
        assert!(!spectrum.frames.is_empty());
        assert!(spectrum.frames.iter().any(|frame| frame.is_voiced));

        let first_voiced = spectrum
            .frames
            .iter()
            .find(|frame| frame.is_voiced)
            .unwrap();
        let peak_bin = first_voiced
            .magnitudes
            .iter()
            .enumerate()
            .skip(1)
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(index, _)| index)
            .unwrap();
        let peak_hz = peak_bin as f32 * spectrum.bin_hz;
        assert!((peak_hz - 220.0).abs() < 20.0, "peak_hz = {}", peak_hz);
    }
}
