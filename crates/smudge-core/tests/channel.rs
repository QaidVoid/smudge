//! Behavior tests for messages from other threads.
//!
//! Every test drives the same `App::frame` a window does, so what it
//! observes is what a running application shows. No test installs a timer
//! unless it is about timers, so a value that arrives without one proves the
//! channel woke the frame itself.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::channel::{self, Closed};
use smudge_core::element::show;
use smudge_core::geometry::{Color, Corners, Rect, Size};
use smudge_core::paint::{DisplayItem, PaintOp};
use smudge_core::reactive::{RenderEffect, prelude::*, signal_local};
use smudge_core::style::px;
use smudge_core::timer::VirtualClock;
use smudge_core::widget::panel;
use web_time::Instant;

/// The viewport every test lays its tree out for, at a scale of one.
const VIEWPORT: Size = Size {
    width: 120.0,
    height: 60.0,
};

/// A configuration for the test viewport.
fn config() -> AppConfig {
    AppConfig {
        viewport: VIEWPORT,
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint: RepaintMode::Damage,
    }
}

/// A filled rectangle operation.
fn fill(rect: Rect, color: Color) -> PaintOp {
    PaintOp::FillRect {
        rect,
        radius: Corners::ZERO,
        color,
    }
}

/// Runs one frame and copies its display list out of the app.
fn items(app: &mut App) -> Vec<DisplayItem> {
    app.frame().list.items().to_vec()
}

/// Holds a sender that a build closure creates, for the test to reach.
type Slot<T> = Rc<RefCell<Option<channel::UiSender<T>>>>;

/// Creates an empty slot and a closure that stores a sender into it.
fn slot<T: 'static>() -> (Slot<T>, impl Fn(channel::UiSender<T>) + Clone + 'static) {
    let holder: Slot<T> = Rc::new(RefCell::new(None));
    let store = {
        let holder = Rc::clone(&holder);
        move |sender: channel::UiSender<T>| *holder.borrow_mut() = Some(sender)
    };
    (holder, store)
}

/// Takes the sender a build closure stored.
fn stored<T>(holder: &Slot<T>) -> channel::UiSender<T> {
    holder
        .borrow()
        .clone()
        .expect("the build closure stored its sender")
}

#[test]
fn a_message_from_another_thread_is_applied_by_the_next_frame() {
    let (shade, set_shade) = signal_local(Color::BLACK);
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        let sender = channel::channel(move |lit: bool| {
            set_shade.set(if lit {
                Color::rgb(220, 60, 60)
            } else {
                Color::BLACK
            });
        });
        store(sender);
        panel()
            .width(px(40.0))
            .height(px(20.0))
            .background(move || shade.get().into())
    });
    let first = items(&mut app);
    assert_eq!(
        first,
        vec![DisplayItem::Op(fill(
            Rect::new(0.0, 0.0, 120.0, 60.0),
            Color::BLACK
        ))],
        "the frame before the message shows the first shade"
    );

    let sender = stored(&holder);
    let (done, queued) = mpsc::channel();
    thread::spawn(move || {
        sender.send(true).unwrap();
        done.send(()).unwrap();
    });
    queued
        .recv()
        .expect("the thread sends one value and then signals");

    let applied = items(&mut app);
    assert_eq!(
        applied,
        vec![DisplayItem::Op(fill(
            Rect::new(0.0, 0.0, 120.0, 60.0),
            Color::rgb(220, 60, 60)
        ))],
        "the message's signal write painted, with no timer involved"
    );
    assert_eq!(app.stats().nodes_painted, 1, "one box repainted");
}

#[test]
fn messages_apply_in_send_order() {
    let (line, set_line) = signal_local(String::new());
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        let sender = channel::channel(move |digit: u32| {
            set_line.update(|line| line.push(char::from_digit(digit, 10).unwrap()));
        });
        store(sender);
        panel()
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    for digit in 1..=5 {
        sender.send(digit).unwrap();
    }
    let _ = items(&mut app);

    assert_eq!(line.get(), "12345", "values deliver in send order");
}

#[test]
fn a_handler_that_sends_to_its_own_channel_defers_the_value() {
    let (seen, set_seen) = signal_local(Vec::<u32>::new());
    let (holder, store) = slot();
    let self_sender = Rc::clone(&holder);
    let mut app = App::new(config(), move || {
        let sender = channel::channel(move |value: u32| {
            set_seen.update(|seen| seen.push(value));
            if value < 3
                && let Some(sender) = self_sender.borrow().clone()
            {
                sender.send(value + 1).unwrap();
            }
        });
        store(sender);
        panel()
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    sender.send(1).unwrap();
    let _ = items(&mut app);
    assert_eq!(
        *seen.read(|seen| seen.clone()),
        [1],
        "the frame took one value, not the one the handler sent"
    );

    let _ = items(&mut app);
    assert_eq!(*seen.read(|seen| seen.clone()), [1, 2]);

    let _ = items(&mut app);
    assert_eq!(*seen.read(|seen| seen.clone()), [1, 2, 3]);
    assert!(
        !app.needs_frame(),
        "the chain ended, so no frame is pending"
    );
}

#[test]
fn a_value_sent_while_the_drain_runs_waits_for_the_next_frame() {
    let (seen, set_seen) = signal_local(Vec::<u32>::new());
    let (holder, store) = slot();
    let self_sender = Rc::clone(&holder);
    let mut app = App::new(config(), move || {
        let sender = channel::channel(move |value: u32| {
            set_seen.update(|seen| seen.push(value));
            if value == 1 {
                // The queue still holds the two values this frame is about
                // to take, so this send sees a non-empty queue and wakes
                // nothing. The frame must still ask for the one after it.
                if let Some(sender) = self_sender.borrow().clone() {
                    sender.send(99).unwrap();
                }
            }
        });
        store(sender);
        panel()
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    sender.send(1).unwrap();
    sender.send(2).unwrap();
    sender.send(3).unwrap();
    let _ = items(&mut app);

    assert_eq!(*seen.read(|seen| seen.clone()), [1, 2, 3]);
    assert!(
        app.needs_frame(),
        "the value that landed during the drain still waits"
    );

    let _ = items(&mut app);
    assert_eq!(*seen.read(|seen| seen.clone()), [1, 2, 3, 99]);
    assert!(!app.needs_frame(), "the deferred value was delivered");
}

#[test]
fn a_send_after_the_app_dropped_returns_closed() {
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        let sender = channel::channel(|_: u32| panic!("nothing delivers after the drop"));
        store(sender);
        panel()
    });
    let _ = items(&mut app);
    let sender = stored(&holder);
    drop(app);

    let Closed { value } = sender.send(9).expect_err("the app is gone");
    assert_eq!(value, 9, "the value comes back to the sender");
}

#[test]
fn an_empty_channel_needs_no_frame() {
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        let sender = channel::channel(|_: u32| {});
        store(sender);
        panel()
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    assert!(
        !app.needs_frame(),
        "a live channel with an empty queue asks for no frame"
    );
    let idle = app.frame();
    assert!(idle.damage.is_empty(), "an idle channel draws no frame");

    sender.send(1).unwrap();
    assert!(app.needs_frame(), "a queued value asks for a frame");
    let _ = items(&mut app);
    assert!(!app.needs_frame(), "the value was delivered");
}

#[test]
fn the_waker_fires_once_for_a_burst_of_sends() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        let sender = channel::channel(|_: u32| {});
        store(sender);
        panel()
    });
    app.set_remote_waker({
        let wakes = Arc::clone(&wakes);
        move || {
            wakes.fetch_add(1, Ordering::SeqCst);
        }
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    for digit in 1..=5 {
        sender.send(digit).unwrap();
    }
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        1,
        "a burst wakes the window once"
    );

    let _ = items(&mut app);
    sender.send(6).unwrap();
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "the drained queue wakes again for the next value"
    );
}

#[test]
fn channels_drain_before_timers() {
    let clock = VirtualClock::new(Instant::now());
    let order = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let (from_channel, set_from_channel) = signal_local(false);
    let (from_timer, set_from_timer) = signal_local(false);
    let (holder, store) = slot();
    let mut app = App::with_clock(config(), clock.clone().into(), {
        let order = Rc::clone(&order);
        move || {
            let sender = channel::channel(move |_: u32| set_from_channel.set(true));
            store(sender);
            smudge_core::timer::after(Duration::from_secs(1), move || set_from_timer.set(true));
            let channel_order = Rc::clone(&order);
            let _watch_channel = RenderEffect::new(move || {
                if from_channel.get() {
                    channel_order.borrow_mut().push("channel");
                }
            });
            let timer_order = Rc::clone(&order);
            let _watch_timer = RenderEffect::new(move || {
                if from_timer.get() {
                    timer_order.borrow_mut().push("timer");
                }
            });
            panel()
        }
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    clock.advance(Duration::from_secs(1));
    sender.send(0).unwrap();
    let _ = items(&mut app);

    assert_eq!(
        *order.borrow(),
        ["channel", "timer"],
        "the frame delivers messages before it runs due timers"
    );
}

#[test]
fn a_channel_inside_an_unmounted_child_closes_with_it() {
    let (received, set_received) = signal_local(0u32);
    let (shown, set_shown) = signal_local(true);
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        panel().child(show(
            move || shown.get(),
            move || {
                let set = set_received.clone();
                let sender = channel::channel(move |value: u32| set.set(value));
                store(sender);
                panel().width(px(20.0)).height(px(10.0))
            },
        ))
    });
    let _ = items(&mut app);
    let sender = stored(&holder);

    sender.send(1).unwrap();
    let _ = items(&mut app);
    assert_eq!(
        received.get(),
        1,
        "the row's channel delivers while the row is mounted"
    );

    set_shown.set(false);
    let _ = items(&mut app);

    let Closed { value } = sender.send(2).expect_err("the row unmounted");
    assert_eq!(value, 2, "the value comes back");
}

#[test]
fn a_value_queued_behind_an_undelivered_one_during_the_drain_asks_for_the_next_frame() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let (holder, store) = slot();
    let self_sender = Rc::clone(&holder);
    let mut app = App::new(config(), move || {
        let sender = channel::channel(move |value: u32| {
            if value == 1
                && let Some(sender) = self_sender.borrow().clone()
            {
                sender.send(3).unwrap();
            }
        });
        store(sender);
        panel()
    });
    app.set_remote_waker({
        let wakes = Arc::clone(&wakes);
        move || {
            wakes.fetch_add(1, Ordering::SeqCst);
        }
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    sender.send(1).unwrap();
    sender.send(2).unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);

    // The handler sends while 2 still waits, so its send finds the queue
    // holding a value and wakes nothing. The drain must ask for the frame
    // that delivers it.
    let _ = items(&mut app);
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "the drain asks for the frame that delivers the value it left behind"
    );
    assert!(app.needs_frame());
}

#[test]
fn a_handler_that_panics_closes_its_channel() {
    let (holder, store) = slot();
    let mut app = App::new(config(), move || {
        let sender = channel::channel(|value: u32| {
            assert_ne!(value, 13, "the handler rejects 13");
        });
        store(sender);
        panel()
    });
    let sender = stored(&holder);
    let _ = items(&mut app);

    sender.send(13).unwrap();
    sender.send(14).unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app.frame();
    }));
    assert!(unwound.is_err(), "the handler panicked out of the frame");

    assert!(
        !app.needs_frame(),
        "the values behind the panic are dropped, not left for a frame that cannot deliver them"
    );
    assert!(matches!(sender.send(15), Err(Closed { value: 15 })));
}
