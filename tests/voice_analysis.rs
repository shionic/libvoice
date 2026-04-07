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

fn synth_vibrato(
    sample_rate: u32,
    carrier_hz: f32,
    vibrato_hz: f32,
    vibrato_extent_hz: f32,
    seconds: f32,
    amplitude: f32,
) -> Vec<f32> {
    let total = (sample_rate as f32 * seconds) as usize;
    let mut phase = 0.0_f32;
    let dt = 1.0 / sample_rate as f32;

    (0..total)
        .map(|index| {
            let t = index as f32 * dt;
            let instant_hz = carrier_hz + vibrato_extent_hz * (2.0 * PI * vibrato_hz * t).sin();
            phase += 2.0 * PI * instant_hz * dt;
            phase.sin() * amplitude
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
        chunks: Vec::new(),
        overall: analyzer.finalize(),
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
}

#[test]
fn vibrato_signal_produces_nonzero_jitter_metrics() {
    let sample_rate = 16_000;
    let mut config = AnalyzerConfig::new(sample_rate);
    config.frame_size = 1024;
    config.hop_size = 128;

    let samples = synth_vibrato(sample_rate, 220.0, 5.0, 12.0, 2.0, 0.5);
    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    let jitter = report
        .overall
        .jitter
        .expect("vibrato signal should produce jitter metrics");
    assert!(
        jitter.direction_change_rate > 0.02,
        "direction_change_rate={}",
        jitter.direction_change_rate
    );
    assert!(
        jitter.local_hz_mean > 0.5,
        "local_hz_mean={}",
        jitter.local_hz_mean
    );
    assert!(
        jitter.estimated_vibrato_extent_cents > 5.0,
        "estimated_vibrato_extent_cents={}",
        jitter.estimated_vibrato_extent_cents
    );
    assert!(
        jitter.estimated_vibrato_hz > 2.0 && jitter.estimated_vibrato_hz < 8.0,
        "estimated_vibrato_hz={}",
        jitter.estimated_vibrato_hz
    );
    assert!(jitter.rap_ratio >= 0.0, "rap_ratio={}", jitter.rap_ratio);
    assert!(jitter.ppq5_ratio >= 0.0, "ppq5_ratio={}", jitter.ppq5_ratio);
    assert!(jitter.ddp_ratio >= 0.0, "ddp_ratio={}", jitter.ddp_ratio);
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
}

#[test]
fn report_serializes_to_json() {
    let sample_rate = 16_000;
    let samples = synth_sine(sample_rate, 220.0, 0.5, 0.5);
    let report =
        VoiceAnalyzer::analyze_buffer_in_chunks(AnalyzerConfig::new(sample_rate), &samples, 400);

    let json = serde_json::to_string(&report).expect("report should serialize");
    assert!(json.contains("\"overall\""));
    assert!(json.contains("\"chunks\""));
    assert!(json.contains("\"spectral\""));
}
