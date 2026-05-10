use tauri::{AppHandle, State};
use crossbeam_channel::bounded;
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;

use crate::state::{AppState, RecordingData, NoteAccuracy};
use crate::audio;
use crate::midi;
use crate::pitch::midi_to_note_name;

fn normalize_audio_path(path: &str) -> Result<PathBuf, String> {
    let parsed = if path.starts_with("file://") {
        let url = url::Url::parse(path).map_err(|e| format!("Invalid file URL: {e}"))?;
        url.to_file_path()
            .map_err(|_| format!("Invalid file URL path: {path}"))?
    } else {
        PathBuf::from(path)
    };

    let absolute = if parsed.is_absolute() {
        parsed
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&parsed))
            .map_err(|e| format!("Could not resolve relative audio path '{}': {e}", parsed.display()))?
    };

    if !Path::new(&absolute).exists() {
        return Err(format!("Audio file does not exist: {}", absolute.display()));
    }

    absolute
        .canonicalize()
        .map_err(|e| format!("Could not canonicalize audio path '{}': {e}", absolute.display()))
}

#[tauri::command]
pub async fn get_audio_devices() -> Vec<String> {
    audio::get_audio_devices()
}

#[tauri::command]
pub async fn start_audio_capture(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut inner = state.inner.lock();
    if inner.is_capturing {
        return Ok(());
    }

    let (stop_tx, stop_rx) = bounded(1);
    inner.is_capturing = true;
    inner.pitch_history.clear();
    inner.capture_stop_tx = Some(stop_tx);

    let inner_arc = state.inner.clone();
    drop(inner);

    audio::start_capture(app, inner_arc, stop_rx)
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_audio_capture(state: State<'_, AppState>) -> Result<(), String> {
    let mut inner = state.inner.lock();
    if let Some(tx) = inner.capture_stop_tx.take() {
        let _ = tx.send(());
    }
    inner.is_capturing = false;
    Ok(())
}

#[tauri::command]
pub async fn load_midi_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<crate::state::MidiData, String> {
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    let bpm_override = {
        let inner = state.inner.lock();
        // Use user-set BPM only if it differs from default
        if inner.bpm != 120.0 { Some(inner.bpm) } else { None }
    };

    let midi_data = midi::parse_midi(&data, bpm_override)
    .map_err(|e| e.to_string())?;

    let mut inner = state.inner.lock();
    // Update BPM and time sig from MIDI if not already overridden
    inner.bpm = midi_data.tempo_bpm;
    inner.time_sig_num = midi_data.time_sig_num;
    inner.time_sig_den = midi_data.time_sig_den;
    inner.midi_data = Some(midi_data.clone());

    Ok(midi_data)
}

#[tauri::command]
pub async fn load_audio_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let normalized = normalize_audio_path(&path)?;
    let mut inner = state.inner.lock();
    inner.audio_file_path = Some(normalized.to_string_lossy().into_owned());
    Ok(())
}

#[tauri::command]
pub async fn start_playback(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let inner = state.inner.lock();
    if inner.is_playing {
        return Ok(());
    }

    let audio_path = inner.audio_file_path.clone();
    let bpm = inner.bpm;
    let ticks_per_beat = inner.midi_data.as_ref()
    .map(|m| m.ticks_per_beat)
    .unwrap_or(480);

    drop(inner);

    let (stop_tx, stop_rx) = bounded(1);
    state.inner.lock().playback_stop_tx = Some(stop_tx);
    state.inner.lock().is_playing = true;

    if let Some(path) = audio_path {
        audio::start_playback_audio(app, path, bpm, ticks_per_beat, stop_rx)
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn stop_playback(state: State<'_, AppState>) -> Result<(), String> {
    let mut inner = state.inner.lock();
    if let Some(tx) = inner.playback_stop_tx.take() {
        let _ = tx.send(());
    }
    inner.is_playing = false;
    Ok(())
}

#[tauri::command]
pub async fn set_bpm(bpm: f64, state: State<'_, AppState>) -> Result<(), String> {
    state.inner.lock().bpm = bpm;
    Ok(())
}

#[tauri::command]
pub async fn set_time_signature(
    num: u8,
    den: u8,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut inner = state.inner.lock();
    inner.time_sig_num = num;
    inner.time_sig_den = den;
    Ok(())
}

#[tauri::command]
pub async fn get_recording_data(state: State<'_, AppState>) -> Result<RecordingData, String> {
    let inner = state.inner.lock();
    let frames = inner.pitch_history.clone();

    if frames.is_empty() {
        return Ok(RecordingData {
            pitch_frames: vec![],
            duration_ms: 0.0,
            note_accuracies: vec![],
            overall_accuracy: 0.0,
        });
    }

    let duration_ms = frames.last().map(|f| f.timestamp_ms).unwrap_or(0.0);

    // Calculate note accuracies against MIDI (per-note-instance)
    let mut note_accuracies: Vec<NoteAccuracy> = Vec::new();
    let mut overall_accuracy = 0.0;

    if let Some(midi) = &inner.midi_data {
        let mut total_acc = 0.0;
        let mut count = 0;

        // Calculate accuracy for each individual MIDI note instance
        for midi_note in &midi.notes {
            let relevant_frames: Vec<&crate::state::PitchFrame> = frames.iter()
            .filter(|f| {
                f.timestamp_ms >= midi_note.start_ms
                && f.timestamp_ms <= midi_note.end_ms
                && f.midi_note == Some(midi_note.pitch)
            })
            .collect();

            let total_frames = frames.iter()
            .filter(|f| {
                f.timestamp_ms >= midi_note.start_ms
                && f.timestamp_ms <= midi_note.end_ms
                && f.confidence > 0.1
            })
            .count();

            let hit_count = relevant_frames.len();
            let hit_pct = if total_frames > 0 {
                hit_count as f64 / total_frames as f64 * 100.0
            } else {
                0.0
            };

            let deviations: Vec<f64> = relevant_frames.iter()
            .map(|f| f.cents_deviation.abs())
            .collect();

            let avg_dev = if deviations.is_empty() {
                0.0
            } else {
                deviations.iter().sum::<f64>() / deviations.len() as f64
            };
            
            let max_dev = deviations.iter().cloned().fold(0.0f64, f64::max);

            note_accuracies.push(NoteAccuracy {
                midi_note: midi_note.pitch,
                note_name: midi_to_note_name(midi_note.pitch),
                hit_percentage: hit_pct,
                avg_deviation_cents: avg_dev,
                max_deviation_cents: max_dev,
            });

            total_acc += hit_pct;
            count += 1;
        }

        if count > 0 {
            overall_accuracy = total_acc / count as f64;
        }
    }

    Ok(RecordingData {
        pitch_frames: frames,
       duration_ms,
       note_accuracies,
       overall_accuracy,
    })
}
