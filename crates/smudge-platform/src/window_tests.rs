//! Tests for the run entry points that need no window.

use std::time::Duration;

use web_time::Instant;

use smudge_core::geometry::{Color, Size};
use smudge_core::widget::panel;

use super::{WaitDecision, WindowOptions, next_frame_at, run_with, wait_decision};
use crate::PlatformError;

/// Returns options with an opaque clear color, so a test only has to change
/// the field it checks.
fn options(clear_color: Color) -> WindowOptions {
    WindowOptions {
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        clear_color,
        ..WindowOptions::default()
    }
}

#[test]
fn a_translucent_clear_color_fails_before_any_window_opens() {
    let error = run_with(options(Color::rgba(20, 24, 30, 128)), panel, |_| {})
        .expect_err("a clear color that is not opaque must be rejected");

    assert!(matches!(error, PlatformError::ClearColor));
}

#[test]
fn a_due_timer_asks_for_its_frame_at_once() {
    let now = Instant::now();
    assert_eq!(
        wait_decision(Some(now - Duration::from_millis(1)), now),
        WaitDecision::Now,
        "a deadline in the past is due"
    );
    assert_eq!(
        wait_decision(Some(now), now),
        WaitDecision::Now,
        "a deadline exactly now is due"
    );
}

#[test]
fn a_future_timer_sets_the_wake_up_time() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(530);
    assert_eq!(
        wait_decision(Some(deadline), now),
        WaitDecision::Until(deadline)
    );
}

#[test]
fn no_timer_waits_for_events() {
    let now = Instant::now();
    assert_eq!(wait_decision(None, now), WaitDecision::Wait);
}

#[test]
fn a_frame_that_ran_early_waits_for_the_rest_of_the_period() {
    let started = Instant::now();
    let period = Duration::from_millis(16);
    let now = started + Duration::from_millis(3);

    assert_eq!(next_frame_at(started, period, now), Some(started + period));
}

#[test]
fn a_frame_that_ran_late_starts_the_next_at_once_without_catching_up() {
    let started = Instant::now();
    let period = Duration::from_millis(16);

    assert_eq!(
        next_frame_at(started, period, started + Duration::from_millis(40)),
        None,
        "a late frame asks for one frame now, not for the frames it missed"
    );
    assert_eq!(next_frame_at(started, period, started + period), None);
}
