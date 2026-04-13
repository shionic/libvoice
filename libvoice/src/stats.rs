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
        / count as f32;

    Some(SummaryStats {
        count,
        mean,
        std: variance.sqrt(),
        median: percentile_sorted(&values, 0.5),
        min: values[0],
        max: values[count - 1],
        p5: percentile_sorted(&values, 0.05),
        p95: percentile_sorted(&values, 0.95),
    })
}

fn percentile_sorted(values: &[f32], percentile: f32) -> f32 {
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
