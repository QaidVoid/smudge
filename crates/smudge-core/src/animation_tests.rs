use std::time::Duration;

use proptest::prelude::*;

use super::*;

/// The time `millis` milliseconds after `start`.
fn at(start: Instant, millis: u64) -> Instant {
    start + Duration::from_millis(millis)
}

/// Steps a spring from 0 toward 1 every 5 milliseconds, and returns the
/// values and the time it first reported arrived.
fn run(spring: Spring) -> (Vec<f32>, Option<u64>) {
    let start = Instant::now();
    let mut state = SpringState::new(spring, 0.0, start);
    let mut values = Vec::new();
    for millis in (5..=5_000).step_by(5) {
        let (value, _, settled) = state.step(1.0, at(start, millis));
        values.push(value);
        if settled {
            return (values, Some(millis));
        }
    }
    (values, None)
}

#[test]
fn a_retarget_keeps_the_value_and_the_velocity() {
    let start = Instant::now();
    let mut moving = SpringState::new(Spring::SNAPPY, 0.0, start);
    moving.step(1.0, start);
    let mut unchanged = moving;
    let (value, velocity, _) = unchanged.step(1.0, at(start, 60));
    let (retargeted, new_velocity, settled) = moving.step(-1.0, at(start, 60));
    assert!(!settled);
    assert!(
        (retargeted - value).abs() < 1e-6,
        "{retargeted} against {value}"
    );
    assert!(
        (new_velocity - velocity).abs() < 1e-4,
        "{new_velocity} against {velocity}"
    );
    let (later, _, _) = moving.step(-1.0, at(start, 61));
    assert!(
        (later - retargeted).abs() < 0.05,
        "a millisecond later the value moved from {retargeted} to {later}"
    );
}

#[test]
fn each_preset_settles_in_its_own_time() {
    for (spring, latest) in [
        (Spring::SNAPPY, 750),
        (Spring::SMOOTH, 1_000),
        (Spring::BOUNCY, 2_000),
    ] {
        let (_, settled) = run(spring);
        let settled = settled.expect("the spring settles");
        assert!(
            settled <= latest && settled >= latest / 3,
            "{spring:?} settled after {settled} ms"
        );
    }
}

#[test]
fn a_critically_damped_spring_does_not_overshoot_and_a_bouncy_one_does() {
    let (smooth, _) = run(Spring::SMOOTH);
    assert!(smooth.iter().all(|value| *value <= 1.0));
    let (bouncy, _) = run(Spring::BOUNCY);
    assert!(bouncy.iter().any(|value| *value > 1.2));
}

#[test]
fn an_arrived_value_is_exactly_the_target_at_rest() {
    let start = Instant::now();
    let mut state = SpringState::new(Spring::SMOOTH, 3.0, start);
    state.step(-2.5, start);
    let (value, velocity, settled) = state.step(-2.5, at(start, 5_000));
    assert!(settled);
    assert_eq!((value, velocity), (-2.5, 0.0));
}

#[test]
fn a_spring_that_cannot_move_jumps_to_its_target() {
    let start = Instant::now();
    for spring in [
        Spring {
            stiffness: 0.0,
            ..Spring::SMOOTH
        },
        Spring {
            damping: -1.0,
            ..Spring::SMOOTH
        },
        Spring {
            mass: f32::NAN,
            ..Spring::SMOOTH
        },
        Spring {
            stiffness: f32::INFINITY,
            ..Spring::SMOOTH
        },
    ] {
        let mut state = SpringState::new(spring, 0.0, start);
        assert_eq!(state.step(4.0, start), (4.0, 0.0, true), "{spring:?}");
    }
}

/// A finite, positive spring parameter from the smallest to the largest
/// an `f32` holds.
fn parameter() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(f32::MIN_POSITIVE),
        Just(f32::MAX),
        (1e-6f32..1e6f32),
        any::<f32>().prop_filter("finite and positive", |value| value.is_finite()
            && *value > 0.0),
    ]
}

proptest! {
    #[test]
    fn extreme_springs_and_targets_stay_finite(
        stiffness in parameter(),
        damping in parameter(),
        mass in parameter(),
        steps in prop::collection::vec((-1e6f32..1e6f32, 0u64..10_000), 1..12),
    ) {
        let spring = Spring { stiffness, damping, mass };
        let start = Instant::now();
        let mut state = SpringState::new(spring, 0.0, start);
        let mut millis = 0;
        for (target, wait) in steps {
            millis += wait;
            let (value, velocity, _) = state.step(target, at(start, millis));
            prop_assert!(value.is_finite() && velocity.is_finite(),
                "{spring:?} gave {value} moving at {velocity}");
        }
    }
}
