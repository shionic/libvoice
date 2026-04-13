use clap::{Parser, ValueEnum};
use libvoice::{
    AnalysisOutputOptions, AnalysisReport, AnalyzerConfig, FrameAnalysis, HarmonicSummary,
    SpectralSummary, SummaryStats, VoiceAnalyzer,
};
use rayon::prelude::*;
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use voiceanalysis::{
    GraphImage, audio_duration_seconds, build_spectrum_feature_graphs, decode_audio_bytes,
    generate_graphs,
};

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

    #[arg(long, default_value_t = false)]
    high_pitch_mode: bool,

    #[arg(long, default_value_t = 0.85)]
    rolloff_ratio: f32,

    #[arg(long)]
    frame_from: Option<usize>,

    #[arg(long)]
    frame_to: Option<usize>,

    #[arg(long)]
    graph_dir: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    spectrum_graphs: bool,
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
    graph_paths: Vec<PathBuf>,
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
                                "graph_paths": output.graph_paths,
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
                        "graph_paths": output.graph_paths,
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
                                "graph_paths": output.graph_paths,
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
                        "graph_paths": output.graph_paths,
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
    if args.spectrum_graphs && args.graph_dir.is_none() {
        return Err("--spectrum-graphs requires --graph-dir".to_string());
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
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let decoded = decode_audio_bytes(&bytes, path.file_name().and_then(|name| name.to_str()))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let report = VoiceAnalyzer::analyze_buffer_with_output_options(
        build_config(decoded.sample_rate, args),
        &decoded.samples,
        AnalysisOutputOptions {
            frame_analysis: requires_frame_analysis(args),
            fft_spectrum: args.spectrum_graphs,
        },
    );
    let voiced_frames = report.frames.clone();
    let duration_seconds = audio_duration_seconds(&decoded);
    let graph_paths = if args.graph_dir.is_some() {
        write_graphs(path, &report, args)?
    } else {
        Vec::new()
    };

    Ok(FileAnalysisOutput {
        path: path.to_path_buf(),
        backend: decoded.backend,
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        duration_seconds,
        report,
        voiced_intervals: merge_voiced_intervals(&voiced_frames),
        voiced_frames,
        graph_paths,
    })
}

fn requires_frame_analysis(args: &Args) -> bool {
    if args.graph_dir.is_some() {
        return true;
    }

    match args.format {
        OutputFormat::Text => args.frame_from.is_some() || args.frame_to.is_some(),
        OutputFormat::Json | OutputFormat::FramesJson | OutputFormat::VoicedIntervalsJson => true,
    }
}

fn write_graphs(path: &Path, report: &AnalysisReport, args: &Args) -> Result<Vec<PathBuf>, String> {
    let root = args
        .graph_dir
        .as_ref()
        .ok_or_else(|| "graph output directory was not configured".to_string())?;
    let output_dir = root.join(graph_output_dir_name(path));
    std::fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create graph output directory {}: {error}",
            output_dir.display()
        )
    })?;

    let mut graphs = generate_graphs(report)?;
    if args.spectrum_graphs {
        graphs.extend(build_spectrum_feature_graphs(report)?);
    }

    let mut written = Vec::with_capacity(graphs.len());
    for graph in graphs {
        let output_path = output_dir.join(&graph.file_name);
        write_graph(&output_path, graph)?;
        written.push(output_path);
    }

    Ok(written)
}

fn write_graph(path: &Path, graph: GraphImage) -> Result<(), String> {
    std::fs::write(path, &graph.png_bytes)
        .map_err(|error| format!("failed to write graph {}: {error}", path.display()))
}

fn graph_output_dir_name(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut out = String::with_capacity(raw.len());

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        "input".to_string()
    } else {
        out
    }
}

fn build_config(sample_rate: u32, args: &Args) -> AnalyzerConfig {
    let mut config = AnalyzerConfig::new(sample_rate);
    if args.high_pitch_mode {
        config.apply_high_pitch_mode();
    }
    config.frame_size = args.frame_size;
    config.hop_size = args.hop_size;
    config.min_pitch_hz = args.min_pitch_hz;
    config.max_pitch_hz = args.max_pitch_hz;
    config.pitch_clarity_threshold = args.pitch_clarity_threshold;
    config.rolloff_ratio = args.rolloff_ratio;
    config
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
    format_optional_harmonics(&mut out, overall.harmonics.as_ref());
    format_frame_slice(
        &mut out,
        &output.voiced_frames,
        args.frame_from,
        args.frame_to,
    );
    if !output.graph_paths.is_empty() {
        writeln!(&mut out, "Graphs:").unwrap();
        for path in &output.graph_paths {
            writeln!(&mut out, "  {}", path.display()).unwrap();
        }
    }
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

        if !frame.harmonic_strengths.is_empty() {
            writeln!(
                out,
                "  Harmonic strengths (F0=1): {}",
                format_optional_series(&frame.harmonic_strengths)
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

fn format_optional_series(values: &[Option<f32>]) -> String {
    values
        .iter()
        .map(|value| value.map(format_value).unwrap_or_else(|| "-".to_string()))
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

fn format_optional_harmonics(out: &mut String, harmonics: Option<&HarmonicSummary>) {
    match harmonics {
        Some(harmonics) => {
            writeln!(
                out,
                "Harmonics: normalized to F0, max frequency {} Hz",
                format_value(harmonics.max_frequency_hz)
            )
            .unwrap();
            for harmonic in &harmonics.harmonics {
                writeln!(
                    out,
                    "  H{} strength ratio: mean {}, std {}, p5 {}, p95 {}",
                    harmonic.harmonic_number,
                    format_value(harmonic.strength_ratio.mean),
                    format_value(harmonic.strength_ratio.std),
                    format_value(harmonic.strength_ratio.p5),
                    format_value(harmonic.strength_ratio.p95)
                )
                .unwrap();
            }
        }
        None => {
            writeln!(out, "Harmonics: n/a").unwrap();
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
