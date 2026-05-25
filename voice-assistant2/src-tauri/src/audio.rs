use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, StreamConfig};
use crossbeam_channel::{bounded, Receiver};
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use anyhow::Result;

use crate::pitch::{PitchDetector, FFT_SIZE, HOP_SIZE, SAMPLE_RATE};
use crate::state::PitchFrame;

fn resolve_audio_path(path: &str) -> Result<PathBuf> {
    // Handle paths coming from Tauri file dialog (absolute file path)
    if let Some(stripped) = path.strip_prefix("file://") {
        let decoded = url::Url::parse(&format!("file://{stripped}"))
            .or_else(|_| url::Url::parse(path))?;
        let file_path = decoded
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("Invalid file URL: {path}"))?;
        return Ok(file_path);
    }

    Ok(PathBuf::from(path))
}

/// Audio samples ring buffer
struct RingBuffer {
    data: Vec<f32>,
    write_pos: usize,
    count: usize,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            write_pos: 0,
            count: 0,
        }
    }

    fn push(&mut self, sample: f32) {
        self.data[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) % self.data.len();
        self.count = self.count.min(self.data.len() - 1) + 1;
    }

    fn push_slice(&mut self, samples: &[f32]) {
        for &s in samples {
            self.push(s);
        }
    }

    /// Read the most recent `n` samples in chronological order
    fn read_recent(&self, n: usize) -> Vec<f32> {
        let n = n.min(self.count);
        let mut result = vec![0.0f32; n];
        let cap = self.data.len();
        let start = (self.write_pos + cap - n) % cap;
        for i in 0..n {
            result[i] = self.data[(start + i) % cap];
        }
        result
    }
}

pub fn get_audio_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
    .map(|devices| {
        devices
        .filter_map(|d| d.name().ok())
        .collect()
    })
    .unwrap_or_default()
}

pub fn start_capture(
    app: AppHandle,
    state: Arc<parking_lot::Mutex<crate::state::AppStateInner>>,
    stop_rx: Receiver<()>,
) -> Result<()> {
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            log::error!("No input device available");
            return;
        };

        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        log::info!("Using audio input device: {}", device_name);

        // Query supported configurations
        let supported_configs = match device.supported_input_configs() {
            Ok(configs) => configs,
            Err(e) => {
                log::error!("Failed to query supported input configs: {}", e);
                return;
            }
        };

        // Try to find a config that matches our requirements or is close
        let mut best_config: Option<cpal::SupportedStreamConfigRange> = None;
        
        for supported in supported_configs {
            // Prefer mono, but stereo is acceptable
            if supported.channels() <= 2 {
                // Check if our desired sample rate is supported
                if supported.min_sample_rate().0 <= SAMPLE_RATE 
                    && supported.max_sample_rate().0 >= SAMPLE_RATE {
                    best_config = Some(supported);
                    break;
                }
                // Fallback: keep any config we find
                if best_config.is_none() {
                    best_config = Some(supported);
                }
            }
        }

        let Some(supported_config) = best_config else {
            log::error!("No suitable audio input configuration found");
            return;
        };

        // Build config from supported range
        let sample_rate = if supported_config.min_sample_rate().0 <= SAMPLE_RATE 
            && supported_config.max_sample_rate().0 >= SAMPLE_RATE {
            SampleRate(SAMPLE_RATE)
        } else {
            log::warn!("Desired sample rate {} not supported, using {}", 
                SAMPLE_RATE, supported_config.default_sample_rate().0);
            supported_config.default_sample_rate()
        };

        let channels = supported_config.channels();
        log::info!("Audio config: {} Hz, {} channel(s)", sample_rate.0, channels);

        let config = StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default, // Let the system choose
        };

        let ring = Arc::new(Mutex::new(RingBuffer::new(FFT_SIZE * 4)));
        let ring_writer = ring.clone();

        // Channel to send audio chunks from callback to processing thread
        let (chunk_tx, chunk_rx) = bounded::<Vec<f32>>(64);

        let stream = match device.build_input_stream(
            &config,
            move |data: &[f32], _: &_| {
                // If stereo, convert to mono by averaging channels
                if channels == 2 {
                    let mono: Vec<f32> = data.chunks_exact(2)
                        .map(|frame| (frame[0] + frame[1]) / 2.0)
                        .collect();
                    ring_writer.lock().push_slice(&mono);
                    let _ = chunk_tx.try_send(mono);
                } else {
                    ring_writer.lock().push_slice(data);
                    let _ = chunk_tx.try_send(data.to_vec());
                }
            },
            |err| log::error!("Audio stream error: {}", err),
            None,
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Could not build input stream: {}", e);
                return;
            }
        };

        if let Err(e) = stream.play() {
            log::error!("Could not start input stream: {}", e);
            return;
        }

        let mut detector = PitchDetector::new();
        let mut hop_counter = 0usize;
        let start_time = std::time::Instant::now();

        loop {
            // Check stop signal
            if stop_rx.try_recv().is_ok() {
                break;
            }

            // Process incoming chunks
            match chunk_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(_chunk) => {
                    hop_counter += 1;

                    // Only run FFT every N hops for performance
                    if hop_counter % 2 == 0 {
                        let samples = ring.lock().read_recent(FFT_SIZE);
                        if samples.len() == FFT_SIZE {
                            let timestamp_ms = start_time.elapsed().as_secs_f64() * 1000.0;

                            if let Some(result) = detector.process(&samples) {
                                let frame = PitchFrame {
                                    timestamp_ms,
                                    frequency: result.frequency,
                                    confidence: result.confidence,
                                    note: Some(result.note_name.clone()),
                       cents_deviation: result.cents_deviation,
                       midi_note: Some(result.midi_note),
                                };

                                // Store in history
                                state.lock().pitch_history.push(frame.clone());

                                // Emit to frontend
                                let _ = app.emit("pitch-frame", &frame);
                            } else {
                                // Emit silence/no-pitch frame
                                let frame = PitchFrame {
                                    timestamp_ms,
                                    frequency: 0.0,
                                    confidence: 0.0,
                                    note: None,
                                    cents_deviation: 0.0,
                                    midi_note: None,
                                };
                                let _ = app.emit("pitch-frame", &frame);
                            }
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                       Err(_) => break,
            }
        }

        // Keep stream alive until we exit
        drop(stream);
    });

    Ok(())
}

pub fn start_playback_audio(
    app: AppHandle,
    path: String,
    bpm: f64,
    ticks_per_beat: u16,
    stop_rx: Receiver<()>,
) -> Result<()> {
    use rodio::{Decoder, OutputStream, Sink};
    use std::fs::File;
    use std::io::BufReader;

    std::thread::spawn(move || {
        let resolved_path = match resolve_audio_path(&path) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Could not resolve audio path '{path}': {e}");
                return;
            }
        };

        let (_stream, stream_handle) = match OutputStream::try_default() {
            Ok(r) => r,
                       Err(e) => {
                           log::error!("Audio output error: {}", e);
                           return;
                       }
        };

        let file = match File::open(&resolved_path) {
            Ok(f) => f,
                       Err(e) => {
                           log::error!("Could not open audio file '{}': {}", resolved_path.display(), e);
                           return;
                       }
        };

        // Check file size before attempting to decode
        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(e) => {
                log::error!("Could not read file metadata for '{}': {}", resolved_path.display(), e);
                return;
            }
        };

        if metadata.len() == 0 {
            log::error!("Audio file '{}' is empty (0 bytes)", resolved_path.display());
            return;
        }

        log::info!("Loading audio file '{}' ({} bytes)", resolved_path.display(), metadata.len());

        let buf_reader = BufReader::new(file);
        let decoder = match Decoder::new(buf_reader) {
            Ok(d) => d,
                       Err(e) => {
                           log::error!("Could not decode audio file '{}': {}. Supported formats: MP3, WAV, Vorbis/OGG. File size: {} bytes", 
                               resolved_path.display(), e, metadata.len());
                           return;
                       }
        };

        let sink = match Sink::try_new(&stream_handle) {
            Ok(s) => s,
                       Err(e) => {
                           log::error!("Sink error: {}", e);
                           return;
                       }
        };

        sink.append(decoder);

        let _ = app.emit("playback-started", ());

        // Tick loop: emit bar/beat events
        let tick_duration_ms = 60_000.0 / bpm;
        let start = std::time::Instant::now();
        let mut last_beat = -1i64;

        loop {
            if stop_rx.try_recv().is_ok() || sink.empty() {
                sink.stop();
                let _ = app.emit("playback-stopped", ());
                break;
            }

            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let beat = (elapsed_ms / tick_duration_ms).floor() as i64;

            if beat != last_beat {
                last_beat = beat;
                let _ = app.emit("beat", serde_json::json!({
                    "beat": beat,
                    "elapsed_ms": elapsed_ms,
                }));
            }

            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });

    Ok(())
}
