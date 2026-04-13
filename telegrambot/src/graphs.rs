use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use libvoice::{AnalysisReport, FrameAnalysis};
use plotters::coord::types::RangedCoordf32;
use plotters::prelude::*;

const WIDTH: u32 = 2560;
const HEIGHT: u32 = 1440;
type Chart2d<'a, 'b> =
    ChartContext<'a, BitMapBackend<'b>, Cartesian2d<RangedCoordf32, RangedCoordf32>>;

pub struct GraphImage {
    pub file_name: String,
    pub title: String,
    pub png_bytes: Vec<u8>,
}

pub fn generate_graphs(report: &AnalysisReport) -> Result<Vec<GraphImage>, String> {
    let frames = &report.frames;
    if frames.is_empty() {
        return Ok(Vec::new());
    }

    let mut graphs = Vec::new();

    if let Some(graph) = build_pitch_graph(frames)? {
        graphs.push(graph);
    }
    if let Some(graph) = build_harmonics_graph(frames)? {
        graphs.push(graph);
    }
    if let Some(graph) = build_hnr_loudness_graph(frames)? {
        graphs.push(graph);
    }
    if let Some(graph) = build_tilt_graph(frames)? {
        graphs.push(graph);
    }
    if let Some(graph) = build_spectral_graph(frames)? {
        graphs.push(graph);
    }

    Ok(graphs)
}

pub fn build_spectrum_graph(report: &AnalysisReport) -> Result<Option<GraphImage>, String> {
    let Some(spectrum) = report.fft_spectrum.as_ref() else {
        return Ok(None);
    };
    if spectrum.frames.is_empty() || spectrum.bin_hz <= 0.0 {
        return Ok(None);
    };

    let bin_count = spectrum.frames[0].magnitudes.len();
    if bin_count < 8 {
        return Ok(None);
    }

    let nyquist_hz = (bin_count.saturating_sub(1)) as f32 * spectrum.bin_hz;
    let max_hz = nyquist_hz.min(5_000.0);
    let max_bin = (max_hz / spectrum.bin_hz).floor() as usize;
    if max_bin < 8 {
        return Ok(None);
    }

    let mut peak_db = f32::NEG_INFINITY;
    for frame in &spectrum.frames {
        for magnitude in frame.magnitudes.iter().take(max_bin + 1).skip(1) {
            peak_db = peak_db.max(20.0 * magnitude.max(1.0e-12).log10());
        }
    }
    if !peak_db.is_finite() {
        return Ok(None);
    }

    let x_range = spectrum.frames[0].start_seconds
        ..spectrum.frames.last().map(|frame| frame.end_seconds).unwrap_or(0.01);
    let y_range = 0.0_f32..max_hz;

    let mut buffer = vec![255u8; (WIDTH * HEIGHT * 3) as usize];
    let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE).map_err(draw_err)?;

    let mut chart = ChartBuilder::on(&root)
        .margin(24)
        .caption("Voice spectrogram", ("sans-serif", 34))
        .x_label_area_size(56)
        .y_label_area_size(72)
        .build_cartesian_2d(x_range, y_range)
        .map_err(draw_err)?;

    chart
        .configure_mesh()
        .x_desc("Time (s)")
        .y_desc("Frequency (Hz)")
        .light_line_style(RGBColor(220, 220, 220))
        .draw()
        .map_err(draw_err)?;

    for frame in &spectrum.frames {
        let top_bin = max_bin.min(frame.magnitudes.len().saturating_sub(1));
        if top_bin < 1 {
            continue;
        }

        for bin in 1..=top_bin {
            let lower_hz = (bin - 1) as f32 * spectrum.bin_hz;
            let upper_hz = bin as f32 * spectrum.bin_hz;
            let db = 20.0 * frame.magnitudes[bin].max(1.0e-12).log10();
            let normalized = ((db - peak_db + 80.0) / 80.0).clamp(0.0, 1.0);
            let color = spectrogram_color(normalized, frame.is_voiced);
            chart
                .draw_series(std::iter::once(Rectangle::new(
                    [
                        (frame.start_seconds, lower_hz),
                        (frame.end_seconds, upper_hz),
                    ],
                    color.filled(),
                )))
                .map_err(draw_err)?;
        }
    }

    for run in voiced_runs(&report.frames) {
        for segment in segmented_optional_series(
            run.iter().map(|frame| (frame.start_seconds, frame.pitch_hz)),
        ) {
        chart
            .draw_series(LineSeries::new(segment, WHITE.stroke_width(2)))
            .map_err(draw_err)?
            .label("Pitch")
            .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], WHITE.stroke_width(2)));
        }
    }

    chart
        .configure_series_labels()
        .background_style(BLACK.mix(0.55))
        .border_style(WHITE)
        .draw()
        .map_err(draw_err)?;

    drop(chart);
    root.present().map_err(draw_err)?;
    drop(root);

    Ok(Some(GraphImage {
        file_name: "voice_spectrogram.png".to_string(),
        title: "Voice spectrogram".to_string(),
        png_bytes: encode_png(buffer, WIDTH, HEIGHT)?,
    }))
}

pub fn build_spectrum_feature_graphs(report: &AnalysisReport) -> Result<Vec<GraphImage>, String> {
    let Some(spectrum) = report.fft_spectrum.as_ref() else {
        return Ok(Vec::new());
    };
    if spectrum.frames.is_empty() || spectrum.bin_hz <= 0.0 {
        return Ok(Vec::new());
    }

    let bin_count = spectrum.frames[0].magnitudes.len();
    if bin_count < 8 {
        return Ok(Vec::new());
    }

    let nyquist_hz = (bin_count.saturating_sub(1)) as f32 * spectrum.bin_hz;
    let max_hz = nyquist_hz.min(5_000.0);
    let max_bin = (max_hz / spectrum.bin_hz).floor() as usize;
    if max_bin < 8 {
        return Ok(Vec::new());
    }

    let mut graphs = Vec::new();

    if let Some(graph) = build_spectrum_graph(report)? {
        graphs.push(graph);
    }
    if let Some(graph) = build_power_envelope_graph(spectrum, max_bin, max_hz)? {
        graphs.push(graph);
    }
    if let Some(graph) = build_envelope_spectrogram_graph(spectrum, max_bin, max_hz)? {
        graphs.push(graph);
    }

    Ok(graphs)
}

fn build_pitch_graph(frames: &[FrameAnalysis]) -> Result<Option<GraphImage>, String> {
    let values: Vec<f32> = frames.iter().filter_map(|frame| frame.pitch_hz).collect();
    if values.is_empty() {
        return Ok(None);
    }

    let x_range = time_range(frames);
    let y_range = padded_range(&values, 0.1, 20.0);
    let runs = voiced_runs(frames);

    render_graph(
        "Pitch contour",
        "Pitch (Hz)",
        x_range,
        y_range,
        |chart: &mut Chart2d<'_, '_>| {
            for run in &runs {
                for segment in segmented_optional_series(
                    run.iter()
                        .map(|frame| (frame.start_seconds, frame.pitch_hz)),
                ) {
                    chart.draw_series(LineSeries::new(segment, &RED))?;
                }
            }
            chart
                .draw_series(std::iter::once(PathElement::new(
                    vec![(0.0, 0.0), (0.0, 0.0)],
                    RED,
                )))?
                .label("Pitch")
                .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], RED));
            Ok(())
        },
    )
    .map(Some)
}

fn build_harmonics_graph(frames: &[FrameAnalysis]) -> Result<Option<GraphImage>, String> {
    let max_harmonics = frames
        .iter()
        .map(|frame| frame.harmonic_strengths.len())
        .max()
        .unwrap_or(0);
    if max_harmonics == 0 {
        return Ok(None);
    }

    let total_strengths: Vec<f32> = frames
        .iter()
        .map(|frame| {
            frame
                .harmonic_strengths
                .iter()
                .filter_map(|value| *value)
                .sum::<f32>()
        })
        .collect();
    if total_strengths.iter().all(|value| *value <= 0.0) {
        return Ok(None);
    }

    let x_range = time_range(frames);
    let display_ceiling = harmonic_display_ceiling(&total_strengths);
    let y_range = 0.0_f32..display_ceiling;
    let runs = voiced_runs(frames);

    render_graph(
        "Harmonic stack",
        "Cumulative strength ratio (F0 = 1)",
        x_range,
        y_range,
        |chart: &mut Chart2d<'_, '_>| {
            let mut drew_boundary_legend = false;
            for run in &runs {
                let mut lower = vec![0.0_f32; run.len()];
                for harmonic_index in 0..max_harmonics {
                    let upper: Vec<f32> = run
                        .iter()
                        .enumerate()
                        .map(|(frame_index, frame)| {
                            (lower[frame_index]
                                + frame
                                    .harmonic_strengths
                                    .get(harmonic_index)
                                    .copied()
                                    .flatten()
                                    .unwrap_or(0.0))
                            .min(display_ceiling)
                        })
                        .collect();

                    if upper
                        .iter()
                        .zip(lower.iter())
                        .all(|(upper, lower)| (upper - lower).abs() <= 1.0e-6)
                    {
                        continue;
                    }

                    let fill = harmonic_fill_color(harmonic_index, max_harmonics);
                    chart.draw_series(std::iter::once(Polygon::new(
                        center_band_polygon(run, &lower, &upper),
                        fill.mix(0.26).filled(),
                    )))?;
                    chart.draw_series(std::iter::once(PathElement::new(
                        center_series_points(run, &upper),
                        WHITE.mix(0.95).stroke_width(3),
                    )))?;
                    if harmonic_index < max_harmonics.saturating_sub(1) {
                        chart.draw_series(std::iter::once(PathElement::new(
                            center_series_points(run, &upper),
                            fill.stroke_width(1),
                        )))?;
                    }
                    if !drew_boundary_legend {
                        chart
                            .draw_series(std::iter::once(PathElement::new(
                                vec![(0.0, 0.0), (0.0, 0.0)],
                                WHITE.mix(0.95).stroke_width(3),
                            )))?
                            .label("Band boundaries")
                            .legend(|(x, y)| {
                                PathElement::new(
                                    vec![(x, y), (x + 24, y)],
                                    WHITE.mix(0.95).stroke_width(3),
                                )
                            });
                        drew_boundary_legend = true;
                    }

                    lower = upper;
                }

                chart.draw_series(std::iter::once(PathElement::new(
                    center_series_points(run, &lower),
                    BLACK.stroke_width(2),
                )))?;
            }
            chart
                .draw_series(std::iter::once(PathElement::new(
                    vec![(0.0, 0.0), (0.0, 0.0)],
                    harmonic_fill_color(0, max_harmonics).mix(0.26).filled(),
                )))?
                .label("Stacked harmonic bands")
                .legend({
                    let color = harmonic_fill_color(0, max_harmonics);
                    move |(x, y)| {
                        Rectangle::new(
                            [(x, y - 4), (x + 24, y + 4)],
                            color.mix(0.26).filled(),
                        )
                    }
                });
            chart
                .draw_series(std::iter::once(PathElement::new(
                    vec![(0.0, 0.0), (0.0, 0.0)],
                    BLACK.stroke_width(2),
                )))?
                .label("Total")
                .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], BLACK.stroke_width(2)));
            Ok(())
        },
    )
    .map(Some)
}

fn harmonic_display_ceiling(total_strengths: &[f32]) -> f32 {
    let mut sorted: Vec<f32> = total_strengths
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    if sorted.is_empty() {
        return 1.0;
    }

    sorted.sort_by(|a, b| a.total_cmp(b));
    let robust_peak = percentile_sorted(&sorted, 0.97);
    let absolute_peak = *sorted.last().unwrap_or(&robust_peak);
    let base = robust_peak.max(sorted[0]).max(0.2);
    let headroom = (base * 1.12).max(base + 0.1);
    headroom.min(absolute_peak.max(headroom))
}

fn percentile_sorted(values: &[f32], percentile: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    if values.len() == 1 {
        return values[0];
    }

    let position = percentile.clamp(0.0, 1.0) * (values.len() - 1) as f32;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        return values[lower];
    }

    let weight = position - lower as f32;
    values[lower] * (1.0 - weight) + values[upper] * weight
}

fn harmonic_fill_color(harmonic_index: usize, harmonic_count: usize) -> RGBColor {
    let normalized = if harmonic_count <= 1 {
        0.0
    } else {
        harmonic_index as f32 / (harmonic_count - 1) as f32
    };

    let hue_degrees = (220.0 - 255.0 * normalized).rem_euclid(360.0);
    let saturation = 0.58 + 0.18 * (1.0 - (2.0 * normalized - 1.0).abs());
    let value = 0.64 + 0.16 * (normalized * std::f32::consts::TAU * 2.0).sin().abs();
    hsv_to_rgb(hue_degrees, saturation.clamp(0.0, 1.0), value.clamp(0.0, 1.0))
}

fn hsv_to_rgb(hue_degrees: f32, saturation: f32, value: f32) -> RGBColor {
    let hue = hue_degrees.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let x = chroma * (1.0 - ((hue.rem_euclid(2.0)) - 1.0).abs());
    let (r1, g1, b1) = match hue.floor() as i32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;

    RGBColor(
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

fn center_band_polygon(
    frames: &[FrameAnalysis],
    lower: &[f32],
    upper: &[f32],
) -> Vec<(f32, f32)> {
    let mut polygon = center_series_points(frames, upper);
    for (x, y) in center_series_points(frames, lower).into_iter().rev() {
        polygon.push((x, y));
    }
    polygon
}

fn center_series_points(frames: &[FrameAnalysis], values: &[f32]) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(frames.len());
    for (frame, value) in frames.iter().zip(values.iter().copied()) {
        points.push((((frame.start_seconds + frame.end_seconds) * 0.5), value));
    }
    points
}

fn build_hnr_loudness_graph(frames: &[FrameAnalysis]) -> Result<Option<GraphImage>, String> {
    let hnr_values: Vec<f32> = frames.iter().map(|frame| frame.hnr_db).collect();
    let loudness_values: Vec<f32> = frames.iter().map(|frame| frame.loudness_dbfs).collect();
    if hnr_values.is_empty() || loudness_values.is_empty() {
        return Ok(None);
    }

    let x_range = time_range(frames);
    let mut combined_values = hnr_values.clone();
    combined_values.extend(loudness_values.iter().copied());
    let y_range = padded_range(&combined_values, 0.12, 4.0);
    let runs = voiced_runs(frames);

    let mut buffer = vec![255u8; (WIDTH * HEIGHT * 3) as usize];
    let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE).map_err(draw_err)?;

    let mut chart = ChartBuilder::on(&root)
        .margin(24)
        .caption("HNR and loudness", ("sans-serif", 34))
        .x_label_area_size(48)
        .y_label_area_size(68)
        .build_cartesian_2d(x_range.clone(), y_range)
        .map_err(draw_err)?;

    chart
        .configure_mesh()
        .x_desc("Time (s)")
        .y_desc("Level (dB / dBFS)")
        .light_line_style(RGBColor(220, 220, 220))
        .draw()
        .map_err(draw_err)?;

    for run in &runs {
        chart
            .draw_series(LineSeries::new(
                run.iter().map(|frame| (frame.start_seconds, frame.hnr_db)),
                &RED,
            ))
            .map_err(draw_err)?;
        chart
            .draw_series(LineSeries::new(
                run.iter()
                    .map(|frame| (frame.start_seconds, frame.loudness_dbfs)),
                &BLUE,
            ))
            .map_err(draw_err)?;
    }
    chart
        .draw_series(std::iter::once(PathElement::new(
            vec![(0.0, 0.0), (0.0, 0.0)],
            RED,
        )))
        .map_err(draw_err)?
        .label("HNR")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], RED));
    chart
        .draw_series(std::iter::once(PathElement::new(
            vec![(0.0, 0.0), (0.0, 0.0)],
            BLUE,
        )))
        .map_err(draw_err)?
        .label("Loudness")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], BLUE));

    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.9))
        .border_style(BLACK)
        .draw()
        .map_err(draw_err)?;

    drop(chart);
    root.present().map_err(draw_err)?;
    drop(root);

    Ok(Some(GraphImage {
        file_name: "hnr_loudness.png".to_string(),
        title: "HNR and loudness".to_string(),
        png_bytes: encode_png(buffer, WIDTH, HEIGHT)?,
    }))
}

fn build_tilt_graph(frames: &[FrameAnalysis]) -> Result<Option<GraphImage>, String> {
    let values: Vec<f32> = frames
        .iter()
        .map(|frame| frame.spectral_tilt_db_per_octave)
        .collect();
    if values.is_empty() {
        return Ok(None);
    }

    let x_range = time_range(frames);
    let y_range = padded_range(&values, 0.12, 1.0);
    let runs = voiced_runs(frames);

    render_graph(
        "Spectral tilt",
        "dB per octave",
        x_range,
        y_range,
        |chart: &mut Chart2d<'_, '_>| {
            for run in &runs {
                chart.draw_series(LineSeries::new(
                    run.iter()
                        .map(|frame| (frame.start_seconds, frame.spectral_tilt_db_per_octave)),
                    &MAGENTA,
                ))?;
            }
            chart
                .draw_series(std::iter::once(PathElement::new(
                    vec![(0.0, 0.0), (0.0, 0.0)],
                    MAGENTA,
                )))?
                .label("Tilt")
                .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], MAGENTA));
            Ok(())
        },
    )
    .map(Some)
}

fn build_spectral_graph(frames: &[FrameAnalysis]) -> Result<Option<GraphImage>, String> {
    let mut values = Vec::new();
    for frame in frames {
        values.push(frame.spectral_centroid_hz);
        values.push(frame.spectral_bandwidth_hz);
        values.push(frame.spectral_rolloff_hz);
    }
    if values.is_empty() {
        return Ok(None);
    }

    let x_range = time_range(frames);
    let y_range = padded_range(&values, 0.08, 100.0);
    let runs = voiced_runs(frames);
    let series = [
        ("Centroid", RGBColor(31, 119, 180)),
        ("Bandwidth", RGBColor(255, 127, 14)),
        ("Rolloff", RGBColor(44, 160, 44)),
    ];

    render_graph(
        "Spectral characteristics",
        "Frequency (Hz)",
        x_range,
        y_range,
        |chart: &mut Chart2d<'_, '_>| {
            for (label, color) in series {
                for run in &runs {
                    let points: Vec<(f32, f32)> = match label {
                        "Centroid" => run
                            .iter()
                            .map(|frame| (frame.start_seconds, frame.spectral_centroid_hz))
                            .collect(),
                        "Bandwidth" => run
                            .iter()
                            .map(|frame| (frame.start_seconds, frame.spectral_bandwidth_hz))
                            .collect(),
                        _ => run
                            .iter()
                            .map(|frame| (frame.start_seconds, frame.spectral_rolloff_hz))
                            .collect(),
                    };

                    chart.draw_series(LineSeries::new(points, color))?;
                }
                chart
                    .draw_series(std::iter::once(PathElement::new(
                        vec![(0.0, 0.0), (0.0, 0.0)],
                        color,
                    )))?
                    .label(label)
                    .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], color));
            }
            Ok(())
        },
    )
    .map(Some)
}

fn build_power_envelope_graph(
    spectrum: &libvoice::FftSpectrum,
    max_bin: usize,
    max_hz: f32,
) -> Result<Option<GraphImage>, String> {
    let averaged = average_power_spectrum(spectrum, max_bin);
    if averaged.len() < 8 {
        return Ok(None);
    }

    let envelope = spectral_envelope(&averaged, 10);
    let envelope_db = envelope_to_db(&envelope);
    let raw_db = envelope_to_db(&averaged);
    let y_range = padded_range(&envelope_db, 0.08, 6.0);

    let mut buffer = vec![255u8; (WIDTH * HEIGHT * 3) as usize];
    let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE).map_err(draw_err)?;

    let mut chart = ChartBuilder::on(&root)
        .margin(24)
        .caption("Spectrum envelope", ("sans-serif", 34))
        .x_label_area_size(56)
        .y_label_area_size(84)
        .build_cartesian_2d(0.0_f32..max_hz, y_range)
        .map_err(draw_err)?;

    chart
        .configure_mesh()
        .x_desc("Frequency (Hz)")
        .y_desc("Power (dB)")
        .light_line_style(RGBColor(220, 220, 220))
        .draw()
        .map_err(draw_err)?;

    chart
        .draw_series(LineSeries::new(
            raw_db
                .iter()
                .enumerate()
                .map(|(bin, value)| (bin as f32 * spectrum.bin_hz, *value)),
            RGBColor(160, 185, 220),
        ))
        .map_err(draw_err)?
        .label("Average power")
        .legend(|(x, y)| {
            PathElement::new(vec![(x, y), (x + 24, y)], RGBColor(160, 185, 220))
        });

    chart
        .draw_series(LineSeries::new(
            envelope_db
                .iter()
                .enumerate()
                .map(|(bin, value)| (bin as f32 * spectrum.bin_hz, *value)),
            RED.stroke_width(4),
        ))
        .map_err(draw_err)?
        .label("Peak envelope")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], RED.stroke_width(4)));

    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.9))
        .border_style(BLACK)
        .draw()
        .map_err(draw_err)?;

    drop(chart);
    root.present().map_err(draw_err)?;
    drop(root);

    Ok(Some(GraphImage {
        file_name: "spectrum_envelope.png".to_string(),
        title: "Spectrum envelope".to_string(),
        png_bytes: encode_png(buffer, WIDTH, HEIGHT)?,
    }))
}

fn build_envelope_spectrogram_graph(
    spectrum: &libvoice::FftSpectrum,
    max_bin: usize,
    max_hz: f32,
) -> Result<Option<GraphImage>, String> {
    let x_range = spectrum.frames[0].start_seconds
        ..spectrum.frames.last().map(|frame| frame.end_seconds).unwrap_or(0.01);
    let y_range = 0.0_f32..max_hz;

    let mut peak_db = f32::NEG_INFINITY;
    let mut smoothed_frames = Vec::with_capacity(spectrum.frames.len());
    for frame in &spectrum.frames {
        let powers = frame
            .magnitudes
            .iter()
            .take(max_bin + 1)
            .map(|magnitude| magnitude * magnitude)
            .collect::<Vec<_>>();
        let smoothed = spectral_envelope(&powers, 10);
        for value in smoothed.iter().skip(1) {
            peak_db = peak_db.max(power_to_db(*value));
        }
        smoothed_frames.push((frame.start_seconds, frame.end_seconds, frame.is_voiced, smoothed));
    }

    if !peak_db.is_finite() {
        return Ok(None);
    }

    let mut buffer = vec![255u8; (WIDTH * HEIGHT * 3) as usize];
    let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE).map_err(draw_err)?;

    let mut chart = ChartBuilder::on(&root)
        .margin(24)
        .caption("Envelope spectrogram", ("sans-serif", 34))
        .x_label_area_size(56)
        .y_label_area_size(72)
        .build_cartesian_2d(x_range, y_range)
        .map_err(draw_err)?;

    chart
        .configure_mesh()
        .x_desc("Time (s)")
        .y_desc("Frequency (Hz)")
        .light_line_style(RGBColor(220, 220, 220))
        .draw()
        .map_err(draw_err)?;

    for (start, end, is_voiced, smoothed) in smoothed_frames {
        for bin in 1..smoothed.len() {
            let lower_hz = (bin - 1) as f32 * spectrum.bin_hz;
            let upper_hz = bin as f32 * spectrum.bin_hz;
            let db = power_to_db(smoothed[bin]);
            let normalized = ((db - peak_db + 80.0) / 80.0).clamp(0.0, 1.0);
            let color = spectrogram_color(normalized, is_voiced);
            chart
                .draw_series(std::iter::once(Rectangle::new(
                    [(start, lower_hz), (end, upper_hz)],
                    color.filled(),
                )))
                .map_err(draw_err)?;
        }
    }

    chart
        .configure_series_labels()
        .background_style(BLACK.mix(0.55))
        .border_style(WHITE)
        .draw()
        .map_err(draw_err)?;

    drop(chart);
    root.present().map_err(draw_err)?;
    drop(root);

    Ok(Some(GraphImage {
        file_name: "envelope_spectrogram.png".to_string(),
        title: "Envelope spectrogram".to_string(),
        png_bytes: encode_png(buffer, WIDTH, HEIGHT)?,
    }))
}

fn render_graph<F>(
    title: &str,
    y_desc: &str,
    x_range: std::ops::Range<f32>,
    y_range: std::ops::Range<f32>,
    draw: F,
) -> Result<GraphImage, String>
where
    F: FnOnce(
        &mut Chart2d<'_, '_>,
    ) -> Result<
        (),
        DrawingAreaErrorKind<<BitMapBackend<'static> as DrawingBackend>::ErrorType>,
    >,
{
    let mut buffer = vec![255u8; (WIDTH * HEIGHT * 3) as usize];
    let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE).map_err(draw_err)?;

    let mut chart = ChartBuilder::on(&root)
        .margin(24)
        .caption(title, ("sans-serif", 34))
        .x_label_area_size(48)
        .y_label_area_size(68)
        .build_cartesian_2d(x_range, y_range)
        .map_err(draw_err)?;

    chart
        .configure_mesh()
        .x_desc("Time (s)")
        .y_desc(y_desc)
        .light_line_style(RGBColor(220, 220, 220))
        .draw()
        .map_err(draw_err)?;

    draw(&mut chart).map_err(draw_err)?;

    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.9))
        .border_style(BLACK)
        .draw()
        .map_err(draw_err)?;

    drop(chart);
    root.present().map_err(draw_err)?;
    drop(root);

    Ok(GraphImage {
        file_name: slugify(title),
        title: title.to_string(),
        png_bytes: encode_png(buffer, WIDTH, HEIGHT)?,
    })
}

fn spectrogram_color(level: f32, is_voiced: bool) -> RGBColor {
    let anchors = if is_voiced {
        [
            (8.0, 12.0, 24.0),
            (35.0, 50.0, 110.0),
            (45.0, 135.0, 185.0),
            (245.0, 180.0, 65.0),
            (255.0, 246.0, 220.0),
        ]
    } else {
        [
            (8.0, 10.0, 16.0),
            (20.0, 26.0, 40.0),
            (40.0, 54.0, 74.0),
            (100.0, 110.0, 120.0),
            (180.0, 188.0, 196.0),
        ]
    };
    gradient_color(level, &anchors)
}

fn gradient_color(level: f32, anchors: &[(f32, f32, f32); 5]) -> RGBColor {
    let scaled = level.clamp(0.0, 1.0) * (anchors.len() - 1) as f32;
    let left = scaled.floor() as usize;
    let right = scaled.ceil() as usize;
    if left == right {
        let (r, g, b) = anchors[left];
        return RGBColor(r as u8, g as u8, b as u8);
    }

    let mix = scaled - left as f32;
    let (lr, lg, lb) = anchors[left];
    let (rr, rg, rb) = anchors[right];
    RGBColor(
        (lr + (rr - lr) * mix).round() as u8,
        (lg + (rg - lg) * mix).round() as u8,
        (lb + (rb - lb) * mix).round() as u8,
    )
}

fn segmented_optional_series<I>(iter: I) -> Vec<Vec<(f32, f32)>>
where
    I: IntoIterator<Item = (f32, Option<f32>)>,
{
    let mut segments = Vec::new();
    let mut current = Vec::new();

    for (x, maybe_y) in iter {
        match maybe_y {
            Some(y) => current.push((x, y)),
            None if !current.is_empty() => {
                segments.push(std::mem::take(&mut current));
            }
            None => {}
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    segments
}

fn voiced_runs(frames: &[FrameAnalysis]) -> Vec<&[FrameAnalysis]> {
    if frames.is_empty() {
        return Vec::new();
    }

    let tolerance = voiced_gap_tolerance(frames);
    let mut runs = Vec::new();
    let mut start = 0usize;

    for index in 1..frames.len() {
        let previous = &frames[index - 1];
        let current = &frames[index];
        if current.start_seconds - previous.end_seconds > tolerance {
            runs.push(&frames[start..index]);
            start = index;
        }
    }

    runs.push(&frames[start..]);
    runs
}

fn voiced_gap_tolerance(frames: &[FrameAnalysis]) -> f32 {
    let mut durations: Vec<f32> = frames
        .iter()
        .map(|frame| (frame.end_seconds - frame.start_seconds).max(1.0e-3))
        .collect();
    durations.sort_by(|a, b| a.total_cmp(b));
    let median = durations[durations.len() / 2];
    (median * 0.75).max(0.01)
}

fn time_range(frames: &[FrameAnalysis]) -> std::ops::Range<f32> {
    let start = frames
        .first()
        .map(|frame| frame.start_seconds)
        .unwrap_or(0.0);
    let end = frames
        .last()
        .map(|frame| frame.end_seconds.max(frame.start_seconds + 0.01))
        .unwrap_or(1.0);
    if (end - start).abs() < 1.0e-6 {
        start..(start + 1.0)
    } else {
        start..end
    }
}

fn padded_range(values: &[f32], padding_fraction: f32, min_span: f32) -> std::ops::Range<f32> {
    let min = values.iter().copied().reduce(f32::min).unwrap_or(0.0);
    let max = values.iter().copied().reduce(f32::max).unwrap_or(1.0);
    let span = (max - min).max(min_span);
    let pad = span * padding_fraction.max(0.02);
    (min - pad)..(max + pad)
}

fn encode_png(buffer: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder
        .write_image(&buffer, width, height, ColorType::Rgb8.into())
        .map_err(|error| format!("failed to encode graph png: {error}"))?;
    Ok(png_bytes)
}

fn average_power_spectrum(spectrum: &libvoice::FftSpectrum, max_bin: usize) -> Vec<f32> {
    let mut sum = vec![0.0; max_bin + 1];
    let mut count = 0usize;

    for frame in &spectrum.frames {
        if !frame.is_voiced {
            continue;
        }
        for (bin, magnitude) in frame.magnitudes.iter().take(max_bin + 1).enumerate() {
            sum[bin] += magnitude * magnitude;
        }
        count += 1;
    }

    if count == 0 {
        for frame in &spectrum.frames {
            for (bin, magnitude) in frame.magnitudes.iter().take(max_bin + 1).enumerate() {
                sum[bin] += magnitude * magnitude;
            }
        }
        count = spectrum.frames.len();
    }

    if count == 0 {
        return Vec::new();
    }

    sum.into_iter().map(|value| value / count as f32).collect()
}

fn spectral_envelope(power: &[f32], window_radius: usize) -> Vec<f32> {
    if power.is_empty() {
        return Vec::new();
    }

    let mut peaks = vec![0.0; power.len()];
    for index in 0..power.len() {
        let start = index.saturating_sub(window_radius);
        let end = (index + window_radius + 1).min(power.len());
        let mut peak: f32 = 0.0;
        for value in &power[start..end] {
            peak = peak.max(*value);
        }
        peaks[index] = peak;
    }

    let mut smoothed = vec![0.0; power.len()];
    for index in 0..power.len() {
        let start = index.saturating_sub(window_radius);
        let end = (index + window_radius + 1).min(power.len());
        let slice = &peaks[start..end];
        let sum: f32 = slice.iter().sum();
        smoothed[index] = sum / slice.len() as f32;
    }
    smoothed
}

fn envelope_to_db(values: &[f32]) -> Vec<f32> {
    values.iter().map(|value| power_to_db(*value)).collect()
}

fn power_to_db(value: f32) -> f32 {
    10.0 * value.max(1.0e-24).log10()
}

fn slugify(title: &str) -> String {
    let mut file = title.to_ascii_lowercase().replace(' ', "_");
    file.retain(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if file.is_empty() {
        file = "graph".to_string();
    }
    format!("{file}.png")
}

fn draw_err<E: std::fmt::Display>(error: E) -> String {
    format!("failed to render graph: {error}")
}
