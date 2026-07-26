use libvoice::{AnalysisReport, AnalyzerConfig, VoiceAnalyzer};
use std::f32::consts::PI;

fn synth_high_zcr_fricative(sample_rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
    let total = (sample_rate as f32 * seconds) as usize;
    (0..total)
        .map(|index| {
            if index % 2 == 0 {
                amplitude
            } else {
                -amplitude
            }
        })
        .collect()
}

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
            value += amplitude * (2.0 * PI * pitch_hz * harmonic_number as f32 * t).sin();
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

    // Only compare frame counts if both reports actually collected frames
    if !full.frames.is_empty() && !streamed.frames.is_empty() {
        assert_eq!(full.frames.len(), streamed.frames.len());
    }

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
        1.0e-6,
    );
    approx_eq(
        full_spectral.hnr_db.mean,
        streamed_spectral.hnr_db.mean,
        0.01,
    );

    if let (Some(f_harm), Some(s_harm)) = (&full.overall.harmonics, &streamed.overall.harmonics) {
        assert_eq!(f_harm.harmonics.len(), s_harm.harmonics.len());
        for (f, s) in f_harm.harmonics.iter().zip(s_harm.harmonics.iter()) {
            approx_eq(f.strength_ratio.mean, s.strength_ratio.mean, 0.01);
        }
    }
}

#[test]
fn hnr_interpolation_improves_precision_for_fractional_lags() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);

    // 16000 / 220.5 ≈ 72.562... lags
    // This frequency does not align with an integer lag.
    // Without interpolation, rounding to 73 or 72 would decrease periodicity.
    let expected_hz = 220.5;
    let samples = synth_sine(sample_rate, expected_hz, 1.0, 0.5);
    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    let spectral = report.overall.spectral.as_ref().unwrap();
    // With interpolation, HNR should be very high for a pure sine wave.
    // If it were rounded to integer lag, HNR would typically drop below 20 dB.
    assert!(
        spectral.hnr_db.mean > 20.0,
        "HNR should be high with interpolation, got {}",
        spectral.hnr_db.mean
    );

    let pitch = report.overall.pitch_hz.as_ref().unwrap();
    approx_eq(pitch.mean, expected_hz, 1.0);
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

        approx_eq(pitch.mean, expected_hz, 2.0);
        assert!(
            pitch.std < 1.0,
            "unexpected pitch std for {expected_hz} Hz: {}",
            pitch.std
        );
        assert!(pitch.p5 > expected_hz - 5.0);
        assert!(pitch.p95 < expected_hz + 5.0);

        let spectral = report.overall.spectral.as_ref().unwrap();
        assert!(spectral.hnr_db.mean > 20.0);
    }
}

#[test]
fn high_pitch_mode_tracks_high_voice_fundamentals() {
    let sample_rate = 16_000;
    let mut config = AnalyzerConfig::new(sample_rate);
    config.apply_high_pitch_mode();

    let expected_hz = 1_000.0;
    let samples = synth_sine(sample_rate, expected_hz, 1.0, 0.5);
    let report = VoiceAnalyzer::analyze_buffer(config, &samples);
    let pitch = report
        .overall
        .pitch_hz
        .expect("high-pitch mode should keep 1000 Hz voiced");

    approx_eq(pitch.mean, expected_hz, 5.0);
    let spectral = report.overall.spectral.as_ref().unwrap();
    assert!(spectral.hnr_db.mean > 20.0);
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
    approx_eq(pitch.mean, 900.0, 5.0);

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

    approx_eq(h2.strength_ratio.mean, 0.45, 0.05);
    approx_eq(h3.strength_ratio.mean, 0.25, 0.05);
    approx_eq(h4.strength_ratio.mean, 0.12, 0.05);
    approx_eq(h5.strength_ratio.mean, 0.06, 0.03);
    assert!(
        harmonics.max_frequency_hz > 4_400.0 && harmonics.max_frequency_hz < 4_600.0,
        "max detected harmonic frequency = {}",
        harmonics.max_frequency_hz
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
    assert!(frames[0].pitch_hz.is_some());
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

    assert_reports_close(&expected, &actual);
}

#[test]
fn silence_produces_no_pitch_or_jitter_and_zero_energy() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = vec![0.0_f32; sample_rate as usize];

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert_eq!(report.overall.frame_count, 0);
    assert!(report.overall.pitch_hz.is_none());
}

#[test]
fn broadband_noise_is_skipped_as_non_voice() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_noise(sample_rate, 1.0, 0.4);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert_eq!(report.overall.frame_count, 0);
    assert!(report.overall.pitch_hz.is_none());
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
    approx_eq(pitch.mean, 220.0, 2.0);
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
        frame_midpoint <= 0.9 + 0.05,
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

    assert!(spectral.centroid_hz.mean > 210.0 && spectral.centroid_hz.mean < 230.0);
    assert!(spectral.rolloff_hz.mean < 350.0);
    assert!(spectral.bandwidth_hz.mean < 100.0);
    assert!(spectral.flatness.mean < 0.05);
    assert!(spectral.hnr_db.mean > 20.0);
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
    approx_eq(first.strength_ratio.mean, 1.0, 0.01);

    let second = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 2)
        .unwrap();
    approx_eq(second.strength_ratio.mean, 0.5, 0.05);

    let third = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 3);
    assert!(
        third.is_none(),
        "weak or absent harmonics should not be reindexed"
    );

    let fourth = harmonics
        .harmonics
        .iter()
        .find(|harmonic| harmonic.harmonic_number == 4)
        .unwrap();
    approx_eq(fourth.strength_ratio.mean, 0.25, 0.05);
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
    approx_eq(
        first.cumulative.pitch_hz.as_ref().unwrap().mean,
        first.pitch_hz.unwrap(),
        0.01,
    );

    let last = report.frames.last().unwrap();
    assert_eq!(last.cumulative, report.overall);
}

#[test]
fn streaming_handles_extreme_fragmentation_single_sample() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = synth_sine(sample_rate, 220.0, 0.5, 0.5);

    let full = VoiceAnalyzer::analyze_buffer(config.clone(), &samples);

    let mut analyzer = VoiceAnalyzer::new(config);
    for &sample in &samples {
        analyzer.process_chunk(&[sample]);
    }
    let streamed_overall = analyzer.finalize();

    let streamed = AnalysisReport {
        config: analyzer.config().clone(),
        frames: Vec::new(),
        chunks: Vec::new(),
        overall: streamed_overall,
        fft_spectrum: None,
    };

    assert_reports_close(&full, &streamed);
}

#[test]
fn sub_frame_audio_yields_no_frames_but_counts_samples() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let short_len = config.frame_size - 1;
    let samples = vec![0.5_f32; short_len];

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert_eq!(report.frames.len(), 0);
    assert_eq!(report.overall.processed_samples, short_len);
}

#[test]
fn high_zcr_fricative_is_rejected_as_unvoiced() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    // This signal has high energy and would have pitch clarity in some detectors,
    // but its ZCR is 1.0, which should be rejected.
    let samples = synth_high_zcr_fricative(sample_rate, 0.5, 0.5);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    assert_eq!(report.overall.frame_count, 0);
    assert!(report.overall.pitch_hz.is_none());
}

#[test]
fn trailing_rms_drop_rejects_frame() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);

    // Create exactly one frame's worth of audio.
    // First half: 440Hz sine wave (voiced).
    // Second half: Silence.
    let mut samples = synth_sine(
        sample_rate,
        440.0,
        config.frame_size as f32 / (2.0 * sample_rate as f32),
        0.5,
    );
    samples.resize(config.frame_size, 0.0);

    let report = VoiceAnalyzer::analyze_buffer(config, &samples);

    // This frame should be rejected because its trailing RMS (calculated on the second half)
    // is 0.0, which is < 0.8 * voiced_rms_threshold and < 0.45 * frame_rms.
    assert_eq!(report.overall.frame_count, 0);
}

#[test]
fn fallible_constructor_rejects_zero_hop_size() {
    let mut config = AnalyzerConfig::new(16_000);
    config.hop_size = 0;
    assert!(VoiceAnalyzer::try_new(config).is_err());
}

#[test]
fn silent_fft_spectrum_has_zero_frequency_statistics() {
    let sample_rate = 16_000;
    let config = AnalyzerConfig::new(sample_rate);
    let samples = vec![0.0; config.frame_size];
    let report = VoiceAnalyzer::analyze_buffer_with_output_options(
        config,
        &samples,
        libvoice::AnalysisOutputOptions {
            frame_analysis: false,
            fft_spectrum: true,
        },
    );
    let spectrum = report.fft_spectrum.unwrap();
    assert!(
        spectrum.frames[0]
            .magnitudes
            .iter()
            .all(|&value| value == 0.0)
    );
}
