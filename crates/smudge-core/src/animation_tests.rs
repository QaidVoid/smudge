use std::time::Duration;

use proptest::prelude::*;

use super::*;
use crate::geometry::Color;
use crate::style::Style;
use crate::theme::{ColorValue, RadiusValue, Theme};

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
    let over_damped = Spring {
        stiffness: 100.0,
        damping: 40.0,
        mass: 1.0,
    };
    for (spring, latest) in [
        (Spring::SNAPPY, 750),
        (Spring::SMOOTH, 1_000),
        (Spring::BOUNCY, 2_000),
        (over_damped, 3_000),
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

/// A target, finite or not.
fn target() -> impl Strategy<Value = f32> {
    prop_oneof![
        8 => -1e6f32..1e6f32,
        1 => Just(f32::NAN),
        1 => Just(f32::INFINITY),
        1 => Just(f32::NEG_INFINITY),
    ]
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
        steps in prop::collection::vec((target(), 0u64..10_000), 1..12),
    ) {
        let spring = Spring { stiffness, damping, mass };
        let start = Instant::now();
        let mut state = SpringState::new(spring, 0.0, start);
        let mut millis = 0;
        for (target, wait) in steps {
            millis += wait;
            let (value, velocity, _) = state.step(target, at(start, millis));
            prop_assert!(
                (value.is_finite() || !target.is_finite()) && velocity.is_finite(),
                "{spring:?} gave {value} moving at {velocity} toward {target}"
            );
        }
    }
}

/// A style whose background, text color and corner radius are the ones
/// given.
fn painted(background: Option<Color>, text: Option<Color>, radius: f32) -> Style {
    let mut style = Style::default();
    style.paint.background = background.map(ColorValue::Literal);
    style.paint.text_color = text.map(ColorValue::Literal);
    style.paint.corner_radius = RadiusValue::Literal(radius);
    style
}

/// The background a transition shows, as a color.
fn background(transition: &Transition) -> Option<Color> {
    match transition.animated().paint.background {
        Some(ColorValue::Literal(color)) => Some(color),
        _ => None,
    }
}

/// Moves `transition` from `from` to `to`, frame by frame, and returns the
/// background it shows at each frame and whether it settled.
fn backgrounds(from: Option<Color>, to: Option<Color>) -> Vec<(Option<Color>, bool)> {
    let theme = Theme::light();
    let start = Instant::now();
    let mut transition = Transition::new(Spring::SMOOTH);
    transition.update(&painted(from, None, 0.0), &theme, start, false);
    (1..=150)
        .map(|frame| {
            let moving = transition.update(
                &painted(to, None, 0.0),
                &theme,
                at(start, frame * 16),
                false,
            );
            (background(&transition), !moving)
        })
        .collect()
}

/// `color` drawn over white.
fn over_white(color: Option<Color>) -> u32 {
    let Some(color) = color else {
        return 255;
    };
    let alpha = u32::from(color.a);
    (u32::from(color.r) * alpha + 255 * (255 - alpha)) / 255
}

#[test]
fn a_fill_fading_in_from_transparent_never_shows_darker_than_its_ends() {
    let hover = Color::rgb(240, 240, 240);
    let shown = backgrounds(Some(Color::TRANSPARENT), Some(hover));
    for (color, _) in &shown {
        let red = over_white(*color);
        assert!(red >= 239, "{color:?} draws {red} over white");
    }
    assert_eq!(shown.last().map(|(color, _)| *color), Some(Some(hover)));
}

#[test]
fn a_fill_that_appears_keeps_its_own_color_while_it_fades_in() {
    let accent = Color::rgb(40, 90, 200);
    for (color, _) in backgrounds(None, Some(accent)) {
        let Some(color) = color.filter(|color| color.a > 40) else {
            continue;
        };
        assert!(
            color.r.abs_diff(accent.r) <= 2 && color.b.abs_diff(accent.b) <= 2,
            "{color:?} while fading in toward {accent:?}"
        );
    }
}

#[test]
fn a_fade_to_near_black_arrives_without_a_jump() {
    let dark = Color::rgb(6, 6, 6);
    let shown = backgrounds(Some(Color::WHITE), Some(dark));
    let settled = shown
        .iter()
        .position(|(_, settled)| *settled)
        .expect("the fade settles");
    assert!(settled > 0);
    let before = shown[settled - 1].0.expect("a color shows");
    assert_eq!(shown[settled].0, Some(dark));
    assert!(
        before.r.abs_diff(dark.r) <= 1,
        "the last moving frame showed {before:?}, then {dark:?}"
    );
}

#[test]
fn a_text_color_that_is_set_or_unset_switches_at_once() {
    let theme = Theme::light();
    let start = Instant::now();
    let accent = Color::rgb(40, 90, 200);
    let mut transition = Transition::new(Spring::SMOOTH);
    transition.update(&painted(None, None, 0.0), &theme, start, false);
    let moving = transition.update(
        &painted(None, Some(accent), 0.0),
        &theme,
        at(start, 16),
        false,
    );
    assert!(!moving);
    assert_eq!(
        transition.animated().paint.text_color,
        Some(ColorValue::Literal(accent))
    );
    let moving = transition.update(&painted(None, None, 0.0), &theme, at(start, 32), false);
    assert!(!moving);
    assert_eq!(transition.animated().paint.text_color, None);
}

#[test]
fn a_radius_that_is_not_a_number_does_not_keep_moving() {
    let theme = Theme::light();
    let start = Instant::now();
    let mut transition = Transition::new(Spring::SMOOTH);
    transition.update(&painted(None, None, 4.0), &theme, start, false);
    for frame in 1..=3 {
        let moving = transition.update(
            &painted(None, None, f32::NAN),
            &theme,
            at(start, frame * 16),
            false,
        );
        assert!(!moving, "frame {frame} still moving");
    }
    let moving = transition.update(
        &painted(None, None, f32::INFINITY),
        &theme,
        at(start, 64),
        false,
    );
    assert!(!moving);
}
