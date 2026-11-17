//! Behavior tests for hit testing, hover and press.
//!
//! Every test drives the same `App::handle_event` a window does, through the
//! harness, and observes the handlers the tree runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::element::{IntoElement, show};
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{Drag, Handled, InputEvent, PointerButton};
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::{Overflow, px};
use smudge_core::widget::panel;
use smudge_core::window::Cursor;
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

/// A log of pointer positions, shared between a test and its handler.
type PointLog = Rc<RefCell<Vec<Point>>>;

/// A log of positions with a handler closure that appends to it.
fn point_log() -> (PointLog, impl FnMut(Point)) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let handler = {
        let log = Rc::clone(&log);
        move |position: Point| log.borrow_mut().push(position)
    };
    (log, handler)
}

#[test]
fn a_drag_sends_every_move_and_the_release_to_the_pressed_node() {
    let (moves, on_move) = point_log();
    let (ups, on_up) = point_log();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .row()
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .on_pointer_move(on_move)
                        .on_pointer_up(on_up),
                )
                .child(panel().width(px(40.0)).height(px(20.0)).on_press(|| {})),
        )
    });
    harness.frame();

    let inside = Point { x: 10.0, y: 5.0 };
    let beside = Point { x: 60.0, y: 5.0 };
    let outside = Point { x: 90.0, y: 50.0 };
    harness.button_down(inside);
    harness.move_to(beside);
    harness.event(InputEvent::PointerLeft);
    harness.move_to(outside);
    harness.button_up(outside);
    assert_eq!(
        *moves.borrow(),
        [inside, beside, outside],
        "the pressed node hears every move of the drag, over a sibling and \
         after the pointer left and came back"
    );
    assert_eq!(
        *ups.borrow(),
        [outside],
        "the release reaches the pressed node away from it"
    );

    moves.borrow_mut().clear();
    harness.move_to(beside);
    harness.move_to(inside);
    assert_eq!(
        *moves.borrow(),
        [inside],
        "with no press held, only the node under the pointer hears a move"
    );
}

#[test]
fn a_press_held_across_a_scroll_hears_positions_from_where_the_node_went() {
    let (moves, on_move) = point_log();
    let (ups, on_up) = point_log();
    let (offset, set_offset) = reactive::signal_local(0.0_f32);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .height(px(40.0))
                .clip(true)
                .scroll_offset(move || Point {
                    x: offset.get(),
                    y: 0.0,
                })
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .on_press(|| {})
                        .on_pointer_move(on_move)
                        .on_pointer_up(on_up),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 20.0, y: 5.0 });
    set_offset.set(10.0);
    harness.frame();
    harness.move_to(Point { x: 20.0, y: 5.0 });
    harness.button_up(Point { x: 20.0, y: 5.0 });

    assert_eq!(
        *moves.borrow(),
        [Point { x: 20.0, y: 5.0 }, Point { x: 30.0, y: 5.0 }],
        "the node moved 10 to the left under the held pointer"
    );
    assert_eq!(*ups.borrow(), [Point { x: 30.0, y: 5.0 }]);
}

#[test]
fn pointer_handlers_measure_from_the_node_and_follow_its_scroll() {
    let (downs, on_down) = point_log();
    let (moves, on_move) = point_log();
    let (ups, on_up) = point_log();
    let (offset, set_offset) = reactive::signal_local(0.0_f32);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .padding(px(10.0))
                .height(px(40.0))
                .clip(true)
                .scroll_offset(move || Point {
                    x: offset.get(),
                    y: 0.0,
                })
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .margin(px(5.0))
                        .on_pointer_down(on_down)
                        .on_pointer_move(on_move)
                        .on_pointer_up(on_up),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 18.0, y: 17.0 });
    harness.move_to(Point { x: 30.0, y: 20.0 });
    harness.button_up(Point { x: 30.0, y: 20.0 });
    set_offset.set(10.0);
    harness.frame();
    harness.button_down(Point { x: 18.0, y: 17.0 });

    assert_eq!(
        *downs.borrow(),
        [Point { x: 3.0, y: 2.0 }, Point { x: 13.0, y: 2.0 }],
        "a press is relative to the node, which the scroll moved 10 to the left"
    );
    assert_eq!(
        *moves.borrow(),
        [
            Point { x: 3.0, y: 2.0 },
            Point { x: 15.0, y: 5.0 },
            Point { x: 13.0, y: 2.0 }
        ],
        "every move a press brings is relative to the node too"
    );
    assert_eq!(*ups.borrow(), [Point { x: 15.0, y: 5.0 }]);
}

#[test]
fn a_second_press_without_a_release_ends_the_first_drag() {
    let (ups, on_up) = point_log();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .row()
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .on_pointer_up(on_up),
                )
                .child(panel().width(px(40.0)).height(px(20.0)).on_press(|| {})),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 5.0 });
    harness.button_down(Point { x: 60.0, y: 5.0 });

    assert_eq!(
        *ups.borrow(),
        [Point { x: 60.0, y: 5.0 }],
        "the first node hears its press end where the second press landed"
    );
}

#[test]
fn a_disabled_node_hears_no_moves_or_release() {
    let (moves, on_move) = point_log();
    let (ups, on_up) = point_log();
    let (disabled, set_disabled) = reactive::signal_local(false);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .disabled(move || disabled.get())
                .on_pointer_move(on_move)
                .on_pointer_up(on_up),
        )
    });
    harness.frame();

    let inside = Point { x: 10.0, y: 5.0 };
    harness.button_down(inside);
    set_disabled.set(true);
    harness.frame();
    harness.move_to(Point { x: 20.0, y: 5.0 });
    harness.button_up(inside);
    assert_eq!(*moves.borrow(), [inside], "only the move before disabling");
    assert!(ups.borrow().is_empty(), "a disabled node hears no release");
}

#[test]
fn removing_the_pressed_node_lets_hover_follow_the_pointer_again() {
    let (shown, set_shown) = reactive::signal_local(true);
    let (log, on_hover) = hover_log("sibling");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .row()
                .child(show(
                    move || shown.get(),
                    || panel().width(px(40.0)).height(px(20.0)).on_press(|| {}),
                ))
                .child(panel().width(px(40.0)).height(px(20.0)).on_hover(on_hover)),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 5.0 });
    set_shown.set(false);
    harness.frame();
    harness.move_to(Point { x: 10.0, y: 5.0 });
    assert_eq!(
        *log.borrow(),
        [("sibling", true)],
        "the sibling took the place of the removed node and is hovered"
    );
}

#[test]
fn a_wheel_handler_that_declines_lets_the_event_reach_its_ancestor() {
    let outer = Rc::new(Cell::new(0u32));
    let inner = Rc::new(Cell::new(0u32));
    let take = Rc::new(Cell::new(false));
    let (outer_count, inner_count, inner_takes) =
        (Rc::clone(&outer), Rc::clone(&inner), Rc::clone(&take));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let outer_count = Rc::clone(&outer_count);
        let inner_count = Rc::clone(&inner_count);
        let inner_takes = Rc::clone(&inner_takes);
        scene(
            panel()
                .width(px(80.0))
                .height(px(40.0))
                .on_wheel(move |_| {
                    outer_count.set(outer_count.get() + 1);
                    Handled::Yes
                })
                .child(panel().width(px(40.0)).height(px(20.0)).on_wheel(move |_| {
                    inner_count.set(inner_count.get() + 1);
                    if inner_takes.get() {
                        Handled::Yes
                    } else {
                        Handled::No
                    }
                })),
        )
    });
    harness.frame();
    harness.move_to(Point { x: 10.0, y: 5.0 });

    let delta = Point { x: 0.0, y: 10.0 };
    harness.event(InputEvent::Wheel { delta });
    assert_eq!(
        (inner.get(), outer.get()),
        (1, 1),
        "a declined wheel bubbles to the next handler"
    );

    take.set(true);
    harness.event(InputEvent::Wheel { delta });
    assert_eq!(
        (inner.get(), outer.get()),
        (2, 1),
        "a taken wheel stops at the handler that took it"
    );
}

#[test]
fn a_press_outside_a_node_s_region_reaches_its_parent() {
    let (child_presses, on_child) = press_counter();
    let (parent_presses, on_parent) = press_counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(80.0))
                .height(px(40.0))
                .on_press(on_parent)
                .child(
                    panel()
                        .width(px(60.0))
                        .height(px(20.0))
                        .on_press(on_child)
                        .responds_at(|point| point.x < 30.0),
                ),
        )
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 5.0 });
    assert_eq!(child_presses.get(), 1, "a press inside the region takes it");
    assert_eq!(
        parent_presses.get(),
        0,
        "the parent did not hear that press"
    );

    harness.click(Point { x: 50.0, y: 5.0 });
    assert_eq!(
        child_presses.get(),
        1,
        "a press outside the region passed by"
    );
    assert_eq!(parent_presses.get(), 1, "the parent took that press");
}

#[test]
fn hover_outside_a_node_s_region_stays_with_its_parent() {
    let (log, on_parent) = hover_log("parent");
    let child_log = Rc::clone(&log);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let child_log = Rc::clone(&child_log);
        scene(
            panel()
                .width(px(80.0))
                .height(px(40.0))
                .on_hover(on_parent)
                .child(
                    panel()
                        .width(px(60.0))
                        .height(px(20.0))
                        .on_hover(move |hovered| child_log.borrow_mut().push(("child", hovered)))
                        .responds_at(|point| point.x < 30.0),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 50.0, y: 5.0 });
    assert_eq!(
        log.borrow().as_slice(),
        [("parent", true)],
        "outside the region only the parent is hovered"
    );
    harness.move_to(Point { x: 10.0, y: 5.0 });
    assert_eq!(
        log.borrow().as_slice(),
        [("parent", true), ("child", true)],
        "inside the region the child joins the hovered chain"
    );
    harness.move_to(Point { x: 50.0, y: 5.0 });
    assert_eq!(
        log.borrow().as_slice(),
        [("parent", true), ("child", true), ("child", false)],
        "leaving the region leaves the child"
    );
}

#[test]
fn a_node_shows_its_cursor_only_inside_its_region() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(80.0))
                .height(px(40.0))
                .cursor(Cursor::Text)
                .child(
                    panel()
                        .width(px(60.0))
                        .height(px(20.0))
                        .on_press(|| {})
                        .cursor(Cursor::Pointer)
                        .responds_at(|point| point.x < 30.0),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 10.0, y: 5.0 });
    assert_eq!(harness.app().cursor(), Cursor::Pointer, "inside the region");
    harness.move_to(Point { x: 50.0, y: 5.0 });
    assert_eq!(
        harness.app().cursor(),
        Cursor::Text,
        "outside the region the parent's cursor shows"
    );
}

/// A log of the drag-over positions a node heard, by the node's name.
type DragLog = Rc<RefCell<Vec<(&'static str, Point)>>>;

/// A handler that logs the drag-over positions of the node named `name`.
fn drag_over_log(log: &DragLog, name: &'static str) -> impl FnMut(Point) + 'static {
    let log = Rc::clone(log);
    move |at| log.borrow_mut().push((name, at))
}

#[test]
fn drag_over_reaches_the_node_under_the_pointer_while_another_holds_the_press() {
    let log: DragLog = Rc::new(RefCell::new(Vec::new()));
    let (presses, on_press) = press_counter();
    let first = drag_over_log(&log, "first");
    let second = drag_over_log(&log, "second");
    let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(60.0))
                .on_press(on_press)
                .child(panel().width(px(80.0)).height(px(20.0)).on_drag_over(first))
                .child(
                    panel()
                        .width(px(80.0))
                        .height(px(20.0))
                        .on_drag_over(second),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 10.0, y: 5.0 });
    assert!(log.borrow().is_empty(), "no drag without a press");
    harness.button_down(Point { x: 10.0, y: 5.0 });
    harness.move_to(Point { x: 12.0, y: 25.0 });
    harness.button_up(Point { x: 12.0, y: 25.0 });
    harness.move_to(Point { x: 14.0, y: 5.0 });

    assert_eq!(
        log.borrow().as_slice(),
        [
            ("first", Point { x: 10.0, y: 5.0 }),
            ("second", Point { x: 12.0, y: 5.0 }),
        ],
        "the press and the move reach the node under the pointer, and nothing after the release"
    );
    assert_eq!(presses.get(), 1, "the parent still took the press");
}

#[test]
fn a_drag_past_a_clipped_area_reaches_the_content_at_its_edge() {
    let log: DragLog = Rc::new(RefCell::new(Vec::new()));
    let top = drag_over_log(&log, "top");
    let bottom = drag_over_log(&log, "bottom");
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(100.0))
                .on_press(|| {})
                .child(panel().width(px(80.0)).height(px(30.0)))
                .child(
                    panel()
                        .column()
                        .width(px(80.0))
                        .height(px(40.0))
                        .overflow(Overflow::Clip)
                        .child(panel().width(px(80.0)).height(px(20.0)).on_drag_over(top))
                        .child(
                            panel()
                                .width(px(80.0))
                                .height(px(20.0))
                                .on_drag_over(bottom),
                        ),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 35.0 });
    // Above the clipped area, and below it, where no listener lies.
    harness.move_to(Point { x: 10.0, y: 5.0 });
    harness.move_to(Point { x: 10.0, y: 90.0 });
    harness.button_up(Point { x: 10.0, y: 90.0 });

    let log = log.borrow();
    assert_eq!(log[0], ("top", Point { x: 10.0, y: 5.0 }), "the press");
    assert_eq!(
        log[1],
        ("top", Point { x: 10.0, y: 0.0 }),
        "above: the top edge"
    );
    assert_eq!(log[2].0, "bottom", "below: the bottom edge");
    assert!(
        (log[2].1.y - 19.5).abs() < 0.01,
        "below lands a hair inside the bottom edge, not {:?}",
        log[2].1
    );
}

#[test]
fn a_drag_observer_hears_moves_outside_it_and_the_end() {
    let steps: Rc<RefCell<Vec<Drag>>> = Rc::new(RefCell::new(Vec::new()));
    let outside: Rc<RefCell<Vec<Drag>>> = Rc::new(RefCell::new(Vec::new()));
    let observer = Rc::clone(&steps);
    let other = Rc::clone(&outside);
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let observer = Rc::clone(&observer);
        let other = Rc::clone(&other);
        scene(
            panel()
                .column()
                .child(
                    panel()
                        .column()
                        .width(px(60.0))
                        .height(px(40.0))
                        .on_drag(move |step| {
                            observer.borrow_mut().push(step);
                            Handled::No
                        })
                        .child(panel().width(px(60.0)).height(px(20.0)).on_press(|| {})),
                )
                .child(
                    panel()
                        .width(px(60.0))
                        .height(px(40.0))
                        .on_drag(move |step| {
                            other.borrow_mut().push(step);
                            Handled::No
                        }),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 10.0 });
    harness.move_to(Point { x: 10.0, y: 70.0 });
    harness.button_up(Point { x: 10.0, y: 70.0 });

    assert_eq!(
        steps.borrow().as_slice(),
        [Drag::Moved(Point { x: 10.0, y: 70.0 }), Drag::Ended],
        "the drag began inside the observer, which hears it past its edge"
    );
    assert!(
        outside.borrow().is_empty(),
        "the drag began outside the other node"
    );
}

#[test]
fn drag_over_follows_content_that_moves_under_a_still_pointer() {
    let log: DragLog = Rc::new(RefCell::new(Vec::new()));
    let first = drag_over_log(&log, "first");
    let second = drag_over_log(&log, "second");
    let gap = reactive::RwSignal::new(0.0f32);
    let bound = gap.clone();
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let bound = bound.clone();
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(100.0))
                .on_press(|| {})
                .child(panel().width(px(80.0)).height(move || px(bound.get())))
                .child(panel().width(px(80.0)).height(px(20.0)).on_drag_over(first))
                .child(
                    panel()
                        .width(px(80.0))
                        .height(px(20.0))
                        .on_drag_over(second),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 25.0 });
    assert_eq!(log.borrow().last().map(|entry| entry.0), Some("second"));
    // The rows move down under the still pointer, so the first row is under
    // it now.
    gap.set(20.0);
    harness.frame();

    assert_eq!(
        log.borrow().last(),
        Some(&("first", Point { x: 10.0, y: 5.0 })),
        "the moved content heard the drag without a pointer move"
    );
    harness.button_up(Point { x: 10.0, y: 25.0 });
}

/// A log of the drag events the nodes of a test heard, in order.
type EventLog = Rc<RefCell<Vec<String>>>;

/// A drag observer that logs its steps under `name` and answers `answer`.
fn drag_logger(
    log: &EventLog,
    name: &'static str,
    answer: Handled,
) -> impl FnMut(Drag) -> Handled + 'static {
    let log = Rc::clone(log);
    move |step| {
        let entry = match step {
            Drag::Moved(_) => format!("{name} moved"),
            Drag::Taken => format!("{name} taken"),
            _ => format!("{name} ended"),
        };
        log.borrow_mut().push(entry);
        answer
    }
}

#[test]
fn drag_handlers_run_before_the_pressed_node_hears_the_pointer() {
    let log: EventLog = Rc::new(RefCell::new(Vec::new()));
    let (over, observer, down, moved) = (
        Rc::clone(&log),
        drag_logger(&log, "observer", Handled::No),
        Rc::clone(&log),
        Rc::clone(&log),
    );
    let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
        let over = Rc::clone(&over);
        let down = Rc::clone(&down);
        let moved = Rc::clone(&moved);
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(60.0))
                .on_drag(observer)
                .on_pointer_down(move |_| down.borrow_mut().push(String::from("down")))
                .on_pointer_move(move |_| moved.borrow_mut().push(String::from("move")))
                .child(
                    panel()
                        .width(px(80.0))
                        .height(px(20.0))
                        .on_drag_over(move |_| over.borrow_mut().push(String::from("over"))),
                ),
        )
    });
    harness.frame();
    harness.button_down(Point { x: 10.0, y: 5.0 });
    harness.move_to(Point { x: 12.0, y: 6.0 });
    harness.button_up(Point { x: 12.0, y: 6.0 });

    assert_eq!(
        log.borrow().as_slice(),
        // The harness moves the pointer to the press before pressing, which
        // the node hears as a plain move with no drag yet.
        [
            "move",
            "over",
            "down",
            "over",
            "observer moved",
            "move",
            "observer ended"
        ],
    );
}

#[test]
fn an_inner_observer_that_takes_a_move_keeps_it_from_the_outer_one() {
    let log: EventLog = Rc::new(RefCell::new(Vec::new()));
    let outer = drag_logger(&log, "outer", Handled::No);
    let inner = drag_logger(&log, "inner", Handled::Yes);
    let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(60.0))
                .on_press(|| {})
                .on_drag(outer)
                .child(panel().width(px(80.0)).height(px(20.0)).on_drag(inner)),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 5.0 });
    harness.move_to(Point { x: 10.0, y: 50.0 });
    harness.button_up(Point { x: 10.0, y: 50.0 });

    assert_eq!(
        log.borrow().as_slice(),
        ["inner moved", "outer taken", "inner ended", "outer ended"],
        "the inner observer took the move, the outer one heard that, and both heard the end"
    );
}

#[test]
fn a_drag_ends_for_its_observers_however_the_press_goes() {
    // The press is taken away, a second press replaces it, the node that
    // took it goes away, or an observer hides: each observer still hears the
    // end once.
    for case in ["release_press", "replaced", "removed", "hidden"] {
        let log: EventLog = Rc::new(RefCell::new(Vec::new()));
        let observer = drag_logger(&log, "observer", Handled::No);
        let shown = reactive::RwSignal::new(true);
        let hidden = reactive::RwSignal::new(false);
        let (child_shown, view_hidden) = (shown.clone(), hidden.clone());
        let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
            let child_shown = child_shown.clone();
            let view_hidden = view_hidden.clone();
            scene(
                panel()
                    .column()
                    .width(px(80.0))
                    .height(px(60.0))
                    .hidden(move || view_hidden.get())
                    .on_drag(observer)
                    .child(show(
                        move || child_shown.get(),
                        || panel().width(px(80.0)).height(px(20.0)).on_press(|| {}),
                    )),
            )
        });
        harness.frame();
        harness.button_down(Point { x: 10.0, y: 5.0 });
        match case {
            "release_press" => harness.app().release_press(),
            "replaced" => harness.button_down(Point { x: 10.0, y: 6.0 }),
            "removed" => {
                shown.set(false);
                harness.frame();
                harness.button_up(Point { x: 10.0, y: 5.0 });
            }
            _ => {
                hidden.set(true);
                harness.frame();
                harness.button_up(Point { x: 10.0, y: 5.0 });
            }
        }
        assert_eq!(
            log.borrow()
                .iter()
                .filter(|entry| *entry == "observer ended")
                .count(),
            1,
            "{case}: the observer heard the end once, in {:?}",
            log.borrow()
        );
    }
}

#[test]
fn a_disabled_drag_over_listener_passes_the_drag_to_its_parent() {
    let log: DragLog = Rc::new(RefCell::new(Vec::new()));
    let parent = drag_over_log(&log, "parent");
    let child = drag_over_log(&log, "child");
    let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(60.0))
                .on_press(|| {})
                .on_drag_over(parent)
                .child(
                    panel()
                        .width(px(80.0))
                        .height(px(20.0))
                        .disabled(true)
                        .on_drag_over(child),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 5.0 });
    harness.button_up(Point { x: 10.0, y: 5.0 });

    assert_eq!(log.borrow().first().map(|entry| entry.0), Some("parent"));
}

#[test]
fn a_drag_from_a_clipped_block_past_a_scroll_container_reaches_its_edge_row() {
    let log: DragLog = Rc::new(RefCell::new(Vec::new()));
    let block = drag_over_log(&log, "block");
    let edge = drag_over_log(&log, "edge");
    let mut harness = Harness::new(100.0, 200.0, 1.0, move || {
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(200.0))
                .on_press(|| {})
                .child(
                    panel()
                        .column()
                        .width(px(80.0))
                        .height(px(100.0))
                        .overflow(Overflow::Scroll)
                        .child(
                            panel()
                                .width(px(80.0))
                                .height(px(40.0))
                                .overflow(Overflow::Clip)
                                .on_drag_over(block),
                        )
                        .child(panel().width(px(80.0)).height(px(40.0)))
                        .child(panel().width(px(80.0)).height(px(40.0)).on_drag_over(edge)),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 10.0 });
    harness.move_to(Point { x: 10.0, y: 180.0 });
    harness.button_up(Point { x: 10.0, y: 180.0 });

    assert_eq!(
        log.borrow().last().map(|entry| entry.0),
        Some("edge"),
        "the drag past the scroll container reached the row at its edge, not the block's"
    );
}

#[test]
fn a_drag_past_a_bordered_scroll_container_reaches_its_edge_row() {
    let log: DragLog = Rc::new(RefCell::new(Vec::new()));
    let first = drag_over_log(&log, "first");
    let last = drag_over_log(&log, "last");
    let mut harness = Harness::new(100.0, 200.0, 1.0, move || {
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(200.0))
                .on_press(|| {})
                .child(
                    panel()
                        .column()
                        .width(px(80.0))
                        .height(px(84.0))
                        .border(2.0)
                        .overflow(Overflow::Scroll)
                        .child(panel().width(px(76.0)).height(px(40.0)).on_drag_over(first))
                        .child(panel().width(px(76.0)).height(px(40.0)).on_drag_over(last)),
                ),
        )
    });
    harness.frame();

    harness.button_down(Point { x: 10.0, y: 10.0 });
    harness.move_to(Point { x: 10.0, y: 150.0 });
    harness.button_up(Point { x: 10.0, y: 150.0 });

    assert_eq!(
        log.borrow().last().map(|entry| entry.0),
        Some("last"),
        "the drag below the bordered container reached its last row inside the border"
    );
}

#[test]
fn a_press_on_a_node_that_only_scrolls_reaches_its_parent() {
    let (presses, on_press) = press_counter();
    let wheels = Rc::new(Cell::new(0u32));
    let wheeled = Rc::clone(&wheels);
    let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
        let wheeled = Rc::clone(&wheeled);
        scene(
            panel()
                .column()
                .width(px(80.0))
                .height(px(60.0))
                .on_press(on_press)
                .child(panel().width(px(80.0)).height(px(40.0)).on_wheel(move |_| {
                    wheeled.set(wheeled.get() + 1);
                    Handled::Yes
                })),
        )
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 10.0 });
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: 10.0 },
    });

    assert_eq!(presses.get(), 1, "the press passed the wheel-only node");
    assert_eq!(wheels.get(), 1, "and the node still takes the wheel");
}
