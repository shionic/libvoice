Jitter / Vibrato Notes

Context

`libvoice` originally reported frame-based jitter and vibrato-style metrics derived from voiced-frame F0 contours. Those values turned out to be unreliable on real speech and were removed from active reporting.

Why It Was Removed

- Frame-to-frame F0 deltas are not a valid replacement for cycle-to-cycle period perturbation.
- On real interview speech, the frame-based method produced numbers that were sometimes plausible in scale but not stable or trustworthy.
- A pulse-based implementation was attempted next, but the pulse picker was not robust enough and produced grossly inflated jitter values.

Reference Comparison Used

File used repeatedly for comparison:

- `/media/data/experiment/voiceanalyzer/data/ariana.wav`

Parselmouth / Praat reference measured locally on that file with pitch floor `60 Hz` and ceiling `500 Hz`:

- `jitter_local = 0.021974484892029255` (`2.20%`)
- `jitter_local_abs = 0.00011380457825794654` (`113.8 us`)
- `jitter_rap = 0.010044417133372516` (`1.00%`)
- `jitter_ppq5 = 0.010214310068396912` (`1.02%`)
- `jitter_ddp = 0.03013325140011755` (`3.01%`)
- `hnr_db = 12.016178854789906`

What Worked

- HNR was improved substantially and matched Praat closely enough on the Ariana file.
- Current HNR is based on detector periodicity rather than the earlier optimistic lag-energy shortcut.

What Was Tried For Jitter

1. Frame-F0 perturbation

- Used voiced-frame pitch contour only.
- Computed local frame-to-frame Hz and ratio changes.
- Produced values in the right rough order sometimes, but conceptually wrong and too dependent on hop size and contour smoothing.

2. Smoothed contour / stable-run analysis

- Tried median smoothing, outlier repair, and longest stable run logic.
- Helped suppress obvious octave slips.
- Did not make jitter valid in a Praat-like sense.

3. Period-based metrics from frame periods

- Replaced Hz deltas with period deltas derived from frame pitch.
- Added `local`, `local_abs`, `rap`, `ppq5`, `ddp`.
- Still not trustworthy because frame periods are not pulse periods.

4. Pulse-based jitter attempt

- Added raw frame carry-through and frame timing.
- Extracted pulse candidates from voiced frame waveforms using amplitude-envelope peak picking constrained by expected period.
- Merged nearby pulse candidates across overlapping voiced frames.
- Computed perturbation metrics from resulting pulse intervals.

Why The Pulse Attempt Failed

- Pulse picking locked onto generic amplitude peaks rather than reliable glottal epochs.
- On Ariana speech, sample count became comparable to Praat, but perturbation values blew up badly:
  - local around `15.7%`
  - local_abs around `716 us`
  - RAP / PPQ5 / DDP far above Praat
- Reliability gating was attempted, but the gate was not strong enough to reject the bad pulse train consistently without also rejecting benign periodic cases.

Code Paths Touched During The Attempt

These areas were involved during the experimentation:

- `libvoice/src/summary.rs`
- `libvoice/src/signal.rs`
- `libvoice/src/spectral.rs`
- `libvoice/src/analyzer.rs`
- `libvoice/src/model.rs`
- `voiceanalyzercli/src/main.rs`

The final state after cleanup is:

- jitter removed from reporting
- vibrato estimate removed from reporting
- HNR retained

Recommended Next Attempt

Do not resume from frame-delta jitter. Start with a proper pulse / epoch detector.

Suggested direction:

1. Keep the current frame-based voiced segmentation and pitch summary.
2. Inside voiced spans only, build a dedicated pulse detector using one of:
   - LPC residual peaks
   - center-clipped waveform + normalized autocorrelation
   - DYPSA-like / epoch-style detector
   - band-limited excitation emphasis before pulse search
3. Search pulses around predicted period, not generic envelope maxima.
4. Reject half-period / double-period candidates aggressively.
5. Only emit jitter when pulse-quality diagnostics pass.

Minimum quality checks before exposing jitter again:

- pulse interval median agrees with frame-period median
- low rate of interval outliers
- low duplicate-pulse rate in overlapping frames
- perturbation metrics stable across neighboring voiced spans

Pragmatic Recommendation

Until a real pulse detector is implemented, do not expose jitter in `libvoice`.

