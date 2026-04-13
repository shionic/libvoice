#[derive(Debug, Default)]
pub(crate) struct HarmonicAnalyzer;

impl HarmonicAnalyzer {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn estimate(
        &mut self,
        magnitudes: &[f32],
        bin_hz: f32,
        pitch_hz: Option<f32>,
        max_harmonic_frequency_hz: f32,
        harmonic_min_strength_ratio: f32,
    ) -> Vec<Option<f32>> {
        let Some(f0_hz) = pitch_hz else {
            return Vec::new();
        };

        if magnitudes.len() < 2 || bin_hz <= 0.0 || f0_hz <= 0.0 {
            return Vec::new();
        }

        let nyquist_hz = (magnitudes.len().saturating_sub(1)) as f32 * bin_hz;
        let max_frequency_hz = max_harmonic_frequency_hz.min(nyquist_hz);
        if max_frequency_hz < f0_hz {
            return Vec::new();
        }

        let harmonic_count = (max_frequency_hz / f0_hz).floor().max(1.0) as usize;
        let mut band_powers = Vec::with_capacity(harmonic_count);
        for harmonic_number in 1..=harmonic_count {
            band_powers.push(measure_harmonic_band_power(
                magnitudes,
                bin_hz,
                f0_hz,
                harmonic_number,
                max_frequency_hz,
            ));
        }

        let Some(fundamental_power) = band_powers.first().copied().flatten() else {
            return vec![None; harmonic_count];
        };
        if fundamental_power <= 1.0e-12 {
            return vec![None; harmonic_count];
        }

        let mut strengths = Vec::with_capacity(harmonic_count);
        strengths.push(Some(1.0));
        for band_power in band_powers.into_iter().skip(1) {
            let Some(band_power) = band_power else {
                strengths.push(None);
                continue;
            };

            let ratio = (band_power / fundamental_power).sqrt();
            if ratio >= harmonic_min_strength_ratio {
                strengths.push(Some(ratio));
            } else {
                strengths.push(None);
            }
        }

        strengths
    }
}

fn measure_harmonic_band_power(
    magnitudes: &[f32],
    bin_hz: f32,
    f0_hz: f32,
    harmonic_number: usize,
    max_frequency_hz: f32,
) -> Option<f32> {
    let target_hz = harmonic_number as f32 * f0_hz;
    if target_hz > max_frequency_hz || harmonic_number == 0 {
        return None;
    }

    let lower_edge_hz = if harmonic_number == 1 {
        (0.5 * f0_hz).max(bin_hz * 0.5)
    } else {
        ((harmonic_number as f32) - 0.5) * f0_hz
    };
    let upper_edge_hz = (((harmonic_number as f32) + 0.5) * f0_hz).min(max_frequency_hz);
    if upper_edge_hz <= lower_edge_hz {
        return None;
    }

    let mut power_sum = 0.0_f32;
    let mut bin_count = 0usize;
    for (index, magnitude) in magnitudes.iter().copied().enumerate().skip(1) {
        let bin_center_hz = index as f32 * bin_hz;
        if bin_center_hz < lower_edge_hz || bin_center_hz > upper_edge_hz {
            continue;
        }
        power_sum += magnitude * magnitude;
        bin_count += 1;
    }

    if bin_count == 0 {
        let target_bin = (target_hz / bin_hz)
            .round()
            .clamp(1.0, (magnitudes.len().saturating_sub(1)) as f32) as usize;
        let power = magnitudes[target_bin] * magnitudes[target_bin];
        (power > 1.0e-12).then_some(power)
    } else {
        (power_sum > 1.0e-12).then_some(power_sum)
    }
}
