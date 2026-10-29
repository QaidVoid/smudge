//! Unit tests for channels.
//!
//! The tests cover the bookkeeping the public API cannot observe from the
//! outside: the misuse case with no application, and a receiver whose owner
//! is not the application's root.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::app::{App, AppConfig, RepaintMode};
use crate::channel::{self, Closed};
use crate::geometry::{Color, Size};
use crate::reactive::{prelude::*, provide_context, signal_local};
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

/// Sets a flag when dropped, so a test can see that a handler is gone.
struct Canary(Rc<Cell<bool>>);

impl Drop for Canary {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[cfg(not(debug_assertions))]
#[test]
fn a_channel_with_no_app_delivers_nothing() {
    let sender = channel::channel(|_: u32| panic!("nothing delivers without an app"));
    let Closed { value } = sender.send(1).expect_err("the sender is closed");
    assert_eq!(value, 1);

    // An owner with no app above it is no app either.
    let owner = crate::reactive::Owner::new();
    let sender = owner.with(|| channel::channel(|_: u32| panic!("nothing delivers")));
    assert!(sender.send(2).is_err(), "the sender is closed");
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "no application owner is current")]
fn a_channel_with_no_app_asserts_in_debug_builds() {
    let _sender = channel::channel(|_: u32| {});
}

#[test]
fn cleaning_the_owner_up_closes_its_channel_and_drops_its_handler() {
    let (received, set_received) = signal_local(0u32);
    let dropped = Rc::new(Cell::new(false));
    let mut app = App::new(config(), panel);
    let holder = Rc::new(RefCell::new(None));
    let child = app.root().child();
    let canary = Canary(Rc::clone(&dropped));
    *holder.borrow_mut() = Some(child.with(|| {
        channel::channel(move |value: u32| {
            let _alive = &canary;
            set_received.set(value);
        })
    }));

    let sender = holder
        .borrow()
        .clone()
        .expect("the owner stored its sender");
    sender.send(1).unwrap();
    assert!(app.needs_frame(), "a queued value asks for a frame");
    app.frame();
    assert_eq!(received.get(), 1);
    assert!(!dropped.get(), "the handler lives while its channel does");

    child.cleanup();
    assert!(dropped.get(), "the handler died with its owner");
    assert!(!app.needs_frame(), "a closed channel asks for no frame");
    let Closed { value } = sender.send(2).expect_err("the channel closed");
    assert_eq!(value, 2, "the value comes back");
    app.frame();
    assert_eq!(received.get(), 1, "nothing more is delivered");
}

#[test]
fn the_handler_runs_inside_the_owner_that_created_the_channel() {
    let (received, set_received) = signal_local(0u32);
    let mut app = App::new(config(), panel);
    let child = app.root().child();
    let sender = child.with(|| {
        provide_context(2u32);
        channel::channel(move |value: u32| {
            let add = crate::reactive::use_context::<u32>().unwrap_or(0);
            set_received.set(value + add);
        })
    });

    sender.send(40).unwrap();
    app.frame();
    assert_eq!(
        received.get(),
        42,
        "the handler read the context of its own owner"
    );
}
