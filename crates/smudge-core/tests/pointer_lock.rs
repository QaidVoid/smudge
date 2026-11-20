//! Behavior tests for the pointer lock and the raw motion it delivers.

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::element::{IntoElement, show};
use smudge_core::geometry::Point;
use smudge_core::input::{Handled, InputEvent, Key};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;

/// A log the test reads and a handler writes.
type Log<T> = Rc<RefCell<Vec<T>>>;

/// A view that locks the pointer while `locked` reads `true`, logs the raw
/// motion it hears, and logs the keys that reach it.
fn view(locked: RwSignal<bool>, motion: Log<Point>, keys: Log<Key>) -> impl IntoElement {
    panel().child(
        panel()
            .width(px(40.0))
            .height(px(40.0))
            .focusable()
            .lock_pointer(locked)
            .on_pointer_motion(move |delta| motion.borrow_mut().push(delta))
            .on_key(move |event| {
                if event.pressed {
                    keys.borrow_mut().push(event.key.clone());
                }
                Handled::No
            }),
    )
}

/// A harness showing [`view`], with the signal and the logs it uses.
fn harness() -> (Harness, RwSignal<bool>, Log<Point>, Log<Key>) {
    let locked = RwSignal::new(false);
    let motion = Rc::new(RefCell::new(Vec::new()));
    let keys = Rc::new(RefCell::new(Vec::new()));
    let (signal, moved, keyed) = (locked.clone(), Rc::clone(&motion), Rc::clone(&keys));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        view(signal.clone(), Rc::clone(&moved), Rc::clone(&keyed))
    });
    harness.frame();
    (harness, locked, motion, keys)
}

/// Raw motion of `x` and `y` logical pixels.
fn motion(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerMotion {
        delta: Point { x, y },
    }
}

#[test]
fn the_signal_locks_the_pointer_and_raw_motion_reaches_the_node() {
    let (mut harness, locked, heard, _) = harness();
    harness.event(motion(3.0, 4.0));
    assert!(!harness.app().pointer_locked());
    assert!(
        heard.borrow().is_empty(),
        "no motion reaches an unlocked node"
    );

    locked.set(true);
    assert!(harness.app().pointer_locked());
    harness.event(motion(3.0, -4.0));
    harness.event(motion(500.0, 0.0));
    assert_eq!(
        *heard.borrow(),
        [Point { x: 3.0, y: -4.0 }, Point { x: 500.0, y: 0.0 }],
        "the motion is not clamped to the window"
    );

    locked.set(false);
    assert!(!harness.app().pointer_locked());
}

#[test]
fn escape_ends_the_lock_and_reaches_no_key_handler() {
    let (mut harness, locked, _, keys) = harness();
    harness.click(Point { x: 10.0, y: 10.0 });
    locked.set(true);

    harness.press_key(Key::Escape);
    assert!(!locked.get(), "the app hears the lock end");
    assert!(!harness.app().pointer_locked());
    assert!(
        keys.borrow().is_empty(),
        "the Escape that ended it is spent"
    );

    harness.press_key(Key::Escape);
    assert_eq!(*keys.borrow(), [Key::Escape], "the next one routes");
}

#[test]
fn losing_the_window_focus_ends_the_lock() {
    let (mut harness, locked, _, _) = harness();
    locked.set(true);

    harness.event(InputEvent::WindowFocus(false));

    assert!(!locked.get());
    assert!(!harness.app().pointer_locked());
}

#[test]
fn a_hidden_or_removed_node_holds_no_lock() {
    let locked = RwSignal::new(true);
    let (shown, gone) = (RwSignal::new(true), RwSignal::new(false));
    let (signal, visible, removed) = (locked.clone(), shown.clone(), gone.clone());
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (signal, visible) = (signal.clone(), visible.clone());
        let removed = removed.clone();
        panel().child(show(
            move || !removed.get(),
            move || {
                let visible = visible.clone();
                panel()
                    .width(px(40.0))
                    .height(px(40.0))
                    .hidden(move || !visible.get())
                    .lock_pointer(signal.clone())
            },
        ))
    });
    harness.frame();
    assert!(harness.app().pointer_locked());

    shown.set(false);
    harness.frame();
    assert!(!harness.app().pointer_locked(), "a hidden node lets go");
    shown.set(true);
    harness.frame();
    assert!(
        harness.app().pointer_locked(),
        "and holds it again once shown"
    );

    gone.set(true);
    harness.frame();
    assert!(!harness.app().pointer_locked(), "a removed node lets go");
}

#[test]
fn escape_frees_the_pointer_from_every_node_that_asked() {
    let (first, second) = (RwSignal::new(true), RwSignal::new(true));
    let (one, two) = (first.clone(), second.clone());
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        panel()
            .child(
                panel()
                    .width(px(20.0))
                    .height(px(20.0))
                    .lock_pointer(one.clone()),
            )
            .child(
                panel()
                    .width(px(20.0))
                    .height(px(20.0))
                    .lock_pointer(two.clone()),
            )
    });
    harness.frame();
    assert!(harness.app().pointer_locked());

    harness.press_key(Key::Escape);

    assert!(!first.get() && !second.get());
    assert!(!harness.app().pointer_locked(), "the lock passed to no one");
}
