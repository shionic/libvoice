use crate::audio::DecodedAudio;
use crate::options::AnalyzeOptions;
use libvoice::{AnalysisReport, HarmonicSummary, SpectralSummary, SummaryStats};
use std::fmt::Write as _;

pub fn format_report(
    label: &str,
    decoded: &DecodedAudio,
    report: &AnalysisReport,
    options: &AnalyzeOptions,
) -> String {
    let overall = &report.overall;
    let duration_seconds = decoded.samples.len() as f32 / decoded.sample_rate as f32;
    let mut out = String::new();

    writeln!(&mut out, "<b>Voice Analysis</b>").unwrap();
    writeln!(&mut out, "{}", escape_html(label)).unwrap();
    writeln!(
        &mut out,
        "<pre>{}</pre>",
        escape_html(&format!(
            "duration : {}\nrate     : {}\nchannels : {}\nvoiced   : {}\ndecoder  : {}",
            format_duration(duration_seconds),
            format_sample_rate(decoded.sample_rate),
            format_channels(decoded.channels),
            format_voiced_frames(overall.frame_count),
            decoded.backend
        ))
    )
    .unwrap();
    let mut printed_section = false;

    if options.pitch || options.hnr {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "<b>Core</b>").unwrap();
        writeln!(
            &mut out,
            "<pre>{}</pre>",
            escape_html(&format_core_section(
                overall.pitch_hz.as_ref(),
                overall.spectral.as_ref(),
                options
            ))
        )
        .unwrap();
        printed_section = true;
    }

    if options.spectral {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "<b>Spectral</b>").unwrap();
        writeln!(
            &mut out,
            "<pre>{}</pre>",
            escape_html(&format_spectral_section(overall.spectral.as_ref()))
        )
        .unwrap();
        printed_section = true;
    }

    if options.energy {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "<b>Energy</b>").unwrap();
        writeln!(
            &mut out,
            "<pre>{}</pre>",
            escape_html(&format_stats_block(
                "signal energy",
                overall.energy.as_ref(),
                None
            ))
        )
        .unwrap();
        printed_section = true;
    }

    if options.harmonics {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "<b>Harmonics</b>").unwrap();
        writeln!(
            &mut out,
            "<pre>{}</pre>",
            escape_html(&format_harmonics_section(overall.harmonics.as_ref()))
        )
        .unwrap();
        printed_section = true;
    }

    if !printed_section {
        writeln!(&mut out).unwrap();
        writeln!(&mut out, "No sections are enabled for this report.").unwrap();
    }

    writeln!(&mut out).unwrap();
    writeln!(
        &mut out,
        "<i>Tip:</i> <code>+energy</code>, <code>+harmonics</code>, <code>+graph</code>, and <code>+spectrum</code> add more detail. Use <code>-feature</code> to hide a section."
    )
    .unwrap();

    out.trim_end().to_string()
}

fn format_core_section(
    pitch: Option<&SummaryStats>,
    spectral: Option<&SpectralSummary>,
    options: &AnalyzeOptions,
) -> String {
    let mut lines = Vec::new();

    if options.pitch {
        match pitch {
            Some(pitch) => {
                lines.push(format!(
                    "pitch mean   : {} Hz\npitch median : {} Hz\npitch std    : {}\npitch p5..p95: {} .. {} Hz",
                    format_value(pitch.mean),
                    format_value(pitch.median),
                    format_value(pitch.std),
                    format_value(pitch.p5),
                    format_value(pitch.p95)
                ));
            }
            None => lines.push("pitch        : not enough voiced audio".to_string()),
        }
    }

    if options.hnr {
        match spectral {
            Some(spectral) => {
                lines.push(format!(
                    "hnr mean     : {} dB\nhnr std      : {}\nloudness     : {} dBFS",
                    format_value(spectral.hnr_db.mean),
                    format_value(spectral.hnr_db.std),
                    format_value(spectral.loudness_dbfs.mean)
                ));
            }
            None => lines.push("hnr          : unavailable".to_string()),
        }
    }

    lines.join("\n")
}

fn format_spectral_section(spectral: Option<&SpectralSummary>) -> String {
    let Some(spectral) = spectral else {
        return "spectral summary: unavailable".to_string();
    };

    [
        format_stats_block("centroid", Some(&spectral.centroid_hz), Some("Hz")),
        format_stats_block("bandwidth", Some(&spectral.bandwidth_hz), Some("Hz")),
        format_stats_block("rolloff", Some(&spectral.rolloff_hz), Some("Hz")),
        format_stats_block("flatness", Some(&spectral.flatness), None),
        format_stats_block("tilt", Some(&spectral.tilt_db_per_octave), Some("dB/oct")),
        format_stats_block("rms", Some(&spectral.rms), None),
        format_stats_block("loudness", Some(&spectral.loudness_dbfs), Some("dBFS")),
        format_stats_block("zcr", Some(&spectral.zcr), None),
    ]
    .join("\n\n")
}

fn format_harmonics_section(harmonics: Option<&HarmonicSummary>) -> String {
    let Some(harmonics) = harmonics else {
        return "harmonics: unavailable".to_string();
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "normalized  : F0 = 1\nmax range   : {} Hz",
        format_value(harmonics.max_frequency_hz)
    ));
    for harmonic in harmonics.harmonics.iter().take(10) {
        lines.push(format!(
            "h{:02} mean    : {}\nh{:02} std     : {}\nh{:02} p5..p95 : {} .. {}",
            harmonic.harmonic_number,
            format_value(harmonic.strength_ratio.mean),
            harmonic.harmonic_number,
            format_value(harmonic.strength_ratio.std),
            harmonic.harmonic_number,
            format_value(harmonic.strength_ratio.p5),
            format_value(harmonic.strength_ratio.p95)
        ));
    }
    if harmonics.harmonics.len() > 10 {
        lines.push(format!(
            "... {} more harmonics",
            harmonics.harmonics.len() - 10
        ));
    }
    lines.join("\n\n")
}

fn format_stats_block(label: &str, stats: Option<&SummaryStats>, unit: Option<&str>) -> String {
    let unit_suffix = unit.map(|u| format!(" {u}")).unwrap_or_default();
    match stats {
        Some(stats) => format!(
            "{label} mean : {}{}\n{label} std  : {}\n{label} med  : {}{}\n{label} p5   : {}{}\n{label} p95  : {}{}",
            format_value(stats.mean),
            unit_suffix,
            format_value(stats.std),
            format_value(stats.median),
            unit_suffix,
            format_value(stats.p5),
            unit_suffix,
            format_value(stats.p95),
            unit_suffix
        ),
        None => format!("{label} : unavailable"),
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

fn format_duration(seconds: f32) -> String {
    format!("{seconds:.2} s")
}

fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate % 1000 == 0 {
        format!("{} kHz", sample_rate / 1000)
    } else {
        format!("{sample_rate} Hz")
    }
}

fn format_voiced_frames(frame_count: usize) -> String {
    match frame_count {
        1 => "1 voiced frame".to_string(),
        count => format!("{count} voiced frames"),
    }
}

fn format_channels(channels: usize) -> String {
    match channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        count => format!("{count} channels"),
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
