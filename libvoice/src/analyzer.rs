use crate::config::AnalyzerConfig;
use crate::model::{
    AnalysisReport, ChunkAnalysis, FftSpectrum, FftSpectrumFrame, FrameAnalysis, FrameFeatures,
    OverallAnalysis,
};
use crate::signal::hann_window;
use crate::spectral::FrameAnalyzer;
use crate::summary::{summarize_chunk, summarize_overall};

#[derive(Debug, Clone, Default)]
pub struct AnalysisOutputOptions {
    pub fft_spectrum: bool,
}

pub struct VoiceAnalyzer {
    config: AnalyzerConfig,
    output_options: AnalysisOutputOptions,
    frame_analyzer: FrameAnalyzer,
    pending: Vec<f32>,
    pending_start: usize,
    processed_samples: usize,
    next_chunk_index: usize,
    next_frame_index: usize,
    overall_frames: Vec<FrameFeatures>,
    fft_spectrum_frames: Option<Vec<FftSpectrumFrame>>,
}

impl VoiceAnalyzer {
    pub fn new(config: AnalyzerConfig) -> Self {
        Self::new_with_output_options(config, AnalysisOutputOptions::default())
    }

    pub fn new_with_output_options(
        config: AnalyzerConfig,
        output_options: AnalysisOutputOptions,
    ) -> Self {
        let window = hann_window(config.frame_size);
        let frame_analyzer = FrameAnalyzer::new(config.clone(), window);
        let fft_spectrum_frames = output_options.fft_spectrum.then(Vec::new);

        Self {
            config,
            output_options,
            frame_analyzer,
            pending: Vec::new(),
            pending_start: 0,
            processed_samples: 0,
            next_chunk_index: 0,
            next_frame_index: 0,
            overall_frames: Vec::new(),
            fft_spectrum_frames,
        }
    }

    pub fn config(&self) -> &AnalyzerConfig {
        &self.config
    }

    pub fn process_chunk(&mut self, samples: &[f32]) -> ChunkAnalysis {
        self.process_chunk_with_frames(samples).0
    }

    pub fn process_chunk_with_frames(
        &mut self,
        samples: &[f32],
    ) -> (ChunkAnalysis, Vec<FrameAnalysis>) {
        self.pending.extend_from_slice(samples);
        self.processed_samples += samples.len();

        let mut frame_features = Vec::new();
        let mut frames = Vec::new();
        while self.pending_start + self.config.frame_size <= self.pending.len() {
            let frame_start_sample =
                self.processed_samples - self.pending.len() + self.pending_start;
            let frame =
                &self.pending[self.pending_start..self.pending_start + self.config.frame_size];
            let features = self.frame_analyzer.analyze(frame);
            self.pending_start += self.config.hop_size;
            let is_voiced = self.is_voiced_frame(&features);
            self.capture_fft_spectrum(frame_start_sample, is_voiced);
            if is_voiced {
                self.overall_frames.push(features.clone());
                frame_features.push(features.clone());
                let cumulative = summarize_overall(
                    frame_start_sample + self.config.frame_size,
                    &self.overall_frames,
                    0.0,
                );
                frames.push(self.build_frame_analysis(frame_start_sample, features, cumulative));
            }
        }

        if let Some(last_frame) = frames.last_mut() {
            last_frame.cumulative.processed_samples = self.processed_samples;
        }

        self.compact_pending();
        let chunk = summarize_chunk(
            self.next_chunk_index,
            samples.len(),
            &frame_features,
            self.config.frame_step_seconds(),
        );
        self.next_chunk_index += 1;
        (chunk, frames)
    }

    pub fn finalize(&self) -> OverallAnalysis {
        summarize_overall(
            self.processed_samples,
            &self.overall_frames,
            self.config.frame_step_seconds(),
        )
    }

    pub fn analyze_buffer(config: AnalyzerConfig, samples: &[f32]) -> AnalysisReport {
        Self::analyze_buffer_with_output_options(config, samples, AnalysisOutputOptions::default())
    }

    pub fn analyze_buffer_with_output_options(
        config: AnalyzerConfig,
        samples: &[f32],
        output_options: AnalysisOutputOptions,
    ) -> AnalysisReport {
        let mut analyzer = Self::new_with_output_options(config, output_options);
        let (chunk, frames) = analyzer.process_chunk_with_frames(samples);
        let overall = analyzer.finalize();
        AnalysisReport {
            config: analyzer.config.clone(),
            frames,
            chunks: vec![chunk],
            overall,
            fft_spectrum: analyzer.finalize_fft_spectrum(),
        }
    }

    pub fn analyze_buffer_in_chunks(
        config: AnalyzerConfig,
        samples: &[f32],
        input_chunk_size: usize,
    ) -> AnalysisReport {
        Self::analyze_buffer_in_chunks_with_output_options(
            config,
            samples,
            input_chunk_size,
            AnalysisOutputOptions::default(),
        )
    }

    pub fn analyze_buffer_in_chunks_with_output_options(
        config: AnalyzerConfig,
        samples: &[f32],
        input_chunk_size: usize,
        output_options: AnalysisOutputOptions,
    ) -> AnalysisReport {
        let mut analyzer = Self::new_with_output_options(config, output_options);
        let mut chunks = Vec::new();
        let mut frames = Vec::new();
        for piece in samples.chunks(input_chunk_size.max(1)) {
            let (chunk, chunk_frames) = analyzer.process_chunk_with_frames(piece);
            chunks.push(chunk);
            frames.extend(chunk_frames);
        }
        let overall = analyzer.finalize();
        AnalysisReport {
            config: analyzer.config.clone(),
            frames,
            chunks,
            overall,
            fft_spectrum: analyzer.finalize_fft_spectrum(),
        }
    }

    fn compact_pending(&mut self) {
        if self.pending_start == 0 {
            return;
        }

        let remaining = self.pending.len() - self.pending_start;
        self.pending.copy_within(self.pending_start.., 0);
        self.pending.truncate(remaining);
        self.pending_start = 0;
    }

    fn is_voiced_frame(&self, features: &FrameFeatures) -> bool {
        features.pitch_hz.is_some()
            && features.pitch_clarity >= self.config.pitch_clarity_threshold
            && features.rms >= self.config.voiced_rms_threshold
            && features.trailing_rms >= self.config.voiced_rms_threshold * 0.8
            && features.trailing_rms >= features.rms * 0.45
            && features.spectral_flatness <= self.config.voiced_max_spectral_flatness
            && features.zcr <= self.config.voiced_max_zero_crossing_rate
    }

    fn capture_fft_spectrum(&mut self, frame_start_sample: usize, is_voiced: bool) {
        if !self.output_options.fft_spectrum {
            return;
        }

        let Some(frames) = self.fft_spectrum_frames.as_mut() else {
            return;
        };

        let sample_rate = self.config.sample_rate as f32;
        let end_sample = frame_start_sample + self.config.frame_size;
        frames.push(FftSpectrumFrame {
            start_seconds: frame_start_sample as f32 / sample_rate,
            end_seconds: end_sample as f32 / sample_rate,
            is_voiced,
            magnitudes: self.frame_analyzer.magnitudes().to_vec(),
        });
    }

    fn finalize_fft_spectrum(&self) -> Option<FftSpectrum> {
        let frames = self.fft_spectrum_frames.as_ref()?;
        if frames.is_empty() {
            return None;
        }

        Some(FftSpectrum {
            frame_size: self.config.frame_size,
            hop_size: self.config.hop_size,
            bin_hz: self.config.sample_rate as f32 / self.config.frame_size as f32,
            frames: frames.clone(),
        })
    }

    fn build_frame_analysis(
        &mut self,
        frame_start_sample: usize,
        features: FrameFeatures,
        cumulative: OverallAnalysis,
    ) -> FrameAnalysis {
        let frame_index = self.next_frame_index;
        self.next_frame_index += 1;

        let end_sample = frame_start_sample + self.config.frame_size;
        let sample_rate = self.config.sample_rate as f32;

        FrameAnalysis {
            frame_index,
            start_sample: frame_start_sample,
            start_seconds: frame_start_sample as f32 / sample_rate,
            end_sample,
            end_seconds: end_sample as f32 / sample_rate,
            pitch_hz: features.pitch_hz,
            pitch_clarity: features.pitch_clarity,
            spectral_rolloff_hz: features.spectral_rolloff_hz,
            spectral_centroid_hz: features.spectral_centroid_hz,
            spectral_bandwidth_hz: features.spectral_bandwidth_hz,
            spectral_flatness: features.spectral_flatness,
            spectral_tilt_db_per_octave: features.spectral_tilt_db_per_octave,
            zcr: features.zcr,
            rms: features.rms,
            loudness_dbfs: features.loudness_dbfs,
            hnr_db: features.hnr_db,
            energy: features.energy,
            harmonic_strengths: features.harmonic_strengths,
            cumulative,
        }
    }
}
