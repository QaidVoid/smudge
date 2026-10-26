//! Unit tests for timers.
//!
//! The tests run the app on a virtual clock and advance it by hand, so a
//! timer fires at exactly the moment the test chooses and nothing sleeps.

use std::time::Duration;

use web_time::Instant;

use crate::app::{App, AppConfig, RepaintMode};
use crate::geometry::{Color, Size};
use crate::reactive::{Owner, prelude::*, signal_local};
use crate::timer::{self, VirtualClock};
use crate::widget::panel;

/// The settings for a small app, shared by the tests.
fn config() -> AppConfig {
    AppConfig {
        viewport: Size {
            width: 40.0,
            height: 20.0,
        },
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint: RepaintMode::Damage,
    }
}

/// An app on a virtual clock that starts at `start`, whose build closure
/// runs `schedule` before it returns the root panel.
fn timed(start: Instant, schedule: impl FnOnce()) -> (App, VirtualClock) {
    let clock = VirtualClock::new(start);
    let app = App::with_clock(config(), clock.clone().into(), || {
        schedule();
        panel()
    });
    (app, clock)
}

#[test]
fn after_fires_once_when_the_time_passes() {
    let start = Instant::now();
    let (fired, set_fired) = signal_local(0u32);
    let (mut app, clock) = timed(start, || {
        timer::after(Duration::from_secs(1), move || {
            set_fired.update(|count| *count += 1)
        });
    });

    clock.advance(Duration::from_secs(1) - Duration::from_nanos(1));
    app.frame();
    assert_eq!(fired.get(), 0, "the timer is not due yet");

    clock.advance(Duration::from_nanos(1));
    app.frame();
    assert_eq!(fired.get(), 1, "the timer fired when its time came");

    clock.advance(Duration::from_secs(10));
    app.frame();
    assert_eq!(fired.get(), 1, "a one-shot never fires again");
}

#[test]
fn every_fires_each_period_until_its_handle_drops() {
    let start = Instant::now();
    let (ticks, set_ticks) = signal_local(0u32);
    let handle = std::rc::Rc::new(std::cell::RefCell::new(None));
    let (mut app, clock) = timed(start, {
        let handle = std::rc::Rc::clone(&handle);
        move || {
            *handle.borrow_mut() = Some(timer::every(Duration::from_millis(250), move || {
                set_ticks.update(|count| *count += 1)
            }));
        }
    });

    clock.advance(Duration::from_millis(250));
    app.frame();
    assert_eq!(ticks.get(), 1, "one period has passed");

    // Falling behind fires once per frame, not once per missed period.
    clock.advance(Duration::from_millis(5_000));
    app.frame();
    assert_eq!(ticks.get(), 2);

    handle.borrow_mut().take();
    clock.advance(Duration::from_millis(250));
    app.frame();
    assert_eq!(ticks.get(), 2, "dropping the handle canceled the timer");
}

#[test]
fn cleaning_the_owner_up_cancels_its_timers() {
    let start = Instant::now();
    let (ticks, set_ticks) = signal_local(0u32);
    let (mut app, clock) = timed(start, || {});

    let holder: Owner = app.root().child();
    let handle = holder.with(|| {
        timer::every(Duration::from_millis(100), move || {
            set_ticks.update(|count| *count += 1)
        })
    });

    clock.advance(Duration::from_millis(100));
    app.frame();
    assert_eq!(ticks.get(), 1);

    // The handle is still alive, but cleaning the owner up stops the timer
    // and takes it out of the deadline at once.
    holder.cleanup();
    assert_eq!(app.next_deadline(), None, "a dead timer wakes nothing");
    clock.advance(Duration::from_millis(100));
    app.frame();
    assert_eq!(ticks.get(), 1, "the timer died with its owner");
    drop(handle);
}

#[test]
fn now_reads_the_app_clock() {
    let start = Instant::now();
    let (seen, set_seen) = signal_local(None);
    let (mut app, clock) = timed(start, || {
        timer::after(Duration::from_secs(1), move || {
            set_seen.set(Some(timer::now()));
        });
    });

    clock.advance(Duration::from_secs(1));
    app.frame();
    assert_eq!(
        seen.get(),
        Some(start + Duration::from_secs(1)),
        "a callback reads the app's clock, not the system clock"
    );
    assert_eq!(app.now(), start + Duration::from_secs(1));
}

#[test]
fn next_deadline_reports_the_soonest_timer() {
    let start = Instant::now();
    let (mut app, clock) = timed(start, || {
        timer::after(Duration::from_secs(2), || {});
        timer::after(Duration::from_secs(1), || {});
    });

    assert_eq!(
        app.next_deadline(),
        Some(start + Duration::from_secs(1)),
        "the soonest timer is the deadline"
    );

    clock.advance(Duration::from_secs(1));
    app.frame();
    assert_eq!(
        app.next_deadline(),
        Some(start + Duration::from_secs(2)),
        "the fired one-shot is gone"
    );
    clock.advance(Duration::from_secs(1));
    app.frame();
    assert_eq!(app.next_deadline(), None, "no timers, no deadline");
}

#[test]
fn a_timer_with_no_app_is_inert() {
    // No owner on this thread: the closure is dropped and the handle is
    // inert, and neither the call nor the drop panics.
    let handle = timer::every(Duration::from_millis(1), || panic!("never runs"));
    drop(handle);
    timer::after(Duration::from_millis(1), || panic!("never runs"));
}
