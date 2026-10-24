//! Behavior tests for hit testing, hover and press.
//!
//! Every test drives the same `App::handle_event` a window does, through the
//! harness, and observes the handlers the tree runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::element::IntoElement;
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{InputEvent, PointerButton};
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;

/// A counter shared with a handler.
type Count = Rc<Cell<u32>>;

/// A counter with a handler closure that increments it.
fn press_counter() -> (Count, impl FnMut()) {
    let count = Rc::new(Cell::new(0u32));
    let handler = {
        let count = Rc::clone(&count);
        move || count.set(count.get() + 1)
    };
    (count, handler)
}

/// A log of hover changes, shared between a test and its handler.
type HoverLog = Rc<RefCell<Vec<(&'static str, bool)>>>;

/// A log of hover changes with a handler closure that appends to it.
fn hover_log(name: &'static str) -> (HoverLog, impl FnMut(bool)) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let handler = {
        let log = Rc::clone(&log);
        move |hovered: bool| log.borrow_mut().push((name, hovered))
    };
    (log, handler)
}

/// Wraps a node under test in a plain parent.
///
/// The layout pass sizes the root to the whole viewport, whatever its style
/// says, so a node with a size of its own has to sit under the root.
fn scene(child: impl IntoElement) -> impl IntoElement {
    panel().child(child)
}

#[test]
fn a_click_on_a_panel_presses_it_once() {
    let (presses, on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .on_press(on_press),
        )
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 5.0 });
    assert_eq!(presses.get(), 1, "a click inside the panel presses it");

    harness.click(Point { x: 80.0, y: 50.0 });
    assert_eq!(
        presses.get(),
        1,
        "a click outside the panel presses nothing"
    );
}

#[test]
fn pressing_moving_off_and_releasing_does_not_press() {
    let (presses, on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .on_press(on_press),
        )
    });
    harness.frame();

    let inside = Point { x: 10.0, y: 5.0 };
    let outside = Point { x: 80.0, y: 50.0 };
    harness.event(InputEvent::PointerDown {
        position: inside,
        button: PointerButton::Primary,
    });
    harness.move_to(outside);
    harness.event(InputEvent::PointerUp {
        position: outside,
        button: PointerButton::Primary,
    });
    assert_eq!(presses.get(), 0, "the release happened off the panel");

    harness.event(InputEvent::PointerDown {
        position: inside,
        button: PointerButton::Primary,
    });
    harness.move_to(outside);
    harness.move_to(inside);
    harness.event(InputEvent::PointerUp {
        position: inside,
        button: PointerButton::Primary,
    });
    assert_eq!(
        presses.get(),
        1,
        "the release happened over the panel again"
    );
}

#[test]
fn a_child_over_its_parent_takes_the_press() {
    let (parent_presses, parent_on_press) = press_counter();
    let (child_presses, child_on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(60.0))
                .background(Color::BLACK)
                .on_press(parent_on_press)
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(40.0))
                        .background(Color::WHITE)
                        .on_press(child_on_press),
                ),
        )
    });
    harness.frame();

    harness.click(Point { x: 30.0, y: 30.0 });
    assert_eq!(
        child_presses.get(),
        1,
        "the child is painted above the parent"
    );
    assert_eq!(
        parent_presses.get(),
        0,
        "the press does not reach the parent"
    );
}

#[test]
fn a_node_is_not_hit_beyond_its_parents_clip() {
    let (presses, on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel().width(px(40.0)).height(px(40.0)).clip(true).child(
                panel()
                    .width(px(80.0))
                    .height(px(80.0))
                    .background(Color::BLACK)
                    .on_press(on_press),
            ),
        )
    });
    harness.frame();

    harness.click(Point { x: 20.0, y: 10.0 });
    assert_eq!(presses.get(), 1, "the visible part of the child is hit");

    harness.click(Point { x: 60.0, y: 10.0 });
    assert_eq!(
        presses.get(),
        1,
        "the part of the child the parent clips away is not hit"
    );
}

#[test]
fn a_disabled_node_receives_no_presses() {
    let (disabled_presses, disabled_on_press) = press_counter();
    let (enabled_presses, enabled_on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(80.0))
                .height(px(40.0))
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(40.0))
                        .background(Color::BLACK)
                        .disabled(true)
                        .on_press(disabled_on_press),
                )
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(40.0))
                        .background(Color::rgb(200, 60, 60))
                        .on_press(enabled_on_press),
                ),
        )
    });
    harness.frame();

    harness.click(Point { x: 20.0, y: 20.0 });
    assert_eq!(disabled_presses.get(), 0, "the disabled node is not hit");

    harness.click(Point { x: 60.0, y: 20.0 });
    assert_eq!(enabled_presses.get(), 1, "the enabled sibling is hit");
}

#[test]
fn moving_between_siblings_swaps_hover_in_order() {
    let (left_log, left_on_hover) = hover_log("left");
    let (right_log, right_on_hover) = hover_log("right");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(80.0))
                .height(px(20.0))
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .background(Color::BLACK)
                        .on_hover(left_on_hover),
                )
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .background(Color::WHITE)
                        .on_hover(right_on_hover),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 20.0, y: 10.0 });
    assert_eq!(*left_log.borrow(), [("left", true)]);
    assert!(right_log.borrow().is_empty());

    harness.move_to(Point { x: 60.0, y: 10.0 });
    assert_eq!(*left_log.borrow(), [("left", true), ("left", false)]);
    assert_eq!(*right_log.borrow(), [("right", true)]);
}

#[test]
fn moves_during_a_held_press_stay_with_the_captured_node() {
    let (presses, on_press) = press_counter();
    let (log, on_hover) = hover_log("panel");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .on_press(on_press)
                .on_hover(on_hover),
        )
    });
    harness.frame();

    let inside = Point { x: 10.0, y: 5.0 };
    let outside = Point { x: 80.0, y: 50.0 };
    harness.move_to(inside);
    assert_eq!(*log.borrow(), [("panel", true)]);

    harness.event(InputEvent::PointerDown {
        position: inside,
        button: PointerButton::Primary,
    });
    harness.move_to(outside);
    assert_eq!(
        *log.borrow(),
        [("panel", true)],
        "the captured node keeps the pointer while the press is held"
    );

    harness.event(InputEvent::PointerUp {
        position: outside,
        button: PointerButton::Primary,
    });
    assert_eq!(
        *log.borrow(),
        [("panel", true), ("panel", false)],
        "the pointer follows the cursor again after the release"
    );
    assert_eq!(
        presses.get(),
        0,
        "the release happened off the captured node"
    );
}

#[test]
fn a_press_outside_a_popup_runs_its_outside_handler() {
    let (outside_presses, on_press_outside) = press_counter();
    let (child_presses, child_on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(30.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .on_press_outside(on_press_outside)
                .child(
                    panel()
                        .width(px(10.0))
                        .height(px(10.0))
                        .background(Color::WHITE)
                        .on_press(child_on_press),
                ),
        )
    });
    harness.frame();

    harness.click(Point { x: 5.0, y: 5.0 });
    assert_eq!(child_presses.get(), 1);
    assert_eq!(
        outside_presses.get(),
        0,
        "a press on a descendant is not outside the popup"
    );

    harness.click(Point { x: 20.0, y: 15.0 });
    assert_eq!(
        outside_presses.get(),
        0,
        "a press on the popup itself is not outside it"
    );

    harness.click(Point { x: 60.0, y: 30.0 });
    assert_eq!(outside_presses.get(), 1, "a press elsewhere is outside");
}

#[test]
fn another_button_during_a_held_press_keeps_the_capture() {
    let (log, on_hover) = hover_log("panel");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .on_press(|| {})
                .on_hover(on_hover),
        )
    });
    harness.frame();

    let inside = Point { x: 10.0, y: 5.0 };
    let outside = Point { x: 80.0, y: 50.0 };
    harness.move_to(inside);
    harness.event(InputEvent::PointerDown {
        position: inside,
        button: PointerButton::Primary,
    });
    harness.event(InputEvent::PointerDown {
        position: outside,
        button: PointerButton::Secondary,
    });

    assert_eq!(
        *log.borrow(),
        [("panel", true)],
        "a secondary press while the primary is held leaves the captured node hovered"
    );
}

#[test]
fn a_press_on_the_content_of_a_disabled_node_runs_none_of_its_handlers() {
    let (presses, on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(40.0))
                .on_press(on_press)
                .disabled(true)
                .child(panel().width(px(10.0)).height(px(10.0))),
        )
    });
    harness.frame();

    harness.click(Point { x: 5.0, y: 5.0 });

    assert_eq!(
        presses.get(),
        0,
        "a disabled node swallows a press on its content"
    );
}

#[test]
fn a_container_stays_hovered_over_a_disabled_control_inside_it() {
    let (log, on_card_hover) = hover_log("card");
    let (control_log, on_control_hover) = hover_log("control");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .on_hover(on_card_hover)
                .child(
                    panel()
                        .width(px(30.0))
                        .height(px(30.0))
                        .focusable()
                        .on_hover(on_control_hover)
                        .disabled(true)
                        .child(panel().width(px(10.0)).height(px(10.0))),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 50.0, y: 35.0 });
    harness.move_to(Point { x: 5.0, y: 5.0 });
    harness.move_to(Point { x: 50.0, y: 35.0 });

    assert_eq!(
        *log.borrow(),
        [("card", true)],
        "the card stays hovered while the pointer crosses the disabled control"
    );
    assert!(
        control_log.borrow().is_empty(),
        "a disabled control hears no hover"
    );
}

#[test]
fn a_press_released_over_a_disabled_control_inside_the_node_still_presses_it() {
    let (presses, on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .on_press(on_press)
                .child(
                    panel()
                        .width(px(30.0))
                        .height(px(30.0))
                        .focusable()
                        .disabled(true)
                        .child(panel().width(px(10.0)).height(px(10.0))),
                ),
        )
    });
    harness.frame();

    harness.event(InputEvent::PointerDown {
        position: Point { x: 50.0, y: 35.0 },
        button: PointerButton::Primary,
    });
    harness.event(InputEvent::PointerUp {
        position: Point { x: 5.0, y: 5.0 },
        button: PointerButton::Primary,
    });

    assert_eq!(
        presses.get(),
        1,
        "the release landed inside the pressed card"
    );
}

#[test]
fn a_node_disabled_during_its_press_does_not_run_on_press() {
    let (disabled, set_disabled) = reactive::signal_local(false);
    let (presses, on_press) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .on_press(on_press)
                .disabled(move || disabled.get())
                .child(panel().width(px(20.0)).height(px(20.0)).on_hover(|_| {})),
        )
    });
    harness.frame();

    harness.event(InputEvent::PointerDown {
        position: Point { x: 50.0, y: 35.0 },
        button: PointerButton::Primary,
    });
    set_disabled.set(true);
    harness.frame();
    harness.event(InputEvent::PointerUp {
        position: Point { x: 10.0, y: 10.0 },
        button: PointerButton::Primary,
    });

    assert_eq!(
        presses.get(),
        0,
        "a node disabled mid-press runs no on_press"
    );
}

#[test]
fn a_hidden_or_disabled_node_hears_no_press_outside() {
    let (hidden_presses, on_hidden_outside) = press_counter();
    let (disabled_presses, on_disabled_outside) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(20.0))
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .on_press_outside(on_hidden_outside)
                        .hidden(true),
                )
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .on_press_outside(on_disabled_outside)
                        .disabled(true),
                ),
        )
    });
    harness.frame();

    harness.click(Point { x: 80.0, y: 50.0 });

    assert_eq!(
        hidden_presses.get(),
        0,
        "a closed popup hears no outside press"
    );
    assert_eq!(
        disabled_presses.get(),
        0,
        "a disabled node's handlers never run"
    );
}
