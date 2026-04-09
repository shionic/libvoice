use crate::config::AnalyzerConfig;
use crate::model::FormantFrame;
use crate::signal::fill_downsampled;
use rustfft::num_complex::Complex32;
use std::f32::consts::PI;

const TARGET_FORMANT_SAMPLE_RATE: u32 = 11_000;
const MIN_FORMANT_FREQUENCY_HZ: f32 = 90.0;
const MIN_FORMANT_SPACING_HZ: f32 = 60.0;
const FORMANT_TRACK_MAX_RELATIVE_JUMP: f32 = 0.22;
const FORMANT_TRACK_MIN_ABSOLUTE_JUMP_HZ: f32 = 180.0;
const FORMANT_TRACK_MAX_MISSES: usize = 6;

#[derive(Debug, Default)]
pub(crate) struct FormantAnalyzer {
    reduced: Vec<f32>,
    autocorrelation: Vec<f32>,
    lpc: Vec<f32>,
    roots: Vec<Complex32>,
    formants: Vec<FormantFrame>,
}

#[derive(Debug, Default)]
pub(crate) struct FormantTracker {
    slots: Vec<Option<TrackedFormant>>,
}

#[derive(Debug, Clone)]
struct TrackedFormant {
    formant: FormantFrame,
    missed_frames: usize,
}

impl FormantAnalyzer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn estimate(&mut self, frame: &[f32], config: &AnalyzerConfig) -> Vec<FormantFrame> {
        let downsample = (config.sample_rate / TARGET_FORMANT_SAMPLE_RATE).max(1) as usize;
        let reduced_len = frame.len() / downsample;
        let order = config.lpc_order();
        if reduced_len <= order + 2 {
            return Vec::new();
        }
        let effective_sample_rate = config.sample_rate / downsample as u32;

        self.reduced.resize(reduced_len, 0.0);
        let reduced = &mut self.reduced[..reduced_len];
        fill_downsampled(frame, downsample, reduced);

        let mean = reduced.iter().copied().sum::<f32>() / reduced_len as f32;
        for sample in reduced.iter_mut() {
            *sample -= mean;
        }

        let pre_emphasis =
            (-2.0 * PI * config.formant_pre_emphasis_hz / effective_sample_rate as f32).exp();
        let mut previous = 0.0_f32;
        for sample in reduced.iter_mut() {
            let current = *sample;
            *sample = current - pre_emphasis * previous;
            previous = current;
        }

        apply_hamming_window(reduced);

        self.autocorrelation.resize(order + 1, 0.0);
        for lag in 0..=order {
            let mut value = 0.0_f32;
            for index in 0..(reduced_len - lag) {
                value += reduced[index] * reduced[index + lag];
            }
            self.autocorrelation[lag] = value;
        }
        let residual = levinson_durbin(&self.autocorrelation, order, &mut self.lpc);
        if residual <= 1.0e-9 {
            return Vec::new();
        }

        self.roots.clear();
        find_polynomial_roots(&self.lpc, &mut self.roots);

        let nyquist = effective_sample_rate as f32 * 0.5;
        let max_frequency = config.formant_max_frequency_hz.min(nyquist - 50.0).max(0.0);

        self.formants.clear();
        self.formants.extend(self.roots.iter().filter_map(|root| {
            if root.im <= 0.0 {
                return None;
            }

            let radius = root.norm();
            if !(0.0..1.0).contains(&radius) {
                return None;
            }

            let frequency_hz = root.arg() * effective_sample_rate as f32 / (2.0 * PI);
            if frequency_hz < MIN_FORMANT_FREQUENCY_HZ || frequency_hz > max_frequency {
                return None;
            }

            let bandwidth_hz = -(effective_sample_rate as f32 / PI) * radius.ln();
            if !bandwidth_hz.is_finite()
                || bandwidth_hz <= 0.0
                || bandwidth_hz > config.formant_max_bandwidth_hz
            {
                return None;
            }

            Some(FormantFrame {
                frequency_hz,
                bandwidth_hz,
            })
        }));

        self.formants
            .sort_by(|left, right| left.frequency_hz.total_cmp(&right.frequency_hz));
        collapse_nearby_formants(&mut self.formants, config.max_formants);
        self.formants.clone()
    }
}

fn apply_hamming_window(samples: &mut [f32]) {
    let denom = (samples.len().saturating_sub(1)).max(1) as f32;
    for (index, sample) in samples.iter_mut().enumerate() {
        let window = 0.54 - 0.46 * (2.0 * PI * index as f32 / denom).cos();
        *sample *= window;
    }
}

fn collapse_nearby_formants(formants: &mut Vec<FormantFrame>, max_formants: usize) {
    if formants.is_empty() {
        return;
    }

    let mut filtered: Vec<FormantFrame> = Vec::with_capacity(formants.len().min(max_formants));
    for candidate in formants.drain(..) {
        if let Some(previous) = filtered.last_mut() {
            let min_spacing_hz =
                MIN_FORMANT_SPACING_HZ.max(previous.bandwidth_hz.min(candidate.bandwidth_hz) * 0.2);
            if candidate.frequency_hz - previous.frequency_hz < min_spacing_hz {
                if candidate.bandwidth_hz < previous.bandwidth_hz {
                    *previous = candidate;
                }
                continue;
            }
        }

        filtered.push(candidate);
        if filtered.len() == max_formants {
            break;
        }
    }

    *formants = filtered;
}

impl FormantTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn track(
        &mut self,
        candidates: &[FormantFrame],
        max_formants: usize,
    ) -> Vec<FormantFrame> {
        self.slots.resize(max_formants, None);
        let mut assignments: Vec<Option<FormantFrame>> = vec![None; max_formants];
        let mut used = vec![false; candidates.len()];
        let mut next_candidate_index = 0usize;

        for slot_index in 0..max_formants {
            let Some(previous) = self.slots[slot_index].as_ref() else {
                continue;
            };

            if let Some(candidate_index) =
                best_candidate_index(candidates, &used, next_candidate_index, previous)
            {
                assignments[slot_index] = Some(candidates[candidate_index].clone());
                used[candidate_index] = true;
                next_candidate_index = candidate_index + 1;
            }
        }

        for (candidate_index, candidate) in candidates.iter().enumerate() {
            if used[candidate_index] {
                continue;
            }

            if let Some(slot_index) = find_insertion_slot(&assignments, candidate.frequency_hz) {
                assignments[slot_index] = Some(candidate.clone());
                used[candidate_index] = true;
            }
        }

        for (slot, assignment) in self.slots.iter_mut().zip(assignments.iter()) {
            match assignment {
                Some(formant) => {
                    *slot = Some(TrackedFormant {
                        formant: formant.clone(),
                        missed_frames: 0,
                    });
                }
                None => match slot {
                    Some(state) if state.missed_frames < FORMANT_TRACK_MAX_MISSES => {
                        state.missed_frames += 1;
                    }
                    _ => *slot = None,
                },
            }
        }

        assignments
            .into_iter()
            .map(|formant| {
                formant.unwrap_or(FormantFrame {
                    frequency_hz: 0.0,
                    bandwidth_hz: 0.0,
                })
            })
            .collect()
    }
}

fn best_candidate_index(
    candidates: &[FormantFrame],
    used: &[bool],
    start_index: usize,
    previous: &TrackedFormant,
) -> Option<usize> {
    let max_jump_hz = FORMANT_TRACK_MIN_ABSOLUTE_JUMP_HZ
        .max(previous.formant.frequency_hz * FORMANT_TRACK_MAX_RELATIVE_JUMP);
    let mut best_index = None;
    let mut best_distance = f32::INFINITY;

    for index in start_index..candidates.len() {
        if used[index] {
            continue;
        }

        let distance = (candidates[index].frequency_hz - previous.formant.frequency_hz).abs();
        if distance <= max_jump_hz && distance < best_distance {
            best_distance = distance;
            best_index = Some(index);
        }
    }

    best_index
}

fn find_insertion_slot(assignments: &[Option<FormantFrame>], frequency_hz: f32) -> Option<usize> {
    assignments
        .iter()
        .enumerate()
        .find_map(|(slot_index, slot)| {
            if slot.is_some() {
                return None;
            }

            let lower_bound_hz = assignments[..slot_index]
                .iter()
                .rev()
                .find_map(|slot| slot.as_ref().map(|formant| formant.frequency_hz))
                .unwrap_or(0.0);
            let upper_bound_hz = assignments[slot_index + 1..]
                .iter()
                .find_map(|slot| slot.as_ref().map(|formant| formant.frequency_hz))
                .unwrap_or(f32::INFINITY);

            (frequency_hz > lower_bound_hz && frequency_hz < upper_bound_hz).then_some(slot_index)
        })
}

fn levinson_durbin(autocorrelation: &[f32], order: usize, lpc: &mut Vec<f32>) -> f32 {
    lpc.clear();
    lpc.resize(order + 1, 0.0);
    lpc[0] = 1.0;

    if autocorrelation.len() <= order || autocorrelation[0] <= 1.0e-9 {
        return 0.0;
    }

    let mut error = autocorrelation[0];
    let mut next = vec![0.0_f32; order + 1];
    next[0] = 1.0;

    for i in 1..=order {
        let mut reflection = autocorrelation[i];
        for j in 1..i {
            reflection += lpc[j] * autocorrelation[i - j];
        }
        reflection = -reflection / error.max(1.0e-9);

        next[..=i].copy_from_slice(&lpc[..=i]);
        next[i] = reflection;
        for j in 1..i {
            next[j] = lpc[j] + reflection * lpc[i - j];
        }
        lpc[..=i].copy_from_slice(&next[..=i]);

        error *= 1.0 - reflection * reflection;
        if error <= 1.0e-9 {
            return 0.0;
        }
    }

    error
}

fn find_polynomial_roots(coefficients: &[f32], output: &mut Vec<Complex32>) {
    let degree = coefficients.len().saturating_sub(1);
    output.clear();
    if degree == 0 {
        return;
    }

    output.extend((0..degree).map(|index| {
        let angle = 2.0 * PI * index as f32 / degree as f32;
        Complex32::new(angle.cos(), angle.sin())
    }));

    for _ in 0..128 {
        let mut converged = true;
        for index in 0..degree {
            let root = output[index];
            let mut denominator = Complex32::new(1.0, 0.0);
            for (other_index, other) in output.iter().copied().enumerate() {
                if other_index != index {
                    denominator *= root - other;
                }
            }

            if denominator.norm() <= 1.0e-12 {
                converged = false;
                continue;
            }

            let correction = evaluate_polynomial(coefficients, root) / denominator;
            output[index] -= correction;
            if correction.norm() > 1.0e-5 {
                converged = false;
            }
        }

        if converged {
            break;
        }
    }
}

fn evaluate_polynomial(coefficients: &[f32], x: Complex32) -> Complex32 {
    coefficients
        .iter()
        .copied()
        .fold(Complex32::new(0.0, 0.0), |acc, coeff| {
            acc * x + Complex32::new(coeff, 0.0)
        })
}

#[cfg(test)]
mod tests {
    use super::{FormantFrame, FormantTracker};

    fn formant(frequency_hz: f32, bandwidth_hz: f32) -> FormantFrame {
        FormantFrame {
            frequency_hz,
            bandwidth_hz,
        }
    }

    #[test]
    fn tracker_keeps_higher_slots_stable_when_extra_candidate_appears() {
        let mut tracker = FormantTracker::new();

        let first = tracker.track(
            &[
                formant(700.0, 80.0),
                formant(1_200.0, 100.0),
                formant(2_500.0, 140.0),
            ],
            4,
        );
        let second = tracker.track(
            &[
                formant(710.0, 82.0),
                formant(980.0, 90.0),
                formant(1_210.0, 110.0),
                formant(2_520.0, 145.0),
            ],
            4,
        );

        assert_eq!(first[0].frequency_hz, 700.0);
        assert_eq!(first[1].frequency_hz, 1_200.0);
        assert_eq!(first[2].frequency_hz, 2_500.0);
        assert_eq!(second[0].frequency_hz, 710.0);
        assert_eq!(second[1].frequency_hz, 1_210.0);
        assert_eq!(second[2].frequency_hz, 2_520.0);
    }

    #[test]
    fn tracker_leaves_gap_when_middle_formant_temporarily_disappears() {
        let mut tracker = FormantTracker::new();
        tracker.track(
            &[
                formant(700.0, 80.0),
                formant(1_200.0, 100.0),
                formant(2_500.0, 140.0),
            ],
            4,
        );

        let tracked = tracker.track(&[formant(705.0, 82.0), formant(2_510.0, 145.0)], 4);

        assert_eq!(tracked[0].frequency_hz, 705.0);
        assert_eq!(tracked[1].frequency_hz, 0.0);
        assert_eq!(tracked[2].frequency_hz, 2_510.0);
    }
}
