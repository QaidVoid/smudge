//! Tests of the step snapping the sliders share, against numbers that
//! fight it: steps that are no use, ends that are no numbers or reversed,
//! and counts that float error leaves a hair short of a whole step.

use super::snap;

#[test]
fn a_step_that_is_not_finite_and_positive_still_clamps() {
    let range = 0.0..=100.0;
    for step in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            snap(30.0, &range, step),
            30.0,
            "a step of {step} leaves an inside value"
        );
        assert_eq!(
            snap(150.0, &range, step),
            100.0,
            "a step of {step} still clamps a value past the end"
        );
        assert_eq!(
            snap(-150.0, &range, step),
            0.0,
            "a step of {step} still clamps a value before the start"
        );
    }
}

#[test]
fn a_nan_end_is_no_bound_and_a_reversed_end_is_the_start() {
    assert_eq!(
        snap(30.0, &(0.0..=f64::NAN), 10.0),
        30.0,
        "a NaN end is no bound"
    );
    assert_eq!(
        snap(30.0, &(f64::NAN..=100.0), 10.0),
        30.0,
        "a NaN start is no bound"
    );
    assert_eq!(
        snap(30.0, &(100.0..=0.0), 10.0),
        100.0,
        "an end below the start counts as the start"
    );
    assert_eq!(
        snap(f64::NAN, &(0.0..=100.0), 10.0),
        0.0,
        "a NaN value counts as the start"
    );
}

#[test]
fn an_end_a_hair_short_of_a_whole_step_still_counts() {
    // The division leaves the count a hair under ten, and the hair counts
    // as rounding: the end stays a step the grid reaches.
    assert_eq!(
        snap(0.3, &(0.0..=0.3), 0.03),
        0.3,
        "the grid reaches an end a hair short of a whole step"
    );
    assert_eq!(
        snap(1.015, &(0.0..=1.015), 0.035),
        1.015,
        "the same rescue holds a step further from a whole"
    );
    // A count that is genuinely short drops the boundary it cannot reach.
    assert_eq!(
        snap(0.3, &(0.0..=0.3), 0.04),
        0.28,
        "a count a whole rounding short keeps the grid off the end"
    );
}

#[test]
fn a_count_past_the_hair_of_a_whole_stays_inside_the_range() {
    // At counts this large the eight-ulp bound would swallow a real half
    // step, so the cap keeps the last step inside the end.
    let end = 1000000000000017.5;
    assert_eq!(
        snap(end, &(0.0..=end), 1.0),
        1000000000000017.0,
        "the capped hair keeps the last step inside the end"
    );
}
