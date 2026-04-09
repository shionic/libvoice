use libvoice::{
    AnalysisReport, AnalyzerConfig, FormantSummary, SpectralSummary, SummaryStats, VoiceAnalyzer,
};
use std::fmt::Write as _;
use std::io::Cursor;
use std::path::Path;
use std::process::Stdio;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{Audio, Document, FileId, Message};
use tokio::task;

use symphonia::core::audio::{AudioBufferRef, SampleBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Debug)]
struct AnalyzeOptions {
    pitch: bool,
    hnr: bool,
    spectral: bool,
    energy: bool,
    formants: bool,
}

#[derive(Clone, Debug)]
struct InputAudio {
    file_id: FileId,
    file_name: Option<String>,
    label: String,
}

#[derive(Clone, Debug)]
struct DecodedAudio {
    backend: &'static str,
    sample_rate: u32,
    channels: usize,
    samples: Vec<f32>,
}

#[tokio::main]
async fn main() {
    let bot = Bot::from_env();

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        if let Err(error) = handle_message(bot, msg).await {
            eprintln!("{error}");
        }
        respond(())
    })
    .await;
}

async fn handle_message(bot: Bot, msg: Message) -> Result<(), String> {
    let Some(text) = msg.text() else {
        return Ok(());
    };
    if !text.starts_with("/analyze") {
        return Ok(());
    }

    let options = parse_analyze_options(text)?;
    let input = find_input_audio(&msg).ok_or_else(|| {
        "reply to a voice message or audio file with /analyze. defaults: +pitch +hnr +spectral. override with +/-feature, for example /analyze +formants -spectral".to_string()
    })?;

    bot.send_message(
        msg.chat.id,
        format!("Analyzing {}. This can take a few seconds.", input.label),
    )
    .await
    .map_err(|error| format!("failed to send progress message: {error}"))?;

    let telegram_file = bot
        .get_file(input.file_id.clone())
        .await
        .map_err(|error| format!("failed to fetch Telegram file metadata: {error}"))?;

    let mut bytes = Vec::new();
    bot.download_file(&telegram_file.path, &mut bytes)
        .await
        .map_err(|error| format!("failed to download Telegram file: {error}"))?;

    let file_name = input.file_name.clone();
    let report_text = task::spawn_blocking(move || {
        let decoded = decode_audio_bytes(&bytes, file_name.as_deref())?;
        let report = analyze_samples(&decoded);
        Ok::<_, String>(format_report(&input.label, &decoded, &report, &options))
    })
    .await
    .map_err(|error| format!("analysis task failed: {error}"))??;

    send_long_message(&bot, msg.chat.id, &report_text).await
}

fn parse_analyze_options(text: &str) -> Result<AnalyzeOptions, String> {
    let mut options = AnalyzeOptions {
        pitch: true,
        hnr: true,
        spectral: true,
        energy: false,
        formants: false,
    };

    for token in text.split_whitespace().skip(1) {
        if token.is_empty() {
            continue;
        }

        let (enabled, feature) = match token.as_bytes().first().copied() {
            Some(b'+') => (true, &token[1..]),
            Some(b'-') => (false, &token[1..]),
            _ => continue,
        };

        match feature {
            "pitch" => options.pitch = enabled,
            "hnr" => options.hnr = enabled,
            "spectral" => options.spectral = enabled,
            "energy" => options.energy = enabled,
            "formants" => options.formants = enabled,
            "all" => {
                options.pitch = enabled;
                options.hnr = enabled;
                options.spectral = enabled;
                options.energy = enabled;
                options.formants = enabled;
            }
            _ => {
                return Err(format!(
                    "unknown feature `{token}`. supported: +/-pitch, +/-hnr, +/-spectral, +/-energy, +/-formants, +/-all"
                ));
            }
        }
    }

    Ok(options)
}

fn find_input_audio(msg: &Message) -> Option<InputAudio> {
    extract_audio_from_message(msg)
        .or_else(|| msg.reply_to_message().and_then(extract_audio_from_message))
}

fn extract_audio_from_message(msg: &Message) -> Option<InputAudio> {
    if let Some(voice) = msg.voice() {
        return Some(InputAudio {
            file_id: voice.file.id.clone(),
            file_name: Some("voice.ogg".to_string()),
            label: "voice message".to_string(),
        });
    }

    if let Some(audio) = msg.audio() {
        if !is_supported_audio_file(audio) {
            return None;
        }
        return Some(InputAudio {
            file_id: audio.file.id.clone(),
            file_name: audio.file_name.clone(),
            label: audio
                .file_name
                .clone()
                .unwrap_or_else(|| "audio file".to_string()),
        });
    }

    let document = msg.document()?;
    if !is_supported_audio_document(document) {
        return None;
    }

    Some(InputAudio {
        file_id: document.file.id.clone(),
        file_name: document.file_name.clone(),
        label: document
            .file_name
            .clone()
            .unwrap_or_else(|| "audio file".to_string()),
    })
}

fn is_supported_audio_document(document: &Document) -> bool {
    is_audio_name_or_mime(document.file_name.as_deref(), document.mime_type.as_ref())
}

fn is_supported_audio_file(audio: &Audio) -> bool {
    is_audio_name_or_mime(audio.file_name.as_deref(), audio.mime_type.as_ref())
}

fn is_audio_name_or_mime(file_name: Option<&str>, mime_type: Option<&mime::Mime>) -> bool {
    let file_name_ok = file_name
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            [".ogg", ".oga", ".opus", ".wav", ".mp3", ".m4a", ".flac"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        })
        .unwrap_or(false);

    let mime_ok = mime_type
        .map(|mime| mime.type_() == mime::AUDIO || mime.as_ref() == "application/ogg")
        .unwrap_or(false);

    file_name_ok || mime_ok
}

fn decode_audio_bytes(bytes: &[u8], file_name: Option<&str>) -> Result<DecodedAudio, String> {
    match decode_audio_bytes_with_symphonia(bytes, file_name) {
        Ok(decoded) => Ok(decoded),
        Err(primary_error) => decode_audio_bytes_with_ffmpeg(bytes).map_err(|fallback_error| {
            format!("{primary_error}; ffmpeg fallback failed: {fallback_error}")
        }),
    }
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
    let mut child = std::process::Command::new("ffmpeg")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg("pipe:0")
        .arg("-f")
        .arg("f32le")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("pipe:1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start ffmpeg: {error}"))?;

    {
        use std::io::Write as _;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ffmpeg stdin was not captured".to_string())?;
        stdin
            .write_all(bytes)
            .map_err(|error| format!("failed to write input to ffmpeg: {error}"))?;
    }

    let output = child
        .wait_with_output()
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
        sample_rate: SAMPLE_RATE,
        channels: 1,
        samples,
    })
}

fn analyze_samples(decoded: &DecodedAudio) -> AnalysisReport {
    let config = AnalyzerConfig::new(decoded.sample_rate);
    VoiceAnalyzer::analyze_buffer(config, &decoded.samples)
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

fn format_report(
    label: &str,
    decoded: &DecodedAudio,
    report: &AnalysisReport,
    options: &AnalyzeOptions,
) -> String {
    let overall = &report.overall;
    let duration_seconds = decoded.samples.len() as f32 / decoded.sample_rate as f32;
    let mut out = String::new();

    writeln!(&mut out, "Analysis for {label}").unwrap();
    writeln!(
        &mut out,
        "Audio: {:.2}s, {} Hz, {} channel(s), {} voiced frame(s), backend {}",
        duration_seconds,
        decoded.sample_rate,
        decoded.channels,
        overall.frame_count,
        decoded.backend
    )
    .unwrap();
    let mut printed_section = false;

    if options.pitch || options.hnr {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "Pitch & HNR").unwrap();

        if options.pitch {
            match overall.pitch_hz.as_ref() {
                Some(pitch) => {
                    writeln!(
                        &mut out,
                        "Pitch: mean {} Hz, median {} Hz, std {}, p5 {} Hz, p95 {} Hz",
                        format_value(pitch.mean),
                        format_value(pitch.median),
                        format_value(pitch.std),
                        format_value(pitch.p5),
                        format_value(pitch.p95)
                    )
                    .unwrap();
                }
                None => {
                    writeln!(&mut out, "Pitch: not enough voiced frames").unwrap();
                }
            }
        }

        if options.hnr {
            match overall.spectral.as_ref() {
                Some(spectral) => {
                    writeln!(
                        &mut out,
                        "HNR: mean {} dB, std {}",
                        format_value(spectral.hnr_db.mean),
                        format_value(spectral.hnr_db.std)
                    )
                    .unwrap();
                }
                None => {
                    writeln!(&mut out, "HNR: unavailable").unwrap();
                }
            }
        }

        printed_section = true;
    }

    if options.spectral {
        if printed_section {
            writeln!(&mut out).unwrap();
        } else {
            writeln!(&mut out).unwrap();
        }
        writeln!(&mut out, "Spectral").unwrap();
        format_spectral_section(&mut out, overall.spectral.as_ref());
        printed_section = true;
    }

    if options.energy {
        if printed_section {
            writeln!(&mut out).unwrap();
        } else {
            writeln!(&mut out).unwrap();
        }
        writeln!(&mut out, "Energy").unwrap();
        format_optional_stats_line(
            &mut out,
            "Mean-square energy",
            overall.energy.as_ref(),
            None,
        );
        printed_section = true;
    }

    if options.formants {
        if printed_section {
            writeln!(&mut out).unwrap();
        } else {
            writeln!(&mut out).unwrap();
        }
        writeln!(&mut out, "Formants").unwrap();
        format_formants_section(&mut out, overall.formants.as_ref());
        printed_section = true;
    }

    if !printed_section {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "No analysis sections enabled.").unwrap();
    }

    writeln!(&mut out).unwrap();
    writeln!(
        &mut out,
        "Feature toggles: defaults are +pitch +hnr +spectral; available overrides: +/-pitch +/-hnr +/-spectral +/-energy +/-formants +/-all"
    )
    .unwrap();

    out.trim_end().to_string()
}

fn format_spectral_section(out: &mut String, spectral: Option<&SpectralSummary>) {
    let Some(spectral) = spectral else {
        writeln!(out, "Spectral summary: unavailable").unwrap();
        return;
    };

    format_optional_stats_line(out, "Centroid", Some(&spectral.centroid_hz), Some("Hz"));
    format_optional_stats_line(out, "Bandwidth", Some(&spectral.bandwidth_hz), Some("Hz"));
    format_optional_stats_line(out, "Rolloff", Some(&spectral.rolloff_hz), Some("Hz"));
    format_optional_stats_line(out, "Flatness", Some(&spectral.flatness), None);
    format_optional_stats_line(
        out,
        "Tilt",
        Some(&spectral.tilt_db_per_octave),
        Some("dB/oct"),
    );
    format_optional_stats_line(out, "RMS", Some(&spectral.rms), None);
    format_optional_stats_line(out, "Zero-crossing rate", Some(&spectral.zcr), None);
}

fn format_formants_section(out: &mut String, formants: Option<&FormantSummary>) {
    let Some(formants) = formants else {
        writeln!(out, "Formants: unavailable").unwrap();
        return;
    };

    for (label, formant) in [
        ("F1", formants.f1.as_ref()),
        ("F2", formants.f2.as_ref()),
        ("F3", formants.f3.as_ref()),
        ("F4", formants.f4.as_ref()),
    ] {
        match formant {
            Some(formant) => {
                writeln!(
                    out,
                    "{label}: {} Hz (std {}), bandwidth {} Hz (std {})",
                    format_value(formant.frequency_hz.mean),
                    format_value(formant.frequency_hz.std),
                    format_value(formant.bandwidth_hz.mean),
                    format_value(formant.bandwidth_hz.std)
                )
                .unwrap();
            }
            None => {
                writeln!(out, "{label}: unavailable").unwrap();
            }
        }
    }
}

fn format_optional_stats_line(
    out: &mut String,
    label: &str,
    stats: Option<&SummaryStats>,
    unit: Option<&str>,
) {
    let unit_suffix = unit.map(|u| format!(" {u}")).unwrap_or_default();
    match stats {
        Some(stats) => {
            writeln!(
                out,
                "{label}: mean {}{}, std {}, median {}{}, p5 {}{}, p95 {}{}",
                format_value(stats.mean),
                unit_suffix,
                format_value(stats.std),
                format_value(stats.median),
                unit_suffix,
                format_value(stats.p5),
                unit_suffix,
                format_value(stats.p95),
                unit_suffix
            )
            .unwrap();
        }
        None => {
            writeln!(out, "{label}: unavailable").unwrap();
        }
    }
}

fn format_value(value: f32) -> String {
    let abs = value.abs();
    if abs == 0.0 {
        return "0".to_string();
    }
    if abs >= 1000.0 {
        return format!("{value:.2}");
    }
    if abs >= 100.0 {
        return format!("{value:.3}");
    }
    if abs >= 1.0 {
        return format!("{value:.4}");
    }
    if abs >= 0.01 {
        return format!("{value:.6}");
    }
    if abs >= 0.0001 {
        return format!("{value:.8}");
    }
    format!("{value:.3e}")
}

async fn send_long_message(bot: &Bot, chat_id: ChatId, text: &str) -> Result<(), String> {
    const LIMIT: usize = 3500;
    if text.len() <= LIMIT {
        bot.send_message(chat_id, text.to_string())
            .await
            .map_err(|error| format!("failed to send analysis result: {error}"))?;
        return Ok(());
    }

    let mut current = String::new();
    for line in text.lines() {
        if !current.is_empty() && current.len() + line.len() + 1 > LIMIT {
            bot.send_message(chat_id, current.clone())
                .await
                .map_err(|error| format!("failed to send analysis chunk: {error}"))?;
            current.clear();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        bot.send_message(chat_id, current)
            .await
            .map_err(|error| format!("failed to send final analysis chunk: {error}"))?;
    }

    Ok(())
}
