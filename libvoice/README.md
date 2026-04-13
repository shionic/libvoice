# libvoice

`libvoice` is a Rust library for frame-based voice analysis. It accepts mono `f32` PCM samples, filters out frames that do not look voiced, and reports pitch, spectral statistics, formants, frame-level results, chunk summaries, overall summaries, and optional FFT magnitudes.

## 1) Public API, List of Functions, Default Settings

### Public Types

- `VoiceAnalyzer`: streaming analyzer state.
- `AnalysisOutputOptions`: optional extra outputs. Currently only `fft_spectrum`.
- `AnalyzerConfig`: runtime analysis settings.
- `AnalysisReport`: full result returned by one-shot helpers.
- `FrameAnalysis`: one voiced frame plus cumulative stats up to that frame.
- `ChunkAnalysis`: summary for one input chunk.
- `OverallAnalysis`: summary across all voiced frames seen so far.
- `SummaryStats`: `count`, `mean`, `std`, `median`, `min`, `max`, `p5`, `p95`.
- `SpectralSummary`: rolloff, centroid, bandwidth, flatness, tilt, ZCR, RMS, loudness, HNR.
- `FormantSummary`: optional `f1`..`f4`.
- `FormantStats`: summary for one tracked formant slot.
- `FftSpectrum`: optional FFT output for each processed frame.
- `FftSpectrumFrame`: one FFT frame with time bounds, voiced flag, and magnitudes.
- `JitterMetrics`: public data model only. Current implementation never populates it.

### Public Functions and Methods

#### `AnalyzerConfig`

- `AnalyzerConfig::new(sample_rate: u32) -> AnalyzerConfig`
  Creates a config with sensible defaults for the given sample rate.
- `AnalyzerConfig::default() -> AnalyzerConfig`
  Equivalent to `AnalyzerConfig::new(16_000)`.
- `frame_step_seconds(&self) -> f32`
  Returns `hop_size / sample_rate`.

#### `VoiceAnalyzer`

- `VoiceAnalyzer::new(config: AnalyzerConfig) -> VoiceAnalyzer`
  Creates a streaming analyzer with default output options.
- `VoiceAnalyzer::new_with_output_options(config, output_options) -> VoiceAnalyzer`
  Creates a streaming analyzer and optionally enables FFT export.
- `config(&self) -> &AnalyzerConfig`
  Returns the active config.
- `process_chunk(&mut self, samples: &[f32]) -> ChunkAnalysis`
  Feeds one chunk and returns a summary for voiced frames found in that chunk.
- `process_chunk_with_frames(&mut self, samples: &[f32]) -> (ChunkAnalysis, Vec<FrameAnalysis>)`
  Same as `process_chunk`, but also returns every voiced frame emitted from that chunk.
- `finalize(&self) -> OverallAnalysis`
  Returns the cumulative summary across all voiced frames processed so far.
- `VoiceAnalyzer::analyze_buffer(config, samples) -> AnalysisReport`
  One-shot whole-buffer analysis.
- `VoiceAnalyzer::analyze_buffer_with_output_options(config, samples, output_options) -> AnalysisReport`
  Whole-buffer analysis with optional FFT export.
- `VoiceAnalyzer::analyze_buffer_in_chunks(config, samples, input_chunk_size) -> AnalysisReport`
  Simulates streaming by feeding the buffer in fixed-size chunks.
- `VoiceAnalyzer::analyze_buffer_in_chunks_with_output_options(config, samples, input_chunk_size, output_options) -> AnalysisReport`
  Same as above, with optional FFT export.

### Default Settings

`AnalyzerConfig::new(sample_rate)` sets:

| Field | Default |
| --- | --- |
| `sample_rate` | caller-provided |
| `frame_size` | `2048` samples |
| `hop_size` | `512` samples |
| `min_pitch_hz` | `60.0` Hz |
| `max_pitch_hz` | `500.0` Hz |
| `pitch_clarity_threshold` | `0.60` |
| `rolloff_ratio` | `0.85` |
| `voiced_rms_threshold` | `0.015` |
| `voiced_max_spectral_flatness` | `0.45` |
| `voiced_max_zero_crossing_rate` | `0.25` |
| `max_formants` | `4` |
| `formant_max_frequency_hz` | `clamp(min(sample_rate / 2 - 50, 5500), 1500, 5500)` |
| `formant_max_bandwidth_hz` | `700.0` Hz |
| `formant_pre_emphasis_hz` | `50.0` Hz |

`AnalysisOutputOptions::default()` sets:

- `fft_spectrum: false`

For the default `16_000` Hz config:

- frame length = `2048 / 16000 = 128 ms`
- frame step = `512 / 16000 = 32 ms`
- FFT bin spacing = `16000 / 2048 = 7.8125 Hz`
- default `formant_max_frequency_hz = 5500 Hz`

## 2) Recommendations for Default Settings

- Start with `AnalyzerConfig::new(16_000)` unless the recording is already stored at a different rate. The implementation is tested at `16 kHz` and also for formant stability at `48 kHz`.
- Keep `frame_size = 2048` and `hop_size = 512` for speech. The code and tests assume this scale, and the optional FFT output is also validated with these values.
- Keep the default pitch range `60..500 Hz` for general voice analysis. It covers the test cases (`110`, `140`, `180`, `205`, `220`, `240`, `320 Hz`) and most adult speech and singing fundamentals.
- Raise `max_pitch_hz` only if you expect very high singing or child speech. Lower it if octave errors above the expected range matter more than recall.
- Do not lower `pitch_clarity_threshold` below `0.60` unless you want more borderline voiced frames. This threshold is used twice: pitch acceptance in the YIN-like detector and final voiced-frame gating.
- Keep `voiced_rms_threshold = 0.015`, `voiced_max_spectral_flatness = 0.45`, and `voiced_max_zero_crossing_rate = 0.25` together. In the current implementation, they are the main protection against silence and broadband noise entering the voiced summaries.
- Leave `max_formants = 4` unless you have a strong reason to expose more LPC poles. The public summary only has slots `f1` through `f4`.
- Keep `formant_max_bandwidth_hz = 700` for speech-like vowels. Narrow resonances survive; wide, unstable poles are rejected.
- Enable `AnalysisOutputOptions { fft_spectrum: true }` only when you actually need per-frame FFT magnitudes. It increases result size substantially.
- Treat `jitter` as unavailable for now. The field exists in the report schema, but the current library always returns `None`.

## 3) List of All Features

### Streaming and One-Shot Analysis

What it is:
Analyze either a full sample buffer at once or incrementally as chunks arrive.

Algorithm:
The analyzer stores pending samples, emits a frame whenever `pending_start + frame_size <= pending.len()`, advances by `hop_size`, keeps overlap in memory, and compacts the pending buffer after processing.

Boundary values:
- `input_chunk_size` is clamped with `.max(1)` in the chunked helper.
- Frames are emitted only when at least `frame_size` samples are available.
- Incomplete trailing audio is ignored.

Known typical values:
- Tests use irregular chunks such as `13`, `97`, `257`, `317`, `509`, `701`, `1024`.
- A 1-second signal at `16 kHz` with default settings produces voiced frames spaced every `32 ms`.

### Voiced-Frame Filtering

What it is:
Only frames that look voiced contribute to `frames`, `chunks`, and `overall` summaries.

Algorithm:
A frame is accepted only if all of these hold:
- `pitch_hz.is_some()`
- `pitch_clarity >= pitch_clarity_threshold`
- `rms >= voiced_rms_threshold`
- `spectral_flatness <= voiced_max_spectral_flatness`
- `zcr <= voiced_max_zero_crossing_rate`

Boundary values:
- Defaults are `clarity >= 0.60`, `rms >= 0.015`, `flatness <= 0.45`, `zcr <= 0.25`.
- Silence and broadband noise are expected to produce zero voiced frames.

Known typical values:
- In tests, silence yields `frame_count = 0`.
- White-noise-like input at amplitude `0.4` is rejected as non-voice.
- A stable sine at amplitude `0.5` passes cleanly.

### Pitch Detection

What it is:
Per-frame fundamental-frequency estimation plus a clarity score.

Algorithm:
The implementation is YIN-like:
1. Downsample toward `16 kHz`.
2. Remove DC.
3. Compute the difference function over allowed lags.
4. Convert it to CMNDF.
5. Pick the first local minimum below the derived threshold, or the global minimum if needed.
6. Refine the lag parabolically.
7. Reject low-clarity and near-boundary weak candidates.

Boundary values:
- Search range is `[min_pitch_hz, max_pitch_hz]`, default `60..500 Hz`.
- Internal YIN threshold is derived from clarity and clamped to `0.05..0.40`.
- Very short reduced frames (`< 3` samples, or not enough lags) return no pitch.

Known typical values:
- Tested stable tones: `110`, `180`, `220`, `320 Hz`.
- Streaming tests also use `140`, `205`, `240 Hz`.
- For a stable sine, mean pitch is expected within about `12 Hz`, with low standard deviation.

### Pitch Post-Processing for Summaries

What it is:
The summary pitch statistics are slightly cleaned before computing `mean`, `median`, percentiles, and standard deviation.

Algorithm:
The raw voiced pitch contour is:
1. Repaired for isolated outliers if both neighboring jumps exceed `18%` but the bridge jump is below `8%`.
2. Median-smoothed with radius `2`.

Boundary values:
- No smoothing is applied when fewer than `3` pitch values are available.

Known typical values:
- This is intended to stabilize summary statistics for otherwise steady voiced segments.

### Spectral Rolloff

What it is:
Frequency below which a chosen fraction of spectral power has accumulated.

Algorithm:
After FFT magnitude calculation, cumulative power is tracked until it reaches `power_sum * rolloff_ratio`.

Boundary values:
- Default `rolloff_ratio = 0.85`.
- If no earlier bin reaches the threshold, rolloff falls back to the last FFT bin.

Known typical values:
- For a clean `220 Hz` sine, tests expect mean rolloff below `500 Hz`.

### Spectral Centroid

What it is:
Magnitude-weighted center of mass of the spectrum.

Algorithm:
`sum(hz * magnitude) / sum(magnitude)`.

Boundary values:
- Returns `0.0` when total magnitude is zero.

Known typical values:
- For a clean `220 Hz` sine, tests expect centroid roughly between `180` and `350 Hz`.

### Spectral Bandwidth

What it is:
Spread of the magnitude spectrum around the centroid.

Algorithm:
Square root of the magnitude-weighted second central moment around the centroid.

Boundary values:
- Returns `0.0` when total magnitude is zero.

Known typical values:
- For a clean `220 Hz` sine, tests expect bandwidth below `250 Hz`.

### Spectral Flatness

What it is:
A tonal-vs-noisy measure derived from the FFT power spectrum.

Algorithm:
Geometric mean of power divided by arithmetic mean of power.

Boundary values:
- Approximately `0` for peaky spectra.
- Approaches `1` for flat/noise-like spectra.
- Used directly in voiced gating with default maximum `0.45`.

Known typical values:
- For a clean sine, tests expect flatness below `0.1`.
- Broadband noise is expected to exceed the voiced limit often enough to be rejected.

### Spectral Tilt

What it is:
Slope of spectral level versus log-frequency, reported in `dB/octave`.

Algorithm:
Weighted linear regression over FFT bins:
1. Use only bins between `80 Hz` and `min(5000 Hz, Nyquist)`.
2. Ignore bins more than `40 dB` below the in-band peak power.
3. Regress `20*log10(magnitude)` on `log2(frequency)` with power weights.

Boundary values:
- Minimum analysis frequency: `80 Hz`.
- Maximum analysis frequency: `5000 Hz` or Nyquist, whichever is lower.
- Requires at least `3` selected bins; otherwise returns `0.0`.

Known typical values:
- A flat synthetic spectrum gives near `0 dB/octave`.
- A decaying synthetic spectrum gives a negative tilt.

### Zero-Crossing Rate (ZCR)

What it is:
Fraction of adjacent sample pairs that cross zero.

Algorithm:
Count sign changes and divide by `frame.len() - 1`.

Boundary values:
- Returns `0.0` for frames shorter than `2` samples.
- Used in voiced gating with default maximum `0.25`.

Known typical values:
- A `220 Hz` sine at `16 kHz` is roughly `0.0275`.
- Noise tends to be much higher.

### Energy, RMS, and Loudness

What it is:
Basic level measurements.

Algorithm:
- `energy`: mean square of the raw frame.
- `rms`: square root of energy.
- `loudness_dbfs`: `20 * log10(max(rms, 1e-12))`.

Boundary values:
- Silent frames have zero energy before voiced gating removes them.
- `loudness_dbfs` is floored numerically at `1e-12` RMS to avoid `-inf`.

Known typical values:
- A sine with amplitude `0.5` has energy near `0.125` and RMS near `0.354`.
- Mixed-signal tests expect voiced energy mean above `0.05`.

### Harmonics-to-Noise Ratio (HNR)

What it is:
A harmonicity estimate derived from pitch periodicity.

Algorithm:
Normalized autocorrelation at the chosen lag is converted to:
`10 * log10(periodicity / (1 - periodicity))`

Boundary values:
- Periodicity `<= 0` returns `0 dB`.
- Periodicity is clamped to `[1e-6, 0.999]` before conversion.

Known typical values:
- Strongly periodic voiced frames produce positive HNR.
- Unvoiced or rejected frames do not enter summaries.

### Formant Estimation

What it is:
Per-frame LPC-based estimation of vocal-tract resonances.

Algorithm:
1. Downsample toward `11 kHz`.
2. Remove DC.
3. Apply first-order pre-emphasis.
4. Apply a Hamming window.
5. Compute autocorrelation.
6. Solve LPC coefficients with Levinson-Durbin.
7. Find LPC polynomial roots.
8. Keep upper-half-plane roots inside the unit circle.
9. Convert roots to frequency and bandwidth.
10. Reject implausible poles and collapse near-duplicates.

Boundary values:
- Minimum kept formant frequency: `90 Hz`.
- Maximum frequency: `min(formant_max_frequency_hz, Nyquist - 50 Hz)`.
- Maximum bandwidth: default `700 Hz`.
- `lpc_order = max(max_formants * 2 + 2, 8)`.
- Nearby candidates are merged when spacing is below `max(60 Hz, 20% of the narrower bandwidth)`.

Known typical values:
- Vowel-like tests target `F1 ≈ 730 Hz`, `F2 ≈ 1090 Hz`.
- The implementation is tested to keep similar `F1/F2` values at `16 kHz` and `48 kHz`.

### Formant Tracking Across Frames

What it is:
Stabilizes `F1`..`F4` slot assignment over time.

Algorithm:
Each slot first tries to match the closest candidate near the previous slot value. Remaining candidates are inserted into empty slots in ascending-frequency order. Unmatched tracked slots can survive for a limited number of missed frames.

Boundary values:
- Maximum relative jump: `22%`.
- Minimum absolute jump allowance: `180 Hz`.
- A slot is dropped after `6` consecutive misses.
- Output is capped at `max_formants`, default `4`.

Known typical values:
- Stable vowel-like test material yields persistent `F1` and `F2` tracks.

### Frame-Level Output

What it is:
Every accepted voiced frame can be returned with its own measurements and cumulative totals up to that frame.

Algorithm:
When a frame passes voiced gating, the library stores its raw per-frame values and recomputes cumulative overall summaries using all voiced frames seen so far.

Boundary values:
- Only voiced frames are emitted.
- `frame_index` increases only for voiced frames, not for all FFT windows.

Known typical values:
- Tests expect `frames.len() == overall.frame_count`.
- The first frame has cumulative `frame_count = 1`.
- The last frame’s cumulative summary matches `overall`.

### Chunk and Overall Statistical Summaries

What it is:
Aggregated statistics for voiced frames within one chunk and across the full stream.

Algorithm:
For each metric, finite values are collected, sorted, and summarized into `mean`, `std`, `median`, `min`, `max`, `p5`, `p95`.

Boundary values:
- Empty frame sets produce `None` summaries.
- `jitter` is always `None` in the current implementation.

Known typical values:
- Silence and noise-only input produce `None` for pitch, spectral, formants, and energy.
- Stable voiced input produces dense summaries with low pitch spread.

### Optional FFT Spectrum Export

What it is:
Raw FFT magnitude vectors for every processed analysis frame.

Algorithm:
If `AnalysisOutputOptions.fft_spectrum` is enabled, the analyzer stores the current frame’s FFT magnitudes together with time bounds and the voiced/non-voiced decision.

Boundary values:
- Disabled by default.
- Returns `None` if no FFT frames were captured.
- `bin_hz = sample_rate / frame_size`.

Known typical values:
- With the default config at `16 kHz`, `frame_size = 2048`, `hop_size = 512`, `bin_hz = 7.8125 Hz`.
- Tests expect a `220 Hz` tone to peak near the corresponding FFT bin, within about `20 Hz`.

### Serialization

What it is:
Configuration and result types derive `Serialize` and `Deserialize`.

Algorithm:
Serde derives are attached directly to the public model types.

Boundary values:
- Serialization preserves optional sections such as `spectral`, `formants`, and `fft_spectrum`.

Known typical values:
- Tests verify JSON output contains `overall`, `frames`, `chunks`, `spectral`, `tilt_db_per_octave`, and `formants`.

### Current Non-Feature: Jitter

What it is:
The public schema contains `JitterMetrics`, but analysis does not compute it yet.

Algorithm:
No algorithm is currently wired in; all summary constructors set `jitter: None`.

Boundary values:
- Always `None`.

Known typical values:
- All current tests expect missing jitter on silence, and no test exercises populated jitter values.
