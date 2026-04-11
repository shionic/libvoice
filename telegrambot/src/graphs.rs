use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use libvoice::{AnalysisReport, FftSpectrum, FrameAnalysis};
use plotters::coord::types::RangedCoordf32;
use plotters::prelude::*;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
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
    if let Some(graph) = build_formants_graph(frames)? {
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
    if spectrum.magnitudes.len() < 8 || spectrum.bin_hz <= 0.0 {
        return Ok(None);
    }

    let nyquist_hz = (spectrum.magnitudes.len().saturating_sub(1)) as f32 * spectrum.bin_hz;
    let min_hz = 50.0_f32.max(spectrum.bin_hz);
    let max_hz = nyquist_hz.min(5_000.0);
    if max_hz <= min_hz {
        return Ok(None);
    }

    let start_index = (min_hz / spectrum.bin_hz).ceil() as usize;
    let end_index = (max_hz / spectrum.bin_hz).floor() as usize;
    if end_index <= start_index {
        return Ok(None);
    }

    let smoothed = smooth_spectrum_magnitudes(spectrum, start_index, end_index);
    let raw_points = spectrum_points_db(spectrum, start_index, end_index, &spectrum.magnitudes);
    let smooth_points = spectrum_points_db(spectrum, start_index, end_index, &smoothed);
    let peak_db = raw_points
        .iter()
        .chain(smooth_points.iter())
        .map(|(_, value)| *value)
        .fold(f32::NEG_INFINITY, f32::max);
    if !peak_db.is_finite() {
        return Ok(None);
    }

    let normalize = |points: Vec<(f32, f32)>| -> Vec<(f32, f32)> {
        points
            .into_iter()
            .map(|(x, y)| (x, (y - peak_db).max(-90.0)))
            .collect()
    };
    let raw_points = normalize(raw_points);
    let smooth_points = normalize(smooth_points);
    let x_range = min_hz.log10()..max_hz.log10();
    let y_range = -90.0_f32..3.0_f32;

    let mut buffer = vec![255u8; (WIDTH * HEIGHT * 3) as usize];
    let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
    root.fill(&WHITE).map_err(draw_err)?;

    let mut chart = ChartBuilder::on(&root)
        .margin(24)
        .caption("Voice spectrum", ("sans-serif", 34))
        .x_label_area_size(56)
        .y_label_area_size(72)
        .build_cartesian_2d(x_range, y_range)
        .map_err(draw_err)?;

    chart
        .configure_mesh()
        .x_desc("Frequency (Hz)")
        .y_desc("Level (dB rel. peak)")
        .x_labels(10)
        .x_label_formatter(&|value| format_frequency_label(10.0_f32.powf(*value)))
        .light_line_style(RGBColor(220, 220, 220))
        .draw()
        .map_err(draw_err)?;

    chart
        .draw_series(LineSeries::new(
            raw_points,
            RGBColor(120, 150, 200).mix(0.55).stroke_width(1),
        ))
        .map_err(draw_err)?
        .label("Average FFT")
        .legend(|(x, y)| {
            PathElement::new(
                vec![(x, y), (x + 24, y)],
                RGBColor(120, 150, 200).mix(0.55).stroke_width(2),
            )
        });

    chart
        .draw_series(LineSeries::new(
            smooth_points,
            RGBColor(196, 45, 42).stroke_width(3),
        ))
        .map_err(draw_err)?
        .label("Smoothed envelope")
        .legend(|(x, y)| {
            PathElement::new(
                vec![(x, y), (x + 24, y)],
                RGBColor(196, 45, 42).stroke_width(3),
            )
        });

    if let Some(formants) = report.overall.formants.as_ref() {
        for (label, color, formant) in [
            ("F1", BLUE, formants.f1.as_ref()),
            ("F2", GREEN, formants.f2.as_ref()),
            ("F3", MAGENTA, formants.f3.as_ref()),
            ("F4", CYAN, formants.f4.as_ref()),
        ] {
            let Some(formant) = formant else {
                continue;
            };
            let hz = formant.frequency_hz.mean;
            if hz < min_hz || hz > max_hz {
                continue;
            }
            let x = hz.log10();
            chart
                .draw_series(std::iter::once(PathElement::new(
                    vec![(x, -90.0), (x, 3.0)],
                    color.mix(0.55),
                )))
                .map_err(draw_err)?
                .label(label)
                .legend(move |(x, y)| PathElement::new(vec![(x, y - 4), (x, y + 4)], color));
        }
    }

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
        file_name: "voice_spectrum.png".to_string(),
        title: "Voice spectrum".to_string(),
        png_bytes: encode_png(buffer, WIDTH, HEIGHT)?,
    }))
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

fn build_formants_graph(frames: &[FrameAnalysis]) -> Result<Option<GraphImage>, String> {
    let mut all_values = Vec::new();
    for frame in frames {
        all_values.extend(
            frame
                .formants_hz
                .iter()
                .copied()
                .filter(|value| *value > 0.0),
        );
    }
    if all_values.is_empty() {
        return Ok(None);
    }

    let x_range = time_range(frames);
    let y_range = padded_range(&all_values, 0.08, 120.0);
    let colors = [BLUE, RED, GREEN, MAGENTA];
    let labels = ["F1", "F2", "F3", "F4"];
    let runs = voiced_runs(frames);

    render_graph(
        "Formants",
        "Frequency (Hz)",
        x_range,
        y_range,
        |chart: &mut Chart2d<'_, '_>| {
            for (slot, (label, color)) in labels.iter().zip(colors.iter()).enumerate() {
                for run in &runs {
                    let series = segmented_optional_series(run.iter().map(|frame| {
                        (
                            frame.start_seconds,
                            frame
                                .formants_hz
                                .get(slot)
                                .copied()
                                .filter(|value| *value > 0.0),
                        )
                    }));
                    for segment in series {
                        chart.draw_series(LineSeries::new(segment, color))?;
                    }
                }
                chart
                    .draw_series(std::iter::once(PathElement::new(
                        vec![(0.0, 0.0), (0.0, 0.0)],
                        color,
                    )))?
                    .label(*label)
                    .legend({
                        let color = *color;
                        move |(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], color)
                    });
            }
            Ok(())
        },
    )
    .map(Some)
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

fn smooth_spectrum_magnitudes(
    spectrum: &FftSpectrum,
    start_index: usize,
    end_index: usize,
) -> Vec<f32> {
    let mut smoothed = spectrum.magnitudes.clone();
    for index in start_index..=end_index {
        let radius = (index / 24).max(2);
        let low = index.saturating_sub(radius).max(1);
        let high = (index + radius).min(spectrum.magnitudes.len().saturating_sub(1));
        let mut sum = 0.0_f32;
        let mut weight_sum = 0.0_f32;

        for inner in low..=high {
            let distance = index.abs_diff(inner) as f32;
            let weight = 1.0 / (1.0 + distance);
            sum += spectrum.magnitudes[inner] * weight;
            weight_sum += weight;
        }

        if weight_sum > 0.0 {
            smoothed[index] = sum / weight_sum;
        }
    }
    smoothed
}

fn spectrum_points_db(
    spectrum: &FftSpectrum,
    start_index: usize,
    end_index: usize,
    magnitudes: &[f32],
) -> Vec<(f32, f32)> {
    (start_index..=end_index)
        .map(|index| {
            let hz = index as f32 * spectrum.bin_hz;
            let db = 20.0 * magnitudes[index].max(1.0e-12).log10();
            (hz.log10(), db)
        })
        .collect()
}

fn format_frequency_label(hz: f32) -> String {
    if hz >= 1000.0 {
        let khz = hz / 1000.0;
        if khz >= 10.0 || (khz - khz.round()).abs() < 0.05 {
            format!("{khz:.0}k")
        } else {
            format!("{khz:.1}k")
        }
    } else {
        format!("{hz:.0}")
    }
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
