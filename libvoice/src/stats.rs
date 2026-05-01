use crate::model::SummaryStats;

pub(crate) fn summarize_optional<I>(values: I) -> Option<SummaryStats>
where
    I: Iterator<Item = f32>,
{
    summarize_values(values.filter(|x| x.is_finite()).collect())
}

pub(crate) fn summarize_required<I>(values: I) -> Option<SummaryStats>
where
    I: Iterator<Item = f32>,
{
    summarize_values(values.filter(|x| x.is_finite()).collect())
}

fn summarize_values(mut values: Vec<f32>) -> Option<SummaryStats> {
    if values.is_empty() {
        return None;
    }

    values.sort_unstable_by(|a, b| a.total_cmp(b));
    let count = values.len();
    let mean = values.iter().sum::<f32>() / count as f32;
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / (count as f32 - 1.0).max(1.0);

    Some(SummaryStats {
        count,
        mean,
        std: variance.sqrt(),
        median: percentile_sorted_ref(&values, 0.5),
        min: values[0],
        max: values[count - 1],
        p5: percentile_sorted_ref(&values, 0.05),
        p95: percentile_sorted_ref(&values, 0.95),
    })
}

pub(crate) fn percentile_sorted_ref(values: &[f32], percentile: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    if values.len() == 1 {
        return values[0];
    }

    let position = percentile.clamp(0.0, 1.0) * (values.len() - 1) as f32;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        return values[lower];
    }

    let weight = position - lower as f32;
    values[lower] * (1.0 - weight) + values[upper] * weight
}

#[cfg(test)]
mod tests {
    use super::summarize_values;

    #[test]
    fn sample_variance_applies_bessels_correction() {
        // Values: [2, 4, 4, 4, 5, 5, 7, 9]
        // Mean = 5.0
        // Population Variance = sum((x-5)^2) / 8 = (9 + 1 + 1 + 1 + 0 + 0 + 4 + 16) / 8 = 32 / 8 = 4.0
        // Sample Variance = 32 / 7 ≈ 4.5714
        let values = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let stats = summarize_values(values).unwrap();

        assert_eq!(stats.mean, 5.0);
        let expected_sample_variance: f32 = 32.0 / 7.0;
        let expected_std = expected_sample_variance.sqrt();
        assert!((stats.std - expected_std).abs() < 1.0e-6);
    }
}
