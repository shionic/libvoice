use crate::config::AnalyzerConfig;
use crate::model::{AnalysisReport, ChunkAnalysis, FrameAnalysis, FrameFeatures, OverallAnalysis};
use crate::signal::hann_window;
use crate::spectral::FrameAnalyzer;
use crate::summary::{summarize_chunk, summarize_overall};

pub struct VoiceAnalyzer {
    config: AnalyzerConfig,
    frame_analyzer: FrameAnalyzer,
    pending: Vec<f32>,
    pending_start: usize,
    processed_samples: usize,
    next_chunk_index: usize,
    next_frame_index: usize,
    overall_frames: Vec<FrameFeatures>,
}

impl VoiceAnalyzer {
    pub fn new(config: AnalyzerConfig) -> Self {
        let window = hann_window(config.frame_size);
        let frame_analyzer = FrameAnalyzer::new(config.clone(), window);

        Self {
            config,
            frame_analyzer,
            pending: Vec::new(),
            pending_start: 0,
            processed_samples: 0,
            next_chunk_index: 0,
            next_frame_index: 0,
            overall_frames: Vec::new(),
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
            if self.is_voiced_frame(&features) {
                self.overall_frames.push(features.clone());
                frame_features.push(features.clone());
                frames.push(self.build_frame_analysis(frame_start_sample, features));
            }
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
        let mut analyzer = Self::new(config);
        let chunk = analyzer.process_chunk(samples);
        let overall = analyzer.finalize();
        AnalysisReport {
            config: analyzer.config.clone(),
            chunks: vec![chunk],
            overall,
        }
    }

    pub fn analyze_buffer_in_chunks(
        config: AnalyzerConfig,
        samples: &[f32],
        input_chunk_size: usize,
    ) -> AnalysisReport {
        let mut analyzer = Self::new(config);
        let mut chunks = Vec::new();
        for piece in samples.chunks(input_chunk_size.max(1)) {
            chunks.push(analyzer.process_chunk(piece));
        }
        let overall = analyzer.finalize();
        AnalysisReport {
            config: analyzer.config.clone(),
            chunks,
            overall,
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
            && features.spectral_flatness <= self.config.voiced_max_spectral_flatness
            && features.zcr <= self.config.voiced_max_zero_crossing_rate
    }

    fn build_frame_analysis(
        &mut self,
        frame_start_sample: usize,
        features: FrameFeatures,
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
            zcr: features.zcr,
            rms: features.rms,
            hnr_db: features.hnr_db,
            energy: features.energy,
        }
    }
}
