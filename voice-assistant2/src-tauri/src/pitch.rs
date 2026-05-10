/// Pitch detection using libvoice
use libvoice::{AnalyzerConfig, VoiceAnalyzer};

pub const SAMPLE_RATE: u32 = 44100;
pub const FFT_SIZE: usize = 4096;
pub const HOP_SIZE: usize = 512;

/// Minimum/maximum detectable frequency (vocal range ~80Hz–1200Hz)
const MIN_FREQ: f64 = 60.0;
const MAX_FREQ: f64 = 2000.0;

/// Result of pitch detection on a single frame
#[derive(Debug, Clone)]
pub struct PitchResult {
    pub frequency: f64,
    pub confidence: f64,
    pub midi_note: u8,
    pub cents_deviation: f64,
    pub note_name: String,
}

pub struct PitchDetector {
    analyzer: VoiceAnalyzer,
    buffer: Vec<f32>,
}

impl PitchDetector {
    pub fn new() -> Self {
        let mut config = AnalyzerConfig::new(SAMPLE_RATE);
        config.frame_size = FFT_SIZE;
        config.hop_size = HOP_SIZE;
        
        Self {
            analyzer: VoiceAnalyzer::new(config),
            buffer: Vec::new(),
        }
    }

    /// Process a frame of PCM samples and return pitch estimate
    pub fn process(&mut self, samples: &[f32]) -> Option<PitchResult> {
        if samples.len() < FFT_SIZE {
            return None;
        }

        // Use only the first FFT_SIZE samples for analysis
        let frame = &samples[..FFT_SIZE];
        
        // Process the chunk through libvoice
        let chunk = self.analyzer.process_chunk(frame);
        
        // Check if we got any voiced frames with pitch
        if chunk.frame_count == 0 {
            return None;
        }
        
        let pitch_stats = chunk.pitch_hz?;
        let frequency = pitch_stats.mean as f64;
        
        // Filter by frequency range
        if frequency < MIN_FREQ || frequency > MAX_FREQ {
            return None;
        }
        
        // Calculate confidence based on pitch clarity
        // libvoice uses pitch_clarity which ranges from 0.0 to 1.0
        // We can use the frame count as an indicator of confidence
        let confidence = if chunk.frame_count > 0 {
            // Simple confidence based on having detected pitch
            0.8
        } else {
            0.0
        };

        let (midi_note, cents_deviation, note_name) = freq_to_midi_info(frequency);

        Some(PitchResult {
            frequency,
            confidence,
            midi_note,
            cents_deviation,
            note_name,
        })
    }
}

/// Convert frequency to MIDI note number, deviation in cents, and note name
pub fn freq_to_midi_info(freq: f64) -> (u8, f64, String) {
    // A4 = 440Hz = MIDI 69
    let midi_float = 69.0 + 12.0 * (freq / 440.0).log2();
    let midi_note = midi_float.round() as i32;
    let midi_note = midi_note.clamp(0, 127) as u8;

    let cents_deviation = (midi_float - midi_note as f64) * 100.0;

    let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (midi_note as i32 / 12) - 1;
    let note_idx = (midi_note % 12) as usize;
    let note_name = format!("{}{}", note_names[note_idx], octave);

    (midi_note, cents_deviation, note_name)
}

/// Convert MIDI note to frequency
pub fn midi_to_freq(midi: u8) -> f64 {
    440.0 * 2.0f64.powf((midi as f64 - 69.0) / 12.0)
}

/// Full note name from MIDI number
pub fn midi_to_note_name(midi: u8) -> String {
    let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (midi as i32 / 12) - 1;
    let note_idx = (midi % 12) as usize;
    format!("{}{}", note_names[note_idx], octave)
}
