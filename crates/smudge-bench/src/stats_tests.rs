//! Tests for the percentile summaries.

use super::{Percentiles, percentile};

#[test]
fn median_of_odd_length_is_the_middle_value() {
    assert_eq!(percentile(&[4.0, 1.0, 3.0, 5.0, 2.0], 50.0), 3.0);
}

#[test]
fn median_of_even_length_is_the_lower_middle_value() {
    assert_eq!(percentile(&[4.0, 1.0, 3.0, 2.0], 50.0), 2.0);
}

#[test]
fn percentile_sorts_unsorted_values_first() {
    assert_eq!(percentile(&[90.0, 10.0, 50.0, 70.0, 30.0], 95.0), 90.0);
    assert_eq!(percentile(&[90.0, 10.0, 50.0, 70.0, 30.0], 50.0), 50.0);
}

#[test]
fn percentile_of_a_single_value_is_that_value() {
    assert_eq!(percentile(&[7.0], 50.0), 7.0);
    assert_eq!(percentile(&[7.0], 95.0), 7.0);
    assert_eq!(percentile(&[7.0], 99.0), 7.0);
}

#[test]
fn percentile_clamps_the_rank_into_range() {
    assert_eq!(percentile(&[5.0, 6.0, 7.0], 0.0), 5.0);
    assert_eq!(percentile(&[5.0, 6.0, 7.0], 100.0), 7.0);
}

#[test]
fn p95_and_p99_of_known_values_hold() {
    let values: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile(&values, 95.0), 95.0);
    assert_eq!(percentile(&values, 99.0), 99.0);
    assert_eq!(
        Percentiles::of(&values),
        Percentiles {
            median: 50.0,
            p95: 95.0,
            p99: 99.0,
        }
    );
}
