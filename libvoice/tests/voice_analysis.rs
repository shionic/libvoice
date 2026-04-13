use libvoice::{AnalysisReport, AnalyzerConfig, VoiceAnalyzer};
use std::f32::consts::PI;

fn synth_sine(sample_rate: u32, frequency_hz: f32, seconds: f32, amplitude: f32) -> Vec<f32> {
    let total = (sample_rate as f32 * seconds) as usize;
    (0..total)
        .map(|index| {
            let t = index as f32 / sample_rate as f32;
            (2.0 * PI * frequency_hz * t).sin() * amplitude
        })
        .collect()
}

fn synth_noise(sample_rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
    let total = (sample_rate as f32 * seconds) as usize;
    let mut state = 0x1234_5678_u32;

    (0..total)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let normalized = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            normalized * amplitude
        })
        .collect()
}

fn synth_harmonic_stack(
    sample_rate: u32,
    pitch_hz: f32,
    seconds: f32,
    harmonic_amplitudes: &[f32],
) -> Vec<f32> {
    let total = (sample_rate as f32 * seconds) as usize;
    let mut output = vec![0.0_f32; total];

    for (index, sample) in output.iter_mut().enumerate() {
        let t = index as f32 / sample_rate as f32;
        let mut value = 0.0_f32;
        for (harmonic_index, amplitude) in harmonic_amplitudes.iter().copied().enumerate() {
            if amplitude <= 0.0 {
                continue;
            }
            let harmonic_number = harmonic_index + 1;
            value += amplitude
                * (2.0 * PI * pitch_hz * harmonic_number as f32 * t).sin();
        }
        *sample = value;
    }

    let peak = output
        .iter()
        .copied()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    if peak > 0.0 {
        for sample in &mut output {
            *sample *= 0.6 / peak;
        }
    }

    output
}

fn approx_eq(left: f32, right: f32, tolerance: f32) {
    assert!(
        (left - right).abs() <= tolerance,
        "left={left}, right={right}, tolerance={tolerance}"
    );
}

fn assert_reports_close(full: &AnalysisReport, streamed: &AnalysisReport) {
    assert_eq!(full.overall.frame_count, streamed.overall.frame_count);
    assert_eq!(
        full.overall.processed_samples,
        streamed.overall.processed_samples
    );
    assert_eq!(full.frames.len(), streamed.frames.len());

    let full_pitch = full.overall.pitch_hz.as_ref().unwrap();
    let streamed_pitch = streamed.overall.pitch_hz.as_ref().unwrap();
    approx_eq(full_pitch.mean, streamed_pitch.mean, 0.01);
    approx_eq(full_pitch.std, streamed_pitch.std, 0.01);

    let full_energy = full.overall.energy.as_ref().unwrap();
    let streamed_energy = streamed.overall.energy.as_ref().unwrap();
    approx_eq(full_energy.mean, streamed_energy.mean, 1.0e-6);
    approx_eq(full_energy.std, streamed_energy.std, 1.0e-6);

    let full_spectral = full.overall.spectral.as_ref().unwrap();
    let streamed_spectral = streamed.overall.spectral.as_ref().unwrap();
    approx_eq(
        full_spectral.centroid_hz.mean,
        streamed_spectral.centroid_hz.mean,
        0.01,
    );
    approx_eq(
        full_spectral.rolloff_hz.mean,
        streamed_spectral.rolloff_hz.mean,
        0.01,
    );
    approx_eq(
        full_spectral.bandwidth_hz.mean,
        streamed_spectral.bandwidth_hz.mean,
        0.01,
    );
    approx_eq(
        full_spectral.tilt_db_per_octave.mean,
        streamed_spectral.tilt_db_per_octave.mean,
        0.01,
    );
}

#[test]
fn pitch_tracks_multiple_stable_frequencies() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);

    for expected_hz in [110.0_f32, 180.0, 220.0, 320.0] {
        let samples = synth_sine(sample_rate, expected_hz, 1.2, 0.5);
        let report = VoiceAnalyzer::analyze_buffer(config.clone(), &samples);
        let pitch = report
            .overall
            .pitch_hz
            .expect("stable tone should be voiced");

        approx_eq(pitch.mean, expected_hz, 12.0);
        assert!(
            pitch.std < 8.0,
            "unexpected pitch std for {expected_hz} Hz: {}",
            pitch.std
        );
        assert!(pitch.p5 > expected_hz * 0.80);
        assert!(pitch.p95 < expected_hz * 1.20);
    }
}

#[test]
fn high_pitch_mode_tracks_high_voice_fundamentals() {
    let sample_rate = 16_000;
    let mut config = AnalyzerConfig::new(sample_rate);
    config.apply_high_pitch_mode();

    let samples = synth_sine(sample_rate, 1_000.0, 1.0, 0.5);
    let report = VoiceAnalyzer::analyze_buffer(config, &samples);
    let pitch = report
        .overall
        .pitch_hz
        .expect("high-pitch mode should keep 1000 Hz voiced");

    approx_eq(pitch.mean, 1_000.0, 25.0);
}

#[test]
fn high_pitch_mode_preserves_harmonic_detection_near_upper_pitch_limit() {
    let sample_rate = 16_000;
    let mut config = AnalyzerConfig::new(sample_rate);
    config.apply_high_pitch_mode();

    let samples = synth_harmonic_stack(sample_rate, 900.0, 1.0, &[1.0, 0.45, 0.25, 0.12, 0.06]);
    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    let pitch = report
        .overall
        .pitch_hz
        .expect("high-pitch harmonic stack should remain voiced");
    approx_eq(pitch.mean, 900.0, 20.0);

    let harmonics = report
        .overall
        .harmonics
        .expect("high-pitch mode should still expose harmonics");

    let h2 = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 2)
        .expect("expected H2 below 5000 Hz");
    let h3 = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 3)
        .expect("expected H3 below 5000 Hz");
    let h4 = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 4)
        .expect("expected H4 below 5000 Hz");
    let h5 = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 5)
        .expect("expected H5 when high-pitch mode raises the harmonic cap");

    approx_eq(h2.strength_ratio.mean, 0.45, 0.14);
    approx_eq(h3.strength_ratio.mean, 0.25, 0.12);
    approx_eq(h4.strength_ratio.mean, 0.12, 0.08);
    approx_eq(h5.strength_ratio.mean, 0.06, 0.05);
    assert!(harmonics.max_frequency_hz > 5_000.0);
    assert!(
        harmonics.harmonics.iter().all(|harmonic| harmonic.harmonic_number <= 8),
        "900 Hz F0 with a raised cap should still be bounded by Nyquist"
    );
}

#[test]
fn streaming_matches_full_buffer_metrics_across_irregular_chunks() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_sine(sample_rate, 205.0, 1.5, 0.5);

    let full = VoiceAnalyzer::analyze_buffer(config.clone(), &samples);
    let streamed = VoiceAnalyzer::analyze_buffer_in_chunks(config, &samples, 317);

    assert!(streamed.chunks.len() > 10);
    assert!(streamed.chunks.iter().any(|chunk| chunk.frame_count > 0));
    assert_reports_close(&full, &streamed);
}

#[test]
fn streaming_can_return_frame_level_results() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_sine(sample_rate, 220.0, 1.0, 0.5);

    let mut analyzer = VoiceAnalyzer::new(config);
    let (chunk, frames) = analyzer.process_chunk_with_frames(&samples);
    let overall = analyzer.finalize();

    assert_eq!(chunk.frame_count, frames.len());
    assert_eq!(overall.frame_count, frames.len());
    assert!(!frames.is_empty());
    assert_eq!(frames[0].frame_index, 0);
    assert!(frames[0].start_seconds >= 0.0);
    assert!(frames[0].end_seconds > frames[0].start_seconds);
    assert!(frames[0].pitch_hz.is_some());
    assert!(!frames[0].harmonic_strengths.is_empty());
    assert_eq!(frames[0].harmonic_strengths[0], Some(1.0));
    assert_eq!(frames[0].cumulative.frame_count, 1);
    assert_eq!(frames.last().unwrap().cumulative, overall);
}

#[test]
fn streaming_accumulates_metrics_consistently_with_variable_chunk_sizes() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_sine(sample_rate, 240.0, 1.1, 0.35);
    let expected = VoiceAnalyzer::analyze_buffer(config.clone(), &samples);

    let mut analyzer = VoiceAnalyzer::new(config);
    let mut offset = 0;
    let chunk_pattern = [13_usize, 257, 509, 1024, 97, 701];
    let mut index = 0;

    while offset < samples.len() {
        let len = chunk_pattern[index % chunk_pattern.len()];
        let end = (offset + len).min(samples.len());
        analyzer.process_chunk(&samples[offset..end]);
        offset = end;
        index += 1;
    }

    let actual = AnalysisReport {
        config: analyzer.config().clone(),
        frames: Vec::new(),
        chunks: Vec::new(),
        overall: analyzer.finalize(),
        fft_spectrum: None,
    };

    assert_eq!(expected.overall.frame_count, actual.overall.frame_count);
    approx_eq(
        expected.overall.pitch_hz.as_ref().unwrap().mean,
        actual.overall.pitch_hz.as_ref().unwrap().mean,
        0.01,
    );
    approx_eq(
        expected.overall.spectral.as_ref().unwrap().flatness.mean,
        actual.overall.spectral.as_ref().unwrap().flatness.mean,
        1.0e-6,
    );
    approx_eq(
        expected
            .overall
            .spectral
            .as_ref()
            .unwrap()
            .tilt_db_per_octave
            .mean,
        actual
            .overall
            .spectral
            .as_ref()
            .unwrap()
            .tilt_db_per_octave
            .mean,
        1.0e-6,
    );
}

#[test]
fn silence_produces_no_pitch_or_jitter_and_zero_energy() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = vec![0.0_f32; sample_rate as usize];

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert_eq!(report.overall.frame_count, 0);
    assert!(report.overall.pitch_hz.is_none());
    assert!(report.overall.jitter.is_none());
    assert!(report.overall.energy.is_none());
    assert!(report.overall.spectral.is_none());
}

#[test]
fn broadband_noise_is_skipped_as_non_voice() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_noise(sample_rate, 1.0, 0.4);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert_eq!(report.overall.frame_count, 0);
    assert!(report.overall.pitch_hz.is_none());
    assert!(report.overall.energy.is_none());
    assert!(report.overall.spectral.is_none());
}

#[test]
fn mixed_signal_excludes_silence_and_noise_from_voiced_metrics() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);

    let mut samples = vec![0.0_f32; sample_rate as usize / 2];
    samples.extend(synth_noise(sample_rate, 0.5, 0.35));
    samples.extend(synth_sine(sample_rate, 220.0, 1.0, 0.5));
    samples.extend(vec![0.0_f32; sample_rate as usize / 2]);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert!(report.overall.frame_count > 0);
    let pitch = report
        .overall
        .pitch_hz
        .expect("voiced section should remain");
    approx_eq(pitch.mean, 220.0, 12.0);
    let energy = report
        .overall
        .energy
        .expect("voiced section should contribute energy");
    assert!(energy.mean > 0.05);
}

#[test]
fn speech_offset_frames_with_silent_tails_are_rejected() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);

    let mut samples = synth_sine(sample_rate, 220.0, 0.9, 0.5);
    samples.extend(vec![0.0_f32; (sample_rate as f32 * 0.5) as usize]);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert!(!report.frames.is_empty());
    let last = report.frames.last().unwrap();
    let frame_midpoint = 0.5 * (last.start_seconds + last.end_seconds);
    assert!(
        frame_midpoint <= 0.9 + 0.01,
        "last voiced midpoint should stay inside spoken region, got frame {:.3}-{:.3}s",
        last.start_seconds,
        last.end_seconds
    );
}

#[test]
fn voiced_sine_produces_concentrated_spectral_summary() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_sine(sample_rate, 220.0, 1.0, 0.5);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);
    let spectral = report
        .overall
        .spectral
        .expect("stable voiced tone should have spectral metrics");

    assert!(spectral.centroid_hz.mean > 180.0);
    assert!(spectral.centroid_hz.mean < 350.0);
    assert!(spectral.rolloff_hz.mean < 500.0);
    assert!(spectral.bandwidth_hz.mean < 250.0);
    assert!(spectral.flatness.mean < 0.1);
    assert!(spectral.tilt_db_per_octave.mean.is_finite());
}

#[test]
fn harmonic_stack_reports_normalized_harmonic_strengths() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_harmonic_stack(sample_rate, 140.0, 1.2, &[1.0, 0.5, 0.0, 0.25, 0.1]);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);
    let harmonics = report
        .overall
        .harmonics
        .expect("voiced harmonic stack should expose harmonics");

    assert!(harmonics.normalized_to_f0);
    let first = &harmonics.harmonics[0];
    assert_eq!(first.harmonic_number, 1);
    approx_eq(first.strength_ratio.mean, 1.0, 0.05);

    let second = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 2)
        .unwrap();
    approx_eq(second.strength_ratio.mean, 0.5, 0.12);

    let third = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 3);
    assert!(third.is_none(), "weak or absent harmonics should not be reindexed");

    let fourth = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 4)
        .unwrap();
    approx_eq(fourth.strength_ratio.mean, 0.25, 0.12);
}

#[test]
fn streaming_matches_harmonic_strengths_for_harmonic_stack() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_harmonic_stack(sample_rate, 140.0, 1.2, &[1.0, 0.5, 0.0, 0.25, 0.1]);

    let full = VoiceAnalyzer::analyze_buffer(config.clone(), &samples);
    let streamed = VoiceAnalyzer::analyze_buffer_in_chunks(config, &samples, 317);

    assert_reports_close(&full, &streamed);

    let full_harmonics = full.overall.harmonics.as_ref().unwrap();
    let streamed_harmonics = streamed.overall.harmonics.as_ref().unwrap();
    approx_eq(
        full_harmonics.harmonics[0].strength_ratio.mean,
        streamed_harmonics.harmonics[0].strength_ratio.mean,
        0.1,
    );
    approx_eq(
        full_harmonics.harmonics[1].strength_ratio.mean,
        streamed_harmonics.harmonics[1].strength_ratio.mean,
        0.1,
    );
}

#[test]
fn harmonic_count_expands_with_available_frequency_range() {
    let low_rate = 16_000;
    let high_rate = 48_000;
    let harmonic_amplitudes: Vec<f32> = (1..=60).map(|harmonic| 1.0 / harmonic as f32).collect();
    let low_report = VoiceAnalyzer::analyze_buffer(
        AnalyzerConfig::new(low_rate),
        &synth_harmonic_stack(low_rate, 110.0, 1.2, &harmonic_amplitudes),
    );
    let high_report = VoiceAnalyzer::analyze_buffer(
        AnalyzerConfig::new(high_rate),
        &synth_harmonic_stack(high_rate, 110.0, 1.2, &harmonic_amplitudes),
    );

    let low_harmonics = low_report.overall.harmonics.as_ref().unwrap();
    let high_harmonics = high_report.overall.harmonics.as_ref().unwrap();

    assert!(low_harmonics.harmonics.len() >= 40);
    assert!(high_harmonics.harmonics.len() >= low_harmonics.harmonics.len());
    assert!(low_harmonics.max_frequency_hz <= 5_000.0 + 150.0);
    assert!(high_harmonics.max_frequency_hz <= 5_000.0 + 150.0);
}

#[test]
fn report_serializes_to_json() {
    let sample_rate = 16_000;
    let samples = synth_sine(sample_rate, 220.0, 0.5, 0.5);
    let report =
        VoiceAnalyzer::analyze_buffer_in_chunks(AnalyzerConfig::new(sample_rate), &samples, 400);

    let json = serde_json::to_string(&report).expect("report should serialize");
    assert!(json.contains("\"overall\""));
    assert!(json.contains("\"frames\""));
    assert!(json.contains("\"chunks\""));
    assert!(json.contains("\"spectral\""));
    assert!(json.contains("\"tilt_db_per_octave\""));
    assert!(json.contains("\"harmonics\""));
}

#[test]
fn report_exposes_frames_with_cumulative_statistics() {
    let sample_rate = 16_000;
    let samples = synth_sine(sample_rate, 220.0, 0.8, 0.5);
    let report = VoiceAnalyzer::analyze_buffer(AnalyzerConfig::new(sample_rate), &samples);

    assert_eq!(report.frames.len(), report.overall.frame_count);
    assert!(!report.frames.is_empty());

    let first = &report.frames[0];
    assert_eq!(first.cumulative.frame_count, 1);
    assert_eq!(
        first.cumulative.pitch_hz.as_ref().unwrap().mean,
        first.pitch_hz.unwrap()
    );
    assert!(first.spectral_tilt_db_per_octave.is_finite());

    let last = report.frames.last().unwrap();
    assert_eq!(last.cumulative, report.overall);
    assert!(last.cumulative.pitch_hz.as_ref().unwrap().median > 0.0);
    assert!(last.cumulative.pitch_hz.as_ref().unwrap().p5 > 0.0);
    assert!(last.cumulative.pitch_hz.as_ref().unwrap().p95 > 0.0);
    assert!(last.cumulative.spectral.as_ref().is_some());
}
