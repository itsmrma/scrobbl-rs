//! Percentile helpers for the latency tables.

/// Nearest-rank percentile.
///
/// `p` is in `[0, 100]`. Returns `None` for an empty sample.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p.clamp(0.0, 100.0) / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.saturating_sub(1).min(sorted.len() - 1)])
}

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn empty_sample_has_no_percentile() {
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn nearest_rank_p50_and_p95() {
        let values: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(percentile(&values, 50.0), Some(10.0));
        assert_eq!(percentile(&values, 95.0), Some(19.0));
        assert_eq!(percentile(&values, 100.0), Some(20.0));
    }

    #[test]
    fn unsorted_input_is_sorted_first() {
        assert_eq!(percentile(&[9.0, 1.0, 5.0], 50.0), Some(5.0));
    }
}
