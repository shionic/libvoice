use parking_lot::Mutex;
use std::sync::Arc;
use crossbeam_channel::{Sender, Receiver};

#[derive(Debug, Clone, serde::Serialize)]
pub struct PitchFrame {
    pub timestamp_ms: f64,
    pub frequency: f64,
    pub confidence: f64,
    pub note: Option<String>,
    pub cents_deviation: f64,
    pub midi_note: Option<u8>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MidiNote {
    pub start_tick: u32,
    pub end_tick: u32,
    pub start_ms: f64,
    pub end_ms: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MidiData {
    pub notes: Vec<MidiNote>,
    pub ticks_per_beat: u16,
    pub tempo_bpm: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub duration_ms: f64,
    pub total_ticks: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NoteAccuracy {
    pub midi_note: u8,
    pub note_name: String,
    pub hit_percentage: f64,
    pub avg_deviation_cents: f64,
    pub max_deviation_cents: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordingData {
    pub pitch_frames: Vec<PitchFrame>,
    pub duration_ms: f64,
    pub note_accuracies: Vec<NoteAccuracy>,
    pub overall_accuracy: f64,
}

pub struct AppStateInner {
    pub is_capturing: bool,
    pub is_playing: bool,
    pub bpm: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub midi_data: Option<MidiData>,
    pub audio_file_path: Option<String>,
    pub pitch_history: Vec<PitchFrame>,
    pub capture_stop_tx: Option<Sender<()>>,
    pub playback_stop_tx: Option<Sender<()>>,
    pub playback_start_ms: Option<f64>,
}

impl AppStateInner {
    pub fn new() -> Self {
        Self {
            is_capturing: false,
            is_playing: false,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            midi_data: None,
            audio_file_path: None,
            pitch_history: Vec::new(),
            capture_stop_tx: None,
            playback_stop_tx: None,
            playback_start_ms: None,
        }
    }
}

pub struct AppState {
    pub inner: Arc<Mutex<AppStateInner>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AppStateInner::new())),
        }
    }
}
