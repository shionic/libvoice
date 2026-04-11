use clap::{Parser, ValueEnum};
use libvoice::{
    AnalysisReport, AnalyzerConfig, FormantSummary, FrameAnalysis, SpectralSummary, SummaryStats,
    VoiceAnalyzer,
};
use rayon::prelude::*;
use serde::Serialize;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
    FramesJson,
    VoicedIntervalsJson,
}

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Analyze one or more audio files with libvoice"
)]
struct Args {
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    #[arg(long)]
    threads: Option<usize>,

    #[arg(long, default_value_t = 2048)]
    frame_size: usize,

    #[arg(long, default_value_t = 512)]
    hop_size: usize,

    #[arg(long, default_value_t = 60.0)]
    min_pitch_hz: f32,

    #[arg(long, default_value_t = 500.0)]
    max_pitch_hz: f32,

    #[arg(long, default_value_t = 0.60)]
    pitch_clarity_threshold: f32,

    #[arg(long, default_value_t = 0.85)]
    rolloff_ratio: f32,

    #[arg(long)]
    frame_from: Option<usize>,

    #[arg(long)]
    frame_to: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct FileAnalysisOutput {
    path: PathBuf,
    backend: &'static str,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
    report: AnalysisReport,
    voiced_frames: Vec<FrameAnalysis>,
    voiced_intervals: Vec<VoicedInterval>,
}

#[derive(Debug, Clone, Serialize)]
struct VoicedInterval {
    start_seconds: f32,
    end_seconds: f32,
    frame_count: usize,
}

fn main() {
    let args = Args::parse();

    if let Err(error) = validate_args(&args) {
        exit_with_error(error);
    }

    if let Err(error) = configure_thread_pool(args.threads) {
        exit_with_error(error);
    }

    let results: Vec<_> = args
        .inputs
        .par_iter()
        .map(|path| analyze_file(path, &args))
        .collect();

    let mut outputs = Vec::new();
    let mut failures = Vec::new();

    for result in results {
        match result {
            Ok(output) => outputs.push(output),
            Err(error) => failures.push(error),
        }
    }

    let filtered_outputs: Vec<_> = outputs
        .iter()
        .map(|output| filter_output_frames(output, &args))
        .collect();

    match args.format {
        OutputFormat::Text => {
            for (index, output) in filtered_outputs.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                print!("{}", format_text_report(output, &args));
            }
        }
        OutputFormat::Json => {
            let json = if failures.is_empty() {
                serde_json::to_string_pretty(&filtered_outputs)
            } else {
                serde_json::to_string_pretty(&serde_json::json!({
                    "files": filtered_outputs,
                    "errors": failures,
                }))
            };

            match json {
                Ok(value) => println!("{value}"),
                Err(error) => exit_with_error(format!("failed to serialize output: {error}")),
            }
        }
        OutputFormat::FramesJson => {
            let payload = if failures.is_empty() {
                serde_json::to_string_pretty(
                    &filtered_outputs
                        .iter()
                        .map(|output| {
                            serde_json::json!({
                                "path": output.path,
                                "backend": output.backend,
                                "sample_rate": output.sample_rate,
                                "channels": output.channels,
                                "duration_seconds": output.duration_seconds,
                                "voiced_frames": output.voiced_frames,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                serde_json::to_string_pretty(&serde_json::json!({
                    "files": filtered_outputs.iter().map(|output| serde_json::json!({
                        "path": output.path,
                        "backend": output.backend,
                        "sample_rate": output.sample_rate,
                        "channels": output.channels,
                        "duration_seconds": output.duration_seconds,
                        "voiced_frames": output.voiced_frames,
                    })).collect::<Vec<_>>(),
                    "errors": failures,
                }))
            };

            match payload {
                Ok(value) => println!("{value}"),
                Err(error) => exit_with_error(format!("failed to serialize frame output: {error}")),
            }
        }
        OutputFormat::VoicedIntervalsJson => {
            let payload = if failures.is_empty() {
                serde_json::to_string_pretty(
                    &outputs
                        .iter()
                        .map(|output| {
                            serde_json::json!({
                                "path": output.path,
                                "backend": output.backend,
                                "sample_rate": output.sample_rate,
                                "channels": output.channels,
                                "duration_seconds": output.duration_seconds,
                                "voiced_intervals": output.voiced_intervals,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                serde_json::to_string_pretty(&serde_json::json!({
                    "files": outputs.iter().map(|output| serde_json::json!({
                        "path": output.path,
                        "backend": output.backend,
                        "sample_rate": output.sample_rate,
                        "channels": output.channels,
                        "duration_seconds": output.duration_seconds,
                        "voiced_intervals": output.voiced_intervals,
                    })).collect::<Vec<_>>(),
                    "errors": failures,
                }))
            };

            match payload {
                Ok(value) => println!("{value}"),
                Err(error) => {
                    exit_with_error(format!("failed to serialize interval output: {error}"))
                }
            }
        }
    }

    if !failures.is_empty() {
        if matches!(args.format, OutputFormat::Text) {
            eprintln!("Errors:");
            for failure in &failures {
                eprintln!("  - {failure}");
            }
        }
        std::process::exit(1);
    }
}

fn validate_args(args: &Args) -> Result<(), String> {
    if let (Some(frame_from), Some(frame_to)) = (args.frame_from, args.frame_to) {
        if frame_from != 0 && frame_to != 0 && frame_from > frame_to {
            return Err("--frame-from must be less than or equal to --frame-to".to_string());
        }
    }

    Ok(())
}

fn filter_output_frames(output: &FileAnalysisOutput, args: &Args) -> FileAnalysisOutput {
    let mut filtered = output.clone();
    let frames = select_frames(&output.voiced_frames, args.frame_from, args.frame_to)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();

    filtered.report.frames = frames.clone();
    filtered.voiced_intervals = merge_voiced_intervals(&frames);
    filtered.voiced_frames = frames;
    filtered
}

fn select_frames<'a>(
    frames: &'a [FrameAnalysis],
    frame_from: Option<usize>,
    frame_to: Option<usize>,
) -> Vec<&'a FrameAnalysis> {
    let (start, end) = resolved_frame_range(frame_from, frame_to);
    frames
        .iter()
        .filter(|frame| frame.frame_index >= start && frame.frame_index <= end)
        .collect()
}

fn resolved_frame_range(frame_from: Option<usize>, frame_to: Option<usize>) -> (usize, usize) {
    let start = match frame_from {
        Some(0) | None => 0,
        Some(value) => value,
    };
    let end = match frame_to {
        Some(0) | None => usize::MAX,
        Some(value) => value,
    };
    (start, end)
}

fn configure_thread_pool(threads: Option<usize>) -> Result<(), String> {
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(count) = threads {
        if count == 0 {
            return Err("--threads must be greater than 0".to_string());
        }
        builder = builder.num_threads(count);
    }

    builder
        .build_global()
        .map_err(|error| format!("failed to configure rayon thread pool: {error}"))
}

fn analyze_file(path: &Path, args: &Args) -> Result<FileAnalysisOutput, String> {
    match analyze_file_with_symphonia(path, args) {
        Ok(output) => Ok(output),
        Err(primary_error) => analyze_file_with_ffmpeg(path, args).map_err(|fallback_error| {
            format!("{primary_error}; ffmpeg fallback failed: {fallback_error}")
        }),
    }
}

fn analyze_file_with_symphonia(path: &Path, args: &Args) -> Result<FileAnalysisOutput, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("{}: failed to probe audio format: {error}", path.display()))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| format!("{}: no default audio track found", path.display()))?;
    let codec_params = track.codec_params.clone();

    if codec_params.codec == CODEC_TYPE_NULL {
        return Err(format!(
            "{}: unsupported or missing codec parameters",
            path.display()
        ));
    }

    let sample_rate = codec_params.sample_rate.ok_or_else(|| {
        format!(
            "{}: missing sample rate in codec parameters",
            path.display()
        )
    })?;
    let channel_count = codec_params
        .channels
        .map(|channels| channels.count())
        .ok_or_else(|| {
            format!(
                "{}: missing channel layout in codec parameters",
                path.display()
            )
        })?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|error| format!("{}: failed to create decoder: {error}", path.display()))?;

    let mut analyzer = VoiceAnalyzer::new(build_config(sample_rate, args));
    let mut mono_scratch = Vec::new();
    let mut chunks = Vec::new();
    let mut voiced_frames = Vec::new();
    let mut processed_samples = 0usize;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err(format!(
                    "{}: decoder reset required; chained streams are not supported",
                    path.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "{}: failed while reading packets: {error}",
                    path.display()
                ));
            }
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
            Err(error) => {
                return Err(format!("{}: decode failed: {error}", path.display()));
            }
        };

        let mono = fold_to_mono(decoded, channel_count, &mut mono_scratch);
        processed_samples += mono.len();
        let (chunk, frames) = analyzer.process_chunk_with_frames(mono);
        chunks.push(chunk);
        voiced_frames.extend(frames);
    }

    let report = AnalysisReport {
        config: analyzer.config().clone(),
        frames: voiced_frames.clone(),
        chunks,
        overall: analyzer.finalize(),
        fft_spectrum: None,
    };
    let duration_seconds = processed_samples as f32 / sample_rate as f32;

    Ok(FileAnalysisOutput {
        path: path.to_path_buf(),
        backend: "symphonia",
        sample_rate,
        channels: channel_count,
        duration_seconds,
        report,
        voiced_intervals: merge_voiced_intervals(&voiced_frames),
        voiced_frames,
    })
}

fn analyze_file_with_ffmpeg(path: &Path, args: &Args) -> Result<FileAnalysisOutput, String> {
    let mut child = Command::new("ffmpeg")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(path)
        .arg("-f")
        .arg("f32le")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: failed to start ffmpeg: {error}", path.display()))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{}: ffmpeg stdout was not captured", path.display()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{}: ffmpeg stderr was not captured", path.display()))?;

    let mut audio_bytes = Vec::new();
    stdout
        .read_to_end(&mut audio_bytes)
        .map_err(|error| format!("{}: failed to read ffmpeg output: {error}", path.display()))?;

    let mut stderr_bytes = Vec::new();
    stderr
        .read_to_end(&mut stderr_bytes)
        .map_err(|error| format!("{}: failed to read ffmpeg stderr: {error}", path.display()))?;

    let status = child
        .wait()
        .map_err(|error| format!("{}: failed to wait for ffmpeg: {error}", path.display()))?;
    if !status.success() {
        let stderr_text = String::from_utf8_lossy(&stderr_bytes);
        let detail = stderr_text.trim();
        return Err(if detail.is_empty() {
            format!("{}: ffmpeg exited with {}", path.display(), status)
        } else {
            format!("{}: {detail}", path.display())
        });
    }

    if audio_bytes.len() % std::mem::size_of::<f32>() != 0 {
        return Err(format!(
            "{}: ffmpeg output length was not aligned to f32 samples",
            path.display()
        ));
    }

    let samples: Vec<f32> = audio_bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    let sample_rate = 16_000;
    let mut analyzer = VoiceAnalyzer::new(build_config(sample_rate, args));
    let (chunk, voiced_frames) = analyzer.process_chunk_with_frames(&samples);
    let report = AnalysisReport {
        config: analyzer.config().clone(),
        frames: voiced_frames.clone(),
        chunks: vec![chunk],
        overall: analyzer.finalize(),
        fft_spectrum: None,
    };
    let duration_seconds = samples.len() as f32 / sample_rate as f32;

    Ok(FileAnalysisOutput {
        path: path.to_path_buf(),
        backend: "ffmpeg",
        sample_rate,
        channels: 1,
        duration_seconds,
        report,
        voiced_intervals: merge_voiced_intervals(&voiced_frames),
        voiced_frames,
    })
}

fn build_config(sample_rate: u32, args: &Args) -> AnalyzerConfig {
    let mut config = AnalyzerConfig::new(sample_rate);
    config.frame_size = args.frame_size;
    config.hop_size = args.hop_size;
    config.min_pitch_hz = args.min_pitch_hz;
    config.max_pitch_hz = args.max_pitch_hz;
    config.pitch_clarity_threshold = args.pitch_clarity_threshold;
    config.rolloff_ratio = args.rolloff_ratio;
    config
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
            let samples = sample_buffer.samples();
            average_interleaved_channels_into(mono, samples, channels);
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
        let mut sum = 0.0_f32;
        for &sample in frame {
            sum += sample;
        }
        mono.push(sum / channel_count as f32);
    }
}

fn format_text_report(output: &FileAnalysisOutput, args: &Args) -> String {
    let mut out = String::new();
    let overall = &output.report.overall;

    writeln!(&mut out, "File: {}", output.path.display()).unwrap();
    writeln!(
        &mut out,
        "Audio: {:.2}s, {} Hz, {} channel(s), {} frame(s), backend {}",
        output.duration_seconds,
        output.sample_rate,
        output.channels,
        overall.frame_count,
        output.backend
    )
    .unwrap();
    writeln!(
        &mut out,
        "Config: frame_size={}, hop_size={}, pitch_range={:.1}-{:.1} Hz",
        output.report.config.frame_size,
        output.report.config.hop_size,
        output.report.config.min_pitch_hz,
        output.report.config.max_pitch_hz
    )
    .unwrap();

    format_optional_stats(&mut out, "Pitch (Hz)", overall.pitch_hz.as_ref());
    format_optional_stats(&mut out, "Energy (mean-square)", overall.energy.as_ref());
    format_optional_spectral(&mut out, overall.spectral.as_ref());
    format_optional_formants(&mut out, overall.formants.as_ref());
    format_frame_slice(
        &mut out,
        &output.voiced_frames,
        args.frame_from,
        args.frame_to,
    );
    out
}

fn format_frame_slice(
    out: &mut String,
    frames: &[FrameAnalysis],
    frame_from: Option<usize>,
    frame_to: Option<usize>,
) {
    if frame_from.is_none() && frame_to.is_none() {
        return;
    }

    writeln!(
        out,
        "Frame range: {}..={}",
        start_display(frame_from),
        end_display(frame_to)
    )
    .unwrap();

    let selected = select_frames(frames, frame_from, frame_to);

    if selected.is_empty() {
        writeln!(out, "Frames: none in requested range").unwrap();
        return;
    }

    for frame in selected {
        writeln!(
            out,
            "Frame {}: {:.3}-{:.3}s pitch {} clarity {} energy {} rms {} loudness_dbfs {} zcr {} rolloff {} centroid {} bandwidth {} flatness {} tilt {} hnr {}",
            frame.frame_index,
            frame.start_seconds,
            frame.end_seconds,
            format_optional_value(frame.pitch_hz),
            format_value(frame.pitch_clarity),
            format_value(frame.energy),
            format_value(frame.rms),
            format_value(frame.loudness_dbfs),
            format_value(frame.zcr),
            format_value(frame.spectral_rolloff_hz),
            format_value(frame.spectral_centroid_hz),
            format_value(frame.spectral_bandwidth_hz),
            format_value(frame.spectral_flatness),
            format_value(frame.spectral_tilt_db_per_octave),
            format_value(frame.hnr_db),
        )
        .unwrap();

        if !frame.formants_hz.is_empty() {
            writeln!(out, "  Formants Hz: {}", format_series(&frame.formants_hz)).unwrap();
        }

        if !frame.formant_bandwidths_hz.is_empty() {
            writeln!(
                out,
                "  Formant bandwidths Hz: {}",
                format_series(&frame.formant_bandwidths_hz)
            )
            .unwrap();
        }
    }
}

fn start_display(start: Option<usize>) -> String {
    if matches!(start, None | Some(0)) {
        "start".to_string()
    } else {
        start.unwrap().to_string()
    }
}

fn end_display(end: Option<usize>) -> String {
    if matches!(end, None | Some(0)) {
        "end".to_string()
    } else {
        end.unwrap().to_string()
    }
}

fn format_optional_value(value: Option<f32>) -> String {
    value.map(format_value).unwrap_or_else(|| "n/a".to_string())
}

fn format_series(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| {
            if *value > 0.0 {
                format_value(*value)
            } else {
                "-".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn merge_voiced_intervals(frames: &[FrameAnalysis]) -> Vec<VoicedInterval> {
    if frames.is_empty() {
        return Vec::new();
    }

    let mut intervals = Vec::new();
    let mut current_start = frames[0].start_seconds;
    let mut current_end = frames[0].end_seconds;
    let mut frame_count = 1usize;

    for frame in frames.iter().skip(1) {
        if frame.start_seconds <= current_end + 1.0e-3 {
            current_end = frame.end_seconds.max(current_end);
            frame_count += 1;
        } else {
            intervals.push(VoicedInterval {
                start_seconds: current_start,
                end_seconds: current_end,
                frame_count,
            });
            current_start = frame.start_seconds;
            current_end = frame.end_seconds;
            frame_count = 1;
        }
    }

    intervals.push(VoicedInterval {
        start_seconds: current_start,
        end_seconds: current_end,
        frame_count,
    });
    intervals
}

fn format_optional_stats(out: &mut String, label: &str, stats: Option<&SummaryStats>) {
    match stats {
        Some(stats) => {
            writeln!(
                out,
                "{}: mean {}, std {}, median {}, min {}, max {}, p5 {}, p95 {}, n {}",
                label,
                format_value(stats.mean),
                format_value(stats.std),
                format_value(stats.median),
                format_value(stats.min),
                format_value(stats.max),
                format_value(stats.p5),
                format_value(stats.p95),
                stats.count
            )
            .unwrap();
        }
        None => {
            writeln!(out, "{}: n/a", label).unwrap();
        }
    }
}

fn format_optional_spectral(out: &mut String, spectral: Option<&SpectralSummary>) {
    match spectral {
        Some(spectral) => {
            writeln!(
                out,
                "Spectral centroid (Hz): mean {}, std {}",
                format_value(spectral.centroid_hz.mean),
                format_value(spectral.centroid_hz.std)
            )
            .unwrap();
            writeln!(
                out,
                "Spectral bandwidth (Hz): mean {}, std {}",
                format_value(spectral.bandwidth_hz.mean),
                format_value(spectral.bandwidth_hz.std)
            )
            .unwrap();
            writeln!(
                out,
                "Spectral rolloff (Hz): mean {}, std {}",
                format_value(spectral.rolloff_hz.mean),
                format_value(spectral.rolloff_hz.std)
            )
            .unwrap();
            writeln!(
                out,
                "Spectral flatness: mean {}, std {}",
                format_value(spectral.flatness.mean),
                format_value(spectral.flatness.std)
            )
            .unwrap();
            writeln!(
                out,
                "Spectral tilt (dB/oct): mean {}, std {}",
                format_value(spectral.tilt_db_per_octave.mean),
                format_value(spectral.tilt_db_per_octave.std)
            )
            .unwrap();
            writeln!(
                out,
                "RMS: mean {}, std {}",
                format_value(spectral.rms.mean),
                format_value(spectral.rms.std)
            )
            .unwrap();
            writeln!(
                out,
                "Loudness (dBFS): mean {}, std {}",
                format_value(spectral.loudness_dbfs.mean),
                format_value(spectral.loudness_dbfs.std)
            )
            .unwrap();
            writeln!(
                out,
                "HNR (dB): mean {}, std {}",
                format_value(spectral.hnr_db.mean),
                format_value(spectral.hnr_db.std)
            )
            .unwrap();
            writeln!(
                out,
                "Zero-crossing rate: mean {}, std {}",
                format_value(spectral.zcr.mean),
                format_value(spectral.zcr.std)
            )
            .unwrap();
        }
        None => {
            writeln!(out, "Spectral: n/a").unwrap();
        }
    }
}

fn format_optional_formants(out: &mut String, formants: Option<&FormantSummary>) {
    match formants {
        Some(formants) => {
            for (label, stats) in [
                ("F1", formants.f1.as_ref()),
                ("F2", formants.f2.as_ref()),
                ("F3", formants.f3.as_ref()),
                ("F4", formants.f4.as_ref()),
            ] {
                match stats {
                    Some(stats) => {
                        writeln!(
                            out,
                            "{label} (Hz): mean {}, std {}; bandwidth mean {}, std {}",
                            format_value(stats.frequency_hz.mean),
                            format_value(stats.frequency_hz.std),
                            format_value(stats.bandwidth_hz.mean),
                            format_value(stats.bandwidth_hz.std)
                        )
                        .unwrap();
                    }
                    None => {
                        writeln!(out, "{label}: n/a").unwrap();
                    }
                }
            }
        }
        None => {
            writeln!(out, "Formants: n/a").unwrap();
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

fn exit_with_error(error: String) -> ! {
    eprintln!("{error}");
    std::process::exit(1);
}
