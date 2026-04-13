use libvoice::{AnalysisOutputOptions, AnalysisReport, AnalyzerConfig, VoiceAnalyzer};
use std::io::Cursor;
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tempfile::NamedTempFile;

#[derive(Clone, Debug)]
pub struct DecodedAudio {
    pub backend: &'static str,
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

pub fn decode_audio_bytes(bytes: &[u8], file_name: Option<&str>) -> Result<DecodedAudio, String> {
    match decode_audio_bytes_with_symphonia(bytes, file_name) {
        Ok(decoded) => Ok(decoded),
        Err(primary_error) => decode_audio_bytes_with_ffmpeg(bytes).map_err(|fallback_error| {
            format!("{primary_error}; ffmpeg fallback failed: {fallback_error}")
        }),
    }
}

pub fn analyze_samples(decoded: &DecodedAudio, include_fft_spectrum: bool) -> AnalysisReport {
    let config = AnalyzerConfig::new(decoded.sample_rate);
    VoiceAnalyzer::analyze_buffer_with_output_options(
        config,
        &decoded.samples,
        AnalysisOutputOptions {
            fft_spectrum: include_fft_spectrum,
        },
    )
}

pub fn audio_duration_seconds(decoded: &DecodedAudio) -> f32 {
    decoded.samples.len() as f32 / decoded.sample_rate as f32
}

pub fn clip_audio_seconds(
    decoded: &DecodedAudio,
    from_seconds: f32,
    to_seconds: f32,
) -> Result<DecodedAudio, String> {
    let sample_rate = decoded.sample_rate as f32;
    let start_sample = (from_seconds * sample_rate).floor() as usize;
    let end_sample = (to_seconds * sample_rate).ceil() as usize;

    if start_sample >= end_sample || end_sample > decoded.samples.len() {
        return Err("invalid audio clip range".to_string());
    }

    Ok(DecodedAudio {
        backend: decoded.backend,
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        samples: decoded.samples[start_sample..end_sample].to_vec(),
    })
}

fn decode_audio_bytes_with_symphonia(
    bytes: &[u8],
    file_name: Option<&str>,
) -> Result<DecodedAudio, String> {
    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|ext| ext.to_str())
    {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("failed to probe audio format: {error}"))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "no default audio track found".to_string())?;
    let codec_params = track.codec_params.clone();

    if codec_params.codec == CODEC_TYPE_NULL {
        return Err("unsupported or missing codec parameters".to_string());
    }

    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| "missing sample rate in codec parameters".to_string())?;
    let channel_count = codec_params
        .channels
        .map(|channels| channels.count())
        .ok_or_else(|| "missing channel layout in codec parameters".to_string())?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|error| format!("failed to create decoder: {error}"))?;

    let mut mono_scratch = Vec::new();
    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err("decoder reset required; chained streams are not supported".to_string());
            }
            Err(error) => return Err(format!("failed while reading packets: {error}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("decode failed: {error}")),
        };

        let mono = fold_to_mono(decoded, channel_count, &mut mono_scratch);
        samples.extend_from_slice(mono);
    }

    Ok(DecodedAudio {
        backend: "symphonia",
        sample_rate,
        channels: channel_count,
        samples,
    })
}

fn decode_audio_bytes_with_ffmpeg(bytes: &[u8]) -> Result<DecodedAudio, String> {
    let mut temp_input =
        NamedTempFile::new().map_err(|error| format!("failed to create temp file: {error}"))?;
    temp_input
        .write_all(bytes)
        .map_err(|error| format!("failed to write temp audio file: {error}"))?;
    temp_input
        .flush()
        .map_err(|error| format!("failed to flush temp audio file: {error}"))?;
    let sample_rate = probe_sample_rate_with_ffprobe(temp_input.path())?;

    let output = std::process::Command::new("ffmpeg")
        .arg("-nostdin")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(temp_input.path())
        .arg("-f")
        .arg("f32le")
        .arg("-ac")
        .arg("1")
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("failed to wait for ffmpeg: {error}"))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("ffmpeg exited with {}", output.status)
        } else {
            detail
        });
    }

    if output.stdout.len() % std::mem::size_of::<f32>() != 0 {
        return Err("ffmpeg output length was not aligned to f32 samples".to_string());
    }

    let samples = output
        .stdout
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    Ok(DecodedAudio {
        backend: "ffmpeg",
        sample_rate,
        channels: 1,
        samples,
    })
}

fn probe_sample_rate_with_ffprobe(path: &Path) -> Result<u32, String> {
    let output = std::process::Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("a:0")
        .arg("-show_entries")
        .arg("stream=sample_rate")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("failed to run ffprobe: {error}"))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("ffprobe exited with {}", output.status)
        } else {
            detail
        });
    }

    let sample_rate = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("failed to parse ffprobe sample rate: {error}"))?;
    if sample_rate == 0 {
        return Err("ffprobe reported a zero sample rate".to_string());
    }

    Ok(sample_rate)
}

fn fold_to_mono<'a>(
    decoded: AudioBufferRef<'_>,
    channel_count: usize,
    mono: &'a mut Vec<f32>,
) -> &'a [f32] {
    let channels = channel_count.max(1);
    match decoded {
        AudioBufferRef::F32(buffer) => {
            average_channels_into(mono, buffer.chan(0).len(), channels, |idx, ch| {
                *buffer.chan(ch).get(idx).unwrap_or(&0.0)
            });
            mono.as_slice()
        }
        other => {
            let mut sample_buffer =
                SampleBuffer::<f32>::new(other.capacity() as u64, *other.spec());
            sample_buffer.copy_interleaved_ref(other);
            average_interleaved_channels_into(mono, sample_buffer.samples(), channels);
            mono.as_slice()
        }
    }
}

fn average_channels_into<F>(
    mono: &mut Vec<f32>,
    frames: usize,
    channel_count: usize,
    mut sample_at: F,
) where
    F: FnMut(usize, usize) -> f32,
{
    mono.clear();
    mono.reserve(frames.saturating_sub(mono.capacity()));
    for index in 0..frames {
        let mut sum = 0.0;
        for channel in 0..channel_count {
            sum += sample_at(index, channel);
        }
        mono.push(sum / channel_count as f32);
    }
}

fn average_interleaved_channels_into(mono: &mut Vec<f32>, samples: &[f32], channel_count: usize) {
    mono.clear();
    mono.reserve(
        samples
            .len()
            .saturating_div(channel_count)
            .saturating_sub(mono.capacity()),
    );
    for frame in samples.chunks_exact(channel_count) {
        let mut sum = 0.0;
        for sample in frame {
            sum += sample;
        }
        mono.push(sum / channel_count as f32);
    }
}
