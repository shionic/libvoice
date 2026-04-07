use crate::config::AnalyzerConfig;
use crate::model::{AnalysisReport, ChunkAnalysis, FrameFeatures, OverallAnalysis};
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
            overall_frames: Vec::new(),
        }
    }

    pub fn config(&self) -> &AnalyzerConfig {
        &self.config
    }

    pub fn process_chunk(&mut self, samples: &[f32]) -> ChunkAnalysis {
        self.pending.extend_from_slice(samples);
        self.processed_samples += samples.len();

        let mut frames = Vec::new();
        while self.pending_start + self.config.frame_size <= self.pending.len() {
            let frame =
                &self.pending[self.pending_start..self.pending_start + self.config.frame_size];
            let features = self.frame_analyzer.analyze(frame);
            self.pending_start += self.config.hop_size;
            if self.is_voiced_frame(&features) {
                self.overall_frames.push(features.clone());
                frames.push(features);
            }
        }

        self.compact_pending();
        let chunk = summarize_chunk(
            self.next_chunk_index,
            samples.len(),
            &frames,
            self.config.frame_step_seconds(),
        );
        self.next_chunk_index += 1;
        chunk
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
}
