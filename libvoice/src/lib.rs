mod analyzer;
mod config;
mod harmonic;
mod model;
mod signal;
mod spectral;
mod stats;
mod summary;

pub use analyzer::AnalysisOutputOptions;
pub use analyzer::VoiceAnalyzer;
pub use config::AnalyzerConfig;
pub use model::{
    AnalysisReport, ChunkAnalysis, FftSpectrum, FftSpectrumFrame, FrameAnalysis, HarmonicStats,
    HarmonicSummary, JitterMetrics, OverallAnalysis, SpectralSummary, SummaryStats,
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
            AnalysisOutputOptions {
                frame_analysis: true,
                fft_spectrum: true,
            },
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

    #[test]
    fn can_skip_frame_analysis_when_only_summaries_are_needed() {
        let sample_rate = 16_000;
        let samples = synth_sine(sample_rate, 220.0, 1.0);
        let report = VoiceAnalyzer::analyze_buffer_with_output_options(
            AnalyzerConfig::new(sample_rate),
            &samples,
            AnalysisOutputOptions {
                frame_analysis: false,
                fft_spectrum: false,
            },
        );

        assert!(report.frames.is_empty());
        assert!(report.overall.frame_count > 0);
        assert_eq!(report.chunks.len(), 1);
        assert_eq!(report.chunks[0].frame_count, report.overall.frame_count);
    }

    #[test]
    fn higher_sample_rates_use_larger_default_windows() {
        let low = AnalyzerConfig::new(16_000);
        let high = AnalyzerConfig::new(48_000);

        assert_eq!(low.frame_size, 2_048);
        assert_eq!(low.hop_size, 512);
        assert_eq!(high.frame_size, 6_144);
        assert_eq!(high.hop_size, 1_536);

        let low_bin_hz = low.sample_rate as f32 / low.frame_size as f32;
        let high_bin_hz = high.sample_rate as f32 / high.frame_size as f32;
        assert!((low_bin_hz - 7.8125).abs() < 1.0e-6);
        assert!((high_bin_hz - low_bin_hz).abs() < 1.0e-6);
    }

    #[test]
    fn high_pitch_mode_expands_pitch_range_and_voiced_zcr_limit() {
        let mut config = AnalyzerConfig::new(16_000);
        config.apply_high_pitch_mode();

        assert_eq!(config.max_pitch_hz, 1_200.0);
        assert!(config.max_harmonic_frequency_hz > 5_000.0);
        assert!(config.max_harmonic_frequency_hz <= 8_000.0);
        assert!(config.voiced_max_zero_crossing_rate >= 0.30);
        assert!(config.voiced_max_zero_crossing_rate <= 0.40);
    }
}
