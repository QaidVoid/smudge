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

#[test]
fn dropping_every_sender_frees_the_handler_once_its_values_are_delivered() {
    let (received, set_received) = signal_local(0u32);
    let dropped = Rc::new(Cell::new(false));
    let mut app = App::new(config(), panel);
    let child = app.root().child();
    let canary = Canary(Rc::clone(&dropped));
    let sender = child.with(|| {
        channel::channel(move |value: u32| {
            let _alive = &canary;
            set_received.set(value);
        })
    });
    let clone = sender.clone();
    drop(sender);
    clone.send(5).unwrap();
    drop(clone);
    assert!(!dropped.get(), "a value still waits for the handler");

    app.frame();
    assert_eq!(received.get(), 5, "the value sent before the drop arrives");
    assert!(
        dropped.get(),
        "with no sender left the handler is freed while its owner lives on"
    );
    assert!(!app.needs_frame());
}

#[test]
fn a_live_sender_keeps_the_handler() {
    let dropped = Rc::new(Cell::new(false));
    let mut app = App::new(config(), panel);
    let child = app.root().child();
    let canary = Canary(Rc::clone(&dropped));
    let sender = child.with(|| {
        channel::channel(move |_: u32| {
            let _alive = &canary;
        })
    });
    app.frame();
    app.frame();
    assert!(!dropped.get(), "an idle channel with a sender stays open");
    sender.send(1).unwrap();
}

#[test]
fn a_channel_opened_in_a_binding_rerun_belongs_to_the_binding_owner() {
    let (trigger, set_trigger) = signal_local(0u32);
    let (received, set_received) = signal_local(0u32);
    let holder: Rc<RefCell<Option<channel::UiSender<u32>>>> = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&holder);
    let mut app = App::new(config(), move || {
        let slot = Rc::clone(&slot);
        crate::reactive::RenderEffect::new(move || {
            if trigger.get() > 0 {
                let set_received = set_received.clone();
                slot.replace(Some(channel::channel(move |value: u32| {
                    set_received.set(value);
                })));
            }
        });
        panel()
    });
    app.frame();
    set_trigger.set(1);
    app.frame();

    let sender = holder.borrow().clone().expect("the rerun opened a channel");
    sender.send(8).unwrap();
    app.frame();
    assert_eq!(received.get(), 8);

    drop(app);
    assert!(
        sender.send(9).is_err(),
        "the channel closed with the owner the binding was created under"
    );
}
