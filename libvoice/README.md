# libvoice

`libvoice` is a Rust crate for frame-based voice analysis. It accepts mono PCM samples as `&[f32]`, processes them with overlapping analysis windows, keeps only frames that pass a voiced-frame gate, and returns:

- frame-level measurements for voiced frames
- chunk-level summaries
- an overall summary for the full processed stream

The implementation is centered in [src/analyzer.rs](/home/shione/projects/rust/voicelib/libvoice/src/analyzer.rs), with signal processing in [src/signal.rs](/home/shione/projects/rust/voicelib/libvoice/src/signal.rs), spectral analysis in [src/spectral.rs](/home/shione/projects/rust/voicelib/libvoice/src/spectral.rs), formant extraction in [src/formant.rs](/home/shione/projects/rust/voicelib/libvoice/src/formant.rs), and statistical aggregation in [src/summary.rs](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs).

## Public API

The crate exports:

- `VoiceAnalyzer`
- `AnalyzerConfig`
- `AnalysisReport`
- `ChunkAnalysis`
- `FrameAnalysis`
- `OverallAnalysis`
- `SummaryStats`
- `SpectralSummary`
- `FormantSummary`
- `FormantStats`
- `JitterMetrics`

See [src/lib.rs](/home/shione/projects/rust/voicelib/libvoice/src/lib.rs) and [src/model.rs](/home/shione/projects/rust/voicelib/libvoice/src/model.rs).

## Processing model

`VoiceAnalyzer` keeps a rolling pending buffer and analyzes every full frame of `frame_size` samples with step `hop_size` ([src/analyzer.rs:43](/home/shione/projects/rust/voicelib/libvoice/src/analyzer.rs:43)).

For each frame it computes:

- pitch and pitch clarity
- spectral rolloff, centroid, bandwidth, flatness, tilt
- zero-crossing rate
- RMS, loudness in dBFS, energy
- harmonic-to-noise ratio estimate
- LPC formants with simple temporal tracking

Only frames that satisfy the voiced-frame gate are retained in `frames`, `chunks[*]`, and `overall` ([src/analyzer.rs:139](/home/shione/projects/rust/voicelib/libvoice/src/analyzer.rs:139)). Silence, noise, and unvoiced frames are analyzed internally but dropped from the public summaries.

Voiced-frame gate:

- `pitch_hz.is_some()`
- `pitch_clarity >= pitch_clarity_threshold`
- `rms >= voiced_rms_threshold`
- `spectral_flatness <= voiced_max_spectral_flatness`
- `zcr <= voiced_max_zero_crossing_rate`

This means `frame_count` is the count of voiced frames, not the count of all windows touched by the analyzer.

## AnalyzerConfig

Defined in [src/config.rs](/home/shione/projects/rust/voicelib/libvoice/src/config.rs).

### Core timing

- `sample_rate: u32`
  Input sampling rate in Hz. It also sets FFT bin spacing and formant search limits.
- `frame_size: usize`
  Analysis window length in samples. Default: `2048`.
- `hop_size: usize`
  Step between adjacent frames in samples. Default: `512`.

Effective timing with defaults at `16 kHz`:

- frame duration: `2048 / 16000 = 128 ms`
- frame step: `512 / 16000 = 32 ms`

### Pitch parameters

- `min_pitch_hz: f32`
  Lowest accepted F0 after lag refinement.
- `max_pitch_hz: f32`
  Highest accepted F0 after lag refinement.
- `pitch_clarity_threshold: f32`
  Minimum accepted clarity. The code converts this to a YIN-style CMNDF threshold of `clamp(1.0 - clarity_threshold, 0.05, 0.40)` ([src/signal.rs:83](/home/shione/projects/rust/voicelib/libvoice/src/signal.rs:83)).

### Spectral parameters

- `rolloff_ratio: f32`
  Fraction used for spectral rolloff. The implementation accumulates FFT power until this fraction is reached ([src/spectral.rs:107](/home/shione/projects/rust/voicelib/libvoice/src/spectral.rs:107)).

### Voicing gate parameters

- `voiced_rms_threshold: f32`
  Minimum RMS required for a frame to be considered voiced.
- `voiced_max_spectral_flatness: f32`
  Maximum spectral flatness allowed for a voiced frame.
- `voiced_max_zero_crossing_rate: f32`
  Maximum zero-crossing rate allowed for a voiced frame.

### Formant parameters

- `max_formants: usize`
  Maximum number of tracked formant slots. Also controls LPC order through `lpc_order() = max(max_formants * 2 + 2, 8)` ([src/config.rs:47](/home/shione/projects/rust/voicelib/libvoice/src/config.rs:47)).
- `formant_max_frequency_hz: f32`
  Highest accepted formant frequency. Default is derived from Nyquist, capped at `5500 Hz`, floored at `1500 Hz`.
- `formant_max_bandwidth_hz: f32`
  Maximum accepted formant bandwidth.
- `formant_pre_emphasis_hz: f32`
  Corner-like control used to derive the first-order pre-emphasis coefficient `exp(-2*pi*f/fs)` before LPC analysis ([src/formant.rs:57](/home/shione/projects/rust/voicelib/libvoice/src/formant.rs:57)).

## Algorithms

### Windowing and frame flow

- FFT analysis uses a Hann window from [src/signal.rs:132](/home/shione/projects/rust/voicelib/libvoice/src/signal.rs:132).
- LPC/formant analysis applies its own Hamming window after pre-emphasis ([src/formant.rs:124](/home/shione/projects/rust/voicelib/libvoice/src/formant.rs:124)).
- Partial trailing audio shorter than one full frame is kept in `pending` during streaming and never flushed into a padded last frame.

### Pitch

Pitch estimation lives in [src/signal.rs:22](/home/shione/projects/rust/voicelib/libvoice/src/signal.rs:22).

Implemented steps:

1. Downsample toward `16 kHz` with simple block averaging (`fill_downsampled`).
2. Remove DC by subtracting the frame mean.
3. Compute the YIN difference function over the lag range derived from `min_pitch_hz` and `max_pitch_hz`.
4. Convert it to cumulative mean normalized difference (CMNDF).
5. Pick the first local minimum below the derived threshold, otherwise the global CMNDF minimum in range.
6. Refine the lag by parabolic interpolation.
7. Reject boundary hits with weak clarity.
8. Convert lag to `Hz`.
9. Compute a normalized autocorrelation-based periodicity score for HNR estimation.

Returned pitch values are only exposed for voiced frames.

### Spectral features

Implemented in [src/spectral.rs](/home/shione/projects/rust/voicelib/libvoice/src/spectral.rs).

- `energy`: mean square on the unwindowed frame
- `rms`: `sqrt(energy)`
- `zcr`: sign-change rate on the unwindowed frame
- `spectral_centroid_hz`: magnitude-weighted centroid
- `spectral_bandwidth_hz`: magnitude-weighted standard deviation around the centroid
- `spectral_rolloff_hz`: first bin where cumulative power reaches `rolloff_ratio`
- `spectral_flatness`: geometric mean power divided by arithmetic mean power
- `spectral_tilt_db_per_octave`: power-weighted least-squares slope of `20*log10(magnitude)` versus `log2(frequency)` inside a speech band, after excluding bins more than `40 dB` below the in-band peak
- `loudness_dbfs`: `20*log10(rms)`
- `hnr_db`: `10*log10(periodicity / (1 - periodicity))`

### Formants

Implemented in [src/formant.rs](/home/shione/projects/rust/voicelib/libvoice/src/formant.rs).

Pipeline:

1. Downsample toward `11 kHz`.
2. Remove DC.
3. Apply first-order pre-emphasis.
4. Apply Hamming window.
5. Compute autocorrelation.
6. Solve LPC coefficients with Levinson-Durbin.
7. Find LPC polynomial roots with a Durand-Kerner style iterative root solver.
8. Keep only upper-half-plane roots inside the unit circle.
9. Convert root angle to formant frequency and root radius to bandwidth.
10. Reject candidates outside the configured frequency and bandwidth limits.
11. Sort by frequency, collapse nearby formants, and keep at most `max_formants`.
12. Track formant slots between frames with bounded jump matching.

Tracking is slot-based, not speaker- or phoneme-aware. Missing slots are returned as `0.0` frequency and `0.0` bandwidth in frame output and ignored in summaries.

### Statistical summaries

Implemented in [src/stats.rs](/home/shione/projects/rust/voicelib/libvoice/src/stats.rs) and [src/summary.rs](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs).

`SummaryStats` contains:

- `count`
- `mean`
- `std`
- `median`
- `min`
- `max`
- `p5`
- `p95`

Pitch summaries use extra contour cleanup before aggregation ([src/summary.rs:112](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs:112)):

- collect voiced `pitch_hz` values
- repair one-frame outliers if neighbors agree
- apply median smoothing with radius `2`
- summarize the smoothed contour

Spectral, energy, and formant summaries are direct summaries over retained voiced frames.

## Output structures

### AnalysisReport

- `config`
  The exact configuration used.
- `frames`
  Per-voiced-frame details with cumulative summary after each retained frame.
- `chunks`
  One summary per processed input chunk.
- `overall`
  Summary over all retained voiced frames.

### FrameAnalysis

Defined in [src/model.rs:92](/home/shione/projects/rust/voicelib/libvoice/src/model.rs:92).

- `frame_index`
  Sequential index of retained voiced frames only.
- `start_sample`, `end_sample`
  Sample range of the original analysis window.
- `start_seconds`, `end_seconds`
  Time range from those sample indices.
- `pitch_hz`
  Per-frame F0, or `None` when pitch failed before voice gating. In practice stored frames are voiced, so this is normally `Some`.
- `pitch_clarity`
  `1 - CMNDF(best_lag)`.
- `spectral_rolloff_hz`, `spectral_centroid_hz`, `spectral_bandwidth_hz`, `spectral_flatness`, `spectral_tilt_db_per_octave`
  Spectral descriptors from the FFT frame.
- `zcr`, `rms`, `loudness_dbfs`, `hnr_db`, `energy`
  Time-domain and derived measures.
- `formants_hz`, `formant_bandwidths_hz`
  Tracked formant slots for that frame.
- `cumulative`
  `OverallAnalysis` recalculated after this retained frame.

### ChunkAnalysis and OverallAnalysis

Both contain:

- `frame_count`
  Count of retained voiced frames.
- `pitch_hz`
  Summary of smoothed voiced pitch contour.
- `spectral`
  Summary of spectral descriptors across retained voiced frames.
- `formants`
  Slot-wise formant summaries.
- `energy`
  Summary of frame energy across retained voiced frames.
- `jitter`
  Currently always `None`.

`ChunkAnalysis` also has:

- `chunk_index`
- `input_samples`

`OverallAnalysis` also has:

- `processed_samples`

## Files

- [Cargo.toml](/home/shione/projects/rust/voicelib/libvoice/Cargo.toml)
  Crate manifest. Uses `realfft`, `rustfft`, and `serde`.
- [src/lib.rs](/home/shione/projects/rust/voicelib/libvoice/src/lib.rs)
  Module wiring, re-exports, and basic unit tests.
- [src/analyzer.rs](/home/shione/projects/rust/voicelib/libvoice/src/analyzer.rs)
  Streaming analyzer, chunk handling, voiced-frame gating, and report construction.
- [src/config.rs](/home/shione/projects/rust/voicelib/libvoice/src/config.rs)
  User-facing configuration and defaults.
- [src/model.rs](/home/shione/projects/rust/voicelib/libvoice/src/model.rs)
  Public report and summary structs.
- [src/signal.rs](/home/shione/projects/rust/voicelib/libvoice/src/signal.rs)
  Pitch analysis, loudness/HNR helpers, zero-crossing, and downsampling helpers.
- [src/spectral.rs](/home/shione/projects/rust/voicelib/libvoice/src/spectral.rs)
  FFT-based feature extraction and integration with pitch and formants.
- [src/formant.rs](/home/shione/projects/rust/voicelib/libvoice/src/formant.rs)
  LPC-based formant estimation and slot tracker.
- [src/stats.rs](/home/shione/projects/rust/voicelib/libvoice/src/stats.rs)
  Generic summary statistics.
- [src/summary.rs](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs)
  Construction of chunk and overall summaries.
- [tests/voice_analysis.rs](/home/shione/projects/rust/voicelib/libvoice/tests/voice_analysis.rs)
  Integration tests for pitch, streaming consistency, spectral values, formants, and serialization.

## Issues found in the current implementation

### 1. Jitter is modeled but never implemented

`JitterMetrics` is a public type, and `ChunkAnalysis` / `OverallAnalysis` expose `jitter`, but the code always sets it to `None` ([src/model.rs:43](/home/shione/projects/rust/voicelib/libvoice/src/model.rs:43), [src/summary.rs:24](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs:24), [src/summary.rs:45](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs:45)). If users expect jitter measurements, the current implementation is incomplete.

### 2. “Overall” and “chunk” metrics are voice-only, not whole-signal metrics

The analyzer discards every frame that fails the voiced-frame gate before building summaries ([src/analyzer.rs:59](/home/shione/projects/rust/voicelib/libvoice/src/analyzer.rs:59)). As a result:

- silence does not contribute zero energy
- noisy or unvoiced regions do not contribute spectral statistics
- `frame_count` is not the number of analyzed windows

That behavior is consistent in code and tests, but it is easy to misread as full-buffer statistics.

### 3. Pitch summary smoothing ignores time gaps

Pitch summary code drops unvoiced gaps and then smooths only the remaining voiced values ([src/summary.rs:116](/home/shione/projects/rust/voicelib/libvoice/src/summary.rs:116)). If two voiced regions are separated by silence or noise, the median smoothing and outlier repair can treat those regions as adjacent in time. This affects summary pitch statistics, not per-frame pitch values.

## Validation status

The current automated tests pass with:

```bash
cargo test -p libvoice
```
