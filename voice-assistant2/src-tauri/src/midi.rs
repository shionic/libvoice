use midly::{Smf, TrackEventKind, MidiMessage, MetaMessage, Timing};
use anyhow::{Result, Context};
use std::collections::HashMap;

use crate::state::{MidiData, MidiNote};

pub fn parse_midi(data: &[u8], override_bpm: Option<f64>) -> Result<MidiData> {
    let smf = Smf::parse(data).context("Failed to parse MIDI file")?;

    let ticks_per_beat = match smf.header.timing {
        Timing::Metrical(tpb) => tpb.as_int() as u16,
        Timing::Timecode(fps, subdivisions) => {
            // Convert timecode to approximate ticks per beat
            let ticks_per_second = fps.as_f32() * subdivisions as f32;
            (ticks_per_second * 0.5) as u16 // assume 120bpm
        }
    };

    // Find tempo from track 0 (or use override)
    let mut tempo_us: u32 = 500_000; // 120 BPM default
    let mut time_sig_num: u8 = 4;
    let mut time_sig_den: u8 = 4;

    // First pass: collect tempo and time signature from all tracks
    for track in &smf.tracks {
        let mut tick: u32 = 0;
        for event in track {
            tick += event.delta.as_int();
            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => {
                    tempo_us = t.as_int();
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den, _, _)) => {
                    time_sig_num = num;
                    time_sig_den = 1u8 << den;
                }
                _ => {}
            }
        }
    }

    let tempo_bpm = if let Some(bpm) = override_bpm {
        bpm
    } else {
        60_000_000.0 / tempo_us as f64
    };

    let ms_per_tick = (60_000.0 / tempo_bpm) / ticks_per_beat as f64;

    // Second pass: collect all notes
    let mut notes: Vec<MidiNote> = Vec::new();
    // Map channel+pitch -> (start_tick, velocity)
    let mut active_notes: HashMap<(u8, u8), (u32, u8)> = HashMap::new();

    let mut max_tick: u32 = 0;

    for (track_idx, track) in smf.tracks.iter().enumerate() {
        let mut tick: u32 = 0;
        active_notes.clear();

        for event in track {
            tick += event.delta.as_int();
            max_tick = max_tick.max(tick);

            match event.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            let pitch = key.as_int();
                            let velocity = vel.as_int();
                            if velocity > 0 {
                                active_notes.insert((ch, pitch), (tick, velocity));
                            } else {
                                // Velocity 0 = note off
                                if let Some((start_tick, vel)) = active_notes.remove(&(ch, pitch)) {
                                    notes.push(MidiNote {
                                        start_tick,
                                        end_tick: tick,
                                        start_ms: start_tick as f64 * ms_per_tick,
                                        end_ms: tick as f64 * ms_per_tick,
                                        pitch,
                                        velocity: vel,
                                        channel: ch,
                                    });
                                }
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            let pitch = key.as_int();
                            if let Some((start_tick, vel)) = active_notes.remove(&(ch, pitch)) {
                                notes.push(MidiNote {
                                    start_tick,
                                    end_tick: tick,
                                    start_ms: start_tick as f64 * ms_per_tick,
                                    end_ms: tick as f64 * ms_per_tick,
                                    pitch,
                                    velocity: vel,
                                    channel: ch,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Close any still-active notes at end of track
        for ((ch, pitch), (start_tick, vel)) in active_notes.drain() {
            notes.push(MidiNote {
                start_tick,
                end_tick: tick,
                start_ms: start_tick as f64 * ms_per_tick,
                end_ms: tick as f64 * ms_per_tick,
                pitch,
                velocity: vel,
                channel: ch,
            });
        }
    }

    // Sort by start time
    notes.sort_by(|a, b| a.start_ms.partial_cmp(&b.start_ms).unwrap());

    let duration_ms = max_tick as f64 * ms_per_tick;

    Ok(MidiData {
        notes,
       ticks_per_beat,
       tempo_bpm,
       time_sig_num,
       time_sig_den,
       duration_ms,
       total_ticks: max_tick,
    })
}
