//! Percentile summaries over a run's frames.
//!
//! Percentiles use the nearest-rank method: the values are sorted and the
//! rank `ceil(p / 100 * n)` is reported. The median of an even number of
//! values is therefore the lower of the two middle values, and no value is
//! ever interpolated.

/// The median, 95th and 99th percentile of one quantity over a run's frames.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Percentiles {
    /// The 50th percentile.
    pub(crate) median: f64,
    /// The 95th percentile.
    pub(crate) p95: f64,
    /// The 99th percentile.
    pub(crate) p99: f64,
}

impl Percentiles {
    /// Summarizes `values`.
    ///
    /// # Panics
    ///
    /// Panics when `values` is empty, because a percentile of nothing is
    /// undefined. Summaries are only built from a run that measured frames.
    pub(crate) fn of(values: &[f64]) -> Percentiles {
        assert!(!values.is_empty(), "a summary needs at least one value");
        Percentiles {
            median: percentile(values, 50.0),
            p95: percentile(values, 95.0),
            p99: percentile(values, 99.0),
        }
    }
}

/// Returns the `p`th percentile of `values` with the nearest-rank method.
///
/// `p` is between 0 and 100. A rank that rounds to zero counts as the first
/// value, and a rank past the end counts as the last, so the result is always
/// one of the values.
///
/// # Panics
///
/// Panics when `values` is empty.
pub(crate) fn percentile(values: &[f64], p: f64) -> f64 {
    assert!(!values.is_empty(), "a percentile needs at least one value");
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (p / 100.0 * sorted.len() as f64).ceil();
    let index = rank.clamp(1.0, sorted.len() as f64) as usize - 1;
    sorted[index]
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
