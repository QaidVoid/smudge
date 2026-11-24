//! Behavior tests for the drag and drop of one app: a source's payload
//! reaching the target that accepts it, the highlight and the preview while
//! it is in the air, and every way a drag ends without a drop.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{Element, show};
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{InputEvent, Key};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

type Count = Rc<Cell<u32>>;
type Ended = Rc<Cell<Option<bool>>>;
type Dropped = Rc<Cell<Option<(u32, Point)>>>;
type OtherDropped = Rc<Cell<Option<(u64, Point)>>>;

/// Places a node at `left`, `top` of its parent.
fn at(left: f32, top: f32) -> Edges<Length> {
    Edges {
        left: Length::px(left),
        top: Length::px(top),
        right: Length::Auto,
        bottom: Length::Auto,
    }
}

fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

/// A source panel at (10, 10) carrying a number, with its press and drag-end
/// records beside it.
fn source(pressed: Count, ended: Ended) -> Element<Panel> {
    panel()
        .absolute()
        .width(px(40.0))
        .height(px(20.0))
        .inset(at(10.0, 10.0))
        .background(Color::rgb(200, 30, 30))
        .drag_source(|| 7u32)
        .on_press(move || pressed.set(pressed.get() + 1))
        .on_drag_end(move |dropped| ended.set(Some(dropped)))
}

/// A target panel of type `T` at `top_left` that records what lands on it.
fn target<T: Clone + 'static>(
    top_left: Point,
    size: (f32, f32),
    dropped: Rc<Cell<Option<(T, Point)>>>,
) -> Element<Panel> {
    panel()
        .absolute()
        .width(px(size.0))
        .height(px(size.1))
        .inset(at(top_left.x, top_left.y))
        .background(Color::rgb(30, 30, 30))
        .drop_target::<T>(
            |_| true,
            move |payload, at| dropped.set(Some((payload, at))),
        )
}

#[test]
fn a_drag_past_the_threshold_reaches_a_target_of_its_type() {
    let pressed: Count = Rc::new(Cell::new(0));
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let (view_p, view_e, view_d) = (pressed.clone(), ended.clone(), dropped.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(view_p.clone(), view_e.clone()))
            .child(target::<u32>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(21.0, 20.0));
    harness.frame();
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    harness.button_up(point(150.0, 60.0));
    harness.frame();

    assert_eq!(
        dropped.get(),
        Some((7, point(30.0, 20.0))),
        "the payload lands at its position in the target"
    );
    assert_eq!(ended.get(), Some(true), "the source hears it was taken");
    assert_eq!(pressed.get(), 0, "a drag is not a press");
    assert_eq!(
        harness.pixel(150, 60),
        Some([30, 30, 30, 255]),
        "the preview is gone when the drag ends"
    );
}

#[test]
fn a_release_over_a_refusing_target_cancels_the_drag() {
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: OtherDropped = Rc::new(Cell::new(None));
    let (view_e, view_d) = (ended.clone(), dropped.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(Rc::new(Cell::new(0)), view_e.clone()))
            .child(target::<u64>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    harness.button_up(point(150.0, 60.0));
    harness.frame();

    assert_eq!(dropped.get(), None, "a target of another type takes none");
    assert_eq!(ended.get(), Some(false), "the source hears it was refused");
}

#[test]
fn escape_cancels_a_drag_in_the_air() {
    let pressed: Count = Rc::new(Cell::new(0));
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let (view_p, view_e, view_d) = (pressed.clone(), ended.clone(), dropped.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(view_p.clone(), view_e.clone()))
            .child(target::<u32>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    harness.press_key(Key::Escape);
    harness.button_up(point(20.0, 20.0));
    harness.frame();

    assert_eq!(dropped.get(), None, "a cancelled drag drops nothing");
    assert_eq!(ended.get(), Some(false), "the source hears the cancel");
    assert_eq!(pressed.get(), 0, "the release after a cancel runs no press");
    assert_eq!(
        harness.pixel(150, 60),
        Some([30, 30, 30, 255]),
        "the preview left with the drag"
    );
}

#[test]
fn a_fresh_press_discards_a_drag_that_never_left_its_source() {
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let small: Dropped = Rc::new(Cell::new(None));
    let pressed: Count = Rc::new(Cell::new(0));
    let (view_e, view_d, view_s, view_p) = (
        ended.clone(),
        dropped.clone(),
        small.clone(),
        pressed.clone(),
    );
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(Rc::new(Cell::new(0)), view_e.clone()))
            // A target that takes presses painted over the source's corner,
            // so a press within the drag threshold can land on a node that
            // carries no source.
            .child(
                panel()
                    .absolute()
                    .width(px(20.0))
                    .height(px(20.0))
                    .inset(at(13.0, 13.0))
                    .background(Color::rgb(30, 30, 30))
                    .on_press(move || view_p.set(view_p.get() + 1))
                    .drop_target::<u32>(
                        |_| true,
                        move |payload, at| view_s.set(Some((payload, at))),
                    ),
            )
            .child(target::<u32>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(12.0, 12.0));
    harness.button_down(point(14.0, 14.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    harness.button_up(point(150.0, 60.0));
    harness.frame();

    assert_eq!(
        dropped.get(),
        None,
        "the replaced press takes its armed drag with it"
    );
    assert_eq!(small.get(), None);
    assert_eq!(ended.get(), None, "and the old source hears nothing");
}

#[test]
fn a_drag_keeps_flying_when_its_source_is_removed() {
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let present = RwSignal::new(true);
    let (view_e, view_d, view_present) = (ended.clone(), dropped.clone(), present.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(show(move || view_present.get(), {
                let view_e = view_e.clone();
                move || source(Rc::new(Cell::new(0)), view_e.clone())
            }))
            .child(
                panel()
                    .absolute()
                    .width(px(60.0))
                    .height(px(40.0))
                    .inset(at(120.0, 40.0))
                    .background(Color::rgb(30, 30, 30))
                    .drop_style(|p| p.background(Color::rgb(40, 90, 40)))
                    .drop_target::<u32>(
                        |_| true,
                        move |payload, at| view_d.set(Some((payload, at))),
                    ),
            )
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    present.set(false);
    harness.frame();
    harness.move_to(point(60.0, 60.0));
    harness.frame();
    assert_eq!(
        harness.pixel(130, 50),
        Some([30, 30, 30, 255]),
        "the drop look leaves when the drag moves off the target"
    );
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    assert_eq!(harness.pixel(130, 50), Some([40, 90, 40, 255]));
    harness.button_up(point(150.0, 60.0));
    harness.frame();

    assert_eq!(
        dropped.get(),
        Some((7, point(30.0, 20.0))),
        "the payload lands even though the source is gone"
    );
}

#[test]
fn a_press_that_never_leaves_its_source_stays_a_press() {
    let pressed: Count = Rc::new(Cell::new(0));
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let (view_p, view_e, view_d) = (pressed.clone(), ended.clone(), dropped.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(view_p.clone(), view_e.clone()))
            .child(target::<u32>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(22.0, 20.0));
    harness.frame();
    harness.button_up(point(22.0, 20.0));
    harness.frame();

    assert_eq!(pressed.get(), 1, "a click on a source is a press");
    assert_eq!(ended.get(), None, "no drag ever started");
    assert_eq!(dropped.get(), None);
}

#[test]
fn a_drag_held_over_a_target_highlights_it_while_it_accepts() {
    let dropped: Dropped = Rc::new(Cell::new(None));
    let view_d = dropped.clone();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(Rc::new(Cell::new(0)), Rc::new(Cell::new(None))))
            .child(
                panel()
                    .absolute()
                    .width(px(60.0))
                    .height(px(40.0))
                    .inset(at(120.0, 40.0))
                    .background(Color::rgb(30, 30, 30))
                    .drop_style(|p| p.background(Color::rgb(40, 90, 40)))
                    .drop_target::<u32>(
                        |_| true,
                        move |payload, at| view_d.set(Some((payload, at))),
                    ),
            )
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    // The preview covers the pointer, so the highlight is read away from it.
    assert_eq!(
        harness.pixel(130, 50),
        Some([40, 90, 40, 255]),
        "the accepting target shows its drop look"
    );

    harness.move_to(point(60.0, 60.0));
    harness.frame();
    assert_eq!(
        harness.pixel(130, 50),
        Some([30, 30, 30, 255]),
        "the look leaves with the drag"
    );

    harness.move_to(point(150.0, 60.0));
    harness.frame();
    assert_eq!(
        harness.pixel(130, 50),
        Some([40, 90, 40, 255]),
        "and comes back when the drag returns"
    );
    harness.button_up(point(150.0, 60.0));
    harness.frame();
    assert_eq!(dropped.get(), Some((7, point(30.0, 20.0))));
}

#[test]
fn the_deepest_accepting_target_takes_the_drop() {
    let outer: Dropped = Rc::new(Cell::new(None));
    let inner: Dropped = Rc::new(Cell::new(None));
    let (view_o, view_i) = (outer.clone(), inner.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(Rc::new(Cell::new(0)), Rc::new(Cell::new(None))))
            .child(
                panel()
                    .absolute()
                    .width(px(60.0))
                    .height(px(40.0))
                    .inset(at(120.0, 40.0))
                    .background(Color::rgb(30, 30, 30))
                    .drop_target::<u32>(
                        |_| true,
                        move |payload, at| view_o.set(Some((payload, at))),
                    )
                    .child(target::<u32>(
                        point(10.0, 10.0),
                        (30.0, 20.0),
                        view_i.clone(),
                    )),
            )
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(145.0, 60.0));
    harness.frame();
    harness.button_up(point(145.0, 60.0));
    harness.frame();

    assert_eq!(
        inner.get(),
        Some((7, point(15.0, 10.0))),
        "the inner zone takes the drop"
    );
    assert_eq!(outer.get(), None, "and the one around it hears nothing");
}

#[test]
fn the_preview_follows_the_pointer_and_leaves_when_the_drag_ends() {
    let ended: Ended = Rc::new(Cell::new(None));
    let view_e = ended.clone();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().child(source(Rc::new(Cell::new(0)), view_e.clone()))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(90.0, 70.0));
    harness.frame();
    // The preview draws the source's surface at 0.6 over the white canvas.
    assert_eq!(
        harness.pixel(90, 70),
        Some([222, 120, 120, 255]),
        "the preview sits at the pointer"
    );
    assert_eq!(
        harness.pixel(20, 20),
        Some([200, 30, 30, 255]),
        "and the source stays where it is"
    );

    harness.move_to(point(100.0, 80.0));
    harness.frame();
    assert_eq!(harness.pixel(100, 80), Some([222, 120, 120, 255]));
    assert_eq!(harness.pixel(90, 70), Some([255, 255, 255, 255]));

    harness.button_up(point(100.0, 80.0));
    harness.frame();
    assert_eq!(harness.pixel(100, 80), Some([255, 255, 255, 255]));
    assert_eq!(ended.get(), Some(false), "the canvas takes no drop");
}

#[test]
fn a_source_disabled_while_pressed_flies_no_drag() {
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let pressed: Count = Rc::new(Cell::new(0));
    let armed = RwSignal::new(true);
    let (view_p, view_e, view_d, view_armed) = (
        pressed.clone(),
        ended.clone(),
        dropped.clone(),
        armed.clone(),
    );
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(
                panel()
                    .absolute()
                    .width(px(40.0))
                    .height(px(20.0))
                    .inset(at(10.0, 10.0))
                    .background(Color::rgb(200, 30, 30))
                    .disabled(move || !view_armed.get())
                    .drag_source(|| 7u32)
                    .on_press(move || view_p.set(view_p.get() + 1))
                    .on_drag_end(move |dropped| view_e.set(Some(dropped))),
            )
            .child(target::<u32>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    armed.set(false);
    harness.frame();
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    harness.button_up(point(20.0, 20.0));
    harness.frame();

    assert_eq!(dropped.get(), None, "a disabled source flies no drag");
    assert_eq!(ended.get(), None);
    assert_eq!(pressed.get(), 0, "and its release runs no press");
}

#[test]
fn a_drag_that_leaves_the_window_relights_its_target_on_reentry() {
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let (view_e, view_d) = (ended.clone(), dropped.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(Rc::new(Cell::new(0)), view_e.clone()))
            .child(
                panel()
                    .absolute()
                    .width(px(60.0))
                    .height(px(40.0))
                    .inset(at(120.0, 40.0))
                    .background(Color::rgb(30, 30, 30))
                    .drop_style(|p| p.background(Color::rgb(40, 90, 40)))
                    .drop_target::<u32>(
                        |_| true,
                        move |payload, at| view_d.set(Some((payload, at))),
                    ),
            )
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    assert_eq!(harness.pixel(130, 50), Some([40, 90, 40, 255]));
    harness.event(InputEvent::PointerLeft);
    harness.frame();
    assert_eq!(
        harness.pixel(130, 50),
        Some([30, 30, 30, 255]),
        "the pointer is gone, so the target keeps nothing lit"
    );
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    assert_eq!(
        harness.pixel(130, 50),
        Some([40, 90, 40, 255]),
        "the look comes back when the drag returns"
    );
    harness.button_up(point(150.0, 60.0));
    harness.frame();

    assert_eq!(
        dropped.get(),
        Some((7, point(30.0, 20.0))),
        "the release after the round trip still drops"
    );
    assert_eq!(ended.get(), Some(true));
}

#[test]
fn the_window_losing_the_press_ends_the_drag() {
    let ended: Ended = Rc::new(Cell::new(None));
    let dropped: Dropped = Rc::new(Cell::new(None));
    let (view_e, view_d) = (ended.clone(), dropped.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(source(Rc::new(Cell::new(0)), view_e.clone()))
            .child(target::<u32>(
                point(120.0, 40.0),
                (60.0, 40.0),
                view_d.clone(),
            ))
    });
    harness.frame();

    harness.button_down(point(20.0, 20.0));
    harness.move_to(point(150.0, 60.0));
    harness.frame();
    harness.app().release_press();
    harness.button_up(point(150.0, 60.0));
    harness.frame();

    assert_eq!(dropped.get(), None, "a drag that lost its press drops none");
    assert_eq!(ended.get(), Some(false), "the source hears it was refused");
}
