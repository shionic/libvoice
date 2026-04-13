#[derive(Debug, Default)]
pub(crate) struct HarmonicAnalyzer {
    power_prefix_sums: Vec<f32>,
}

impl HarmonicAnalyzer {
    pub(crate) fn new() -> Self {
        Self::default()
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

        self.power_prefix_sums.resize(magnitudes.len() + 1, 0.0);
        self.power_prefix_sums[0] = 0.0;
        for (index, magnitude) in magnitudes.iter().copied().enumerate() {
            let power = magnitude * magnitude;
            self.power_prefix_sums[index + 1] = self.power_prefix_sums[index] + power;
        }

        let harmonic_count = (max_frequency_hz / f0_hz).floor().max(1.0) as usize;
        let Some(fundamental_power) = measure_harmonic_band_power(
            magnitudes.len(),
            bin_hz,
            f0_hz,
            1,
            max_frequency_hz,
            &self.power_prefix_sums,
        ) else {
            return vec![None; harmonic_count];
        };
        if fundamental_power <= 1.0e-12 {
            return vec![None; harmonic_count];
        }

        let mut strengths = Vec::with_capacity(harmonic_count);
        strengths.push(Some(1.0));
        for harmonic_number in 2..=harmonic_count {
            let band_power = measure_harmonic_band_power(
                magnitudes.len(),
                bin_hz,
                f0_hz,
                harmonic_number,
                max_frequency_hz,
                &self.power_prefix_sums,
            );
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
    magnitude_len: usize,
    bin_hz: f32,
    f0_hz: f32,
    harmonic_number: usize,
    max_frequency_hz: f32,
    power_prefix_sums: &[f32],
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

    let max_bin = magnitude_len.saturating_sub(1);
    let start_bin = (lower_edge_hz / bin_hz).ceil().max(1.0) as usize;
    let end_bin = (upper_edge_hz / bin_hz).floor().min(max_bin as f32) as usize;

    if start_bin > end_bin {
        let target_bin = (target_hz / bin_hz)
            .round()
            .clamp(1.0, max_bin as f32) as usize;
        let power = power_prefix_sums[target_bin + 1] - power_prefix_sums[target_bin];
        (power > 1.0e-12).then_some(power)
    } else {
        let power_sum = power_prefix_sums[end_bin + 1] - power_prefix_sums[start_bin];
        (power_sum > 1.0e-12).then_some(power_sum)
    }
}
