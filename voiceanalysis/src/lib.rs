mod audio;
mod graphs;
mod options;
mod report;

pub use audio::{
    DecodedAudio, analyze_samples, audio_duration_seconds, clip_audio_seconds, decode_audio_bytes,
};
pub use graphs::{
    GraphImage, build_spectrum_feature_graphs, build_spectrum_graph, generate_graphs,
};
pub use options::{
    AnalyzeOptions, ClipEnd, ClipSpec, ResolvedClip, analyze_usage_hint, parse_analyze_options,
};
pub use report::format_report;

#[derive(Debug)]
pub struct RenderedAnalysis {
    pub report_text: String,
    pub graphs: Vec<GraphImage>,
}

pub fn analyze_audio_bytes(
    bytes: &[u8],
    file_name: Option<&str>,
    label: &str,
    options: &AnalyzeOptions,
) -> Result<RenderedAnalysis, String> {
    let decoded = decode_audio_bytes(bytes, file_name)?;
    let resolved_clip = options
        .clip
        .as_ref()
        .map(|clip| clip.resolve(audio_duration_seconds(&decoded)))
        .transpose()?;
    let analysis_audio = match resolved_clip.as_ref() {
        Some(clip) => clip_audio_seconds(&decoded, clip.from_seconds, clip.to_seconds)?,
        None => decoded,
    };
    let report = analyze_samples(
        &analysis_audio,
        options.graph || options.spectrum,
        options.spectrum,
        options.high_pitch_mode,
    );
    let report_label = format_report_label(label, resolved_clip.as_ref());
    let report_text = format_report(&report_label, &analysis_audio, &report, options);
    let mut graphs = if options.graph {
        generate_graphs(&report)?
    } else {
        Vec::new()
    };
    if options.spectrum {
        graphs.extend(build_spectrum_feature_graphs(&report)?);
    }

    Ok(RenderedAnalysis {
        report_text,
        graphs,
    })
}

fn format_report_label(label: &str, clip: Option<&ResolvedClip>) -> String {
    match clip {
        Some(clip) => format!(
            "{} [{} .. {}]",
            label,
            format_time_seconds(clip.from_seconds),
            format_time_seconds(clip.to_seconds)
        ),
        None => label.to_string(),
    }
}

fn format_time_seconds(seconds: f32) -> String {
    let total_millis = (seconds * 1000.0).round().max(0.0) as u64;
    let total_seconds = total_millis / 1000;
    let minutes = total_seconds / 60;
    let secs = total_seconds % 60;
    let millis = total_millis % 1000;

    if minutes > 0 {
        if millis == 0 {
            format!("{minutes}m{secs:02}s")
        } else {
            format!("{minutes}m{secs:02}.{millis:03}s")
        }
    } else if millis == 0 {
        format!("{secs}s")
    } else {
        format!("{secs}.{millis:03}s")
    }
}
