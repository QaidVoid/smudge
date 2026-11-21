//! Range sliders: two thumbs that never cross, moved by pointer and by
//! keys, and what a screen reader hears.

use smudge_core::element::IntoElement;
use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;
use smudge_widgets::{checkbox, column, range_slider, stack};

/// Returns the projected node named `name`.
fn named(harness: &mut Harness, name: &str) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the window shows {name:?}"))
}

/// Returns the name of the node that holds the focus.
fn focused(harness: &mut Harness) -> Option<String> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
        .and_then(|node| node.label)
}

/// Returns the middle of `node`.
fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: (node.bounds.x0 + node.bounds.x1) / 2.0,
        y: (node.bounds.y0 + node.bounds.y1) / 2.0,
    }
}

/// The window x of `fraction` of the track of a control 204 pixels wide at
/// the window's top left: the track starts the focus ring and half a
/// thumb, 10 pixels in, and runs 184.
fn track_x(fraction: f32) -> f32 {
    10.0 + fraction * 184.0
}

/// A window of 300 by 200 showing `build`'s element at its top left.
fn window<E: IntoElement + 'static>(build: impl Fn() -> E + 'static) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().align_items(Align::Start).child(build())
    });
    harness.frame();
    harness
}

/// A window whose stack holds the control the `build` closure makes at
/// 204 by 20 at the window's top left, so the control runs 184 pixels of
/// track 10 pixels in.
fn slider_window<E: IntoElement + 'static>(build: impl Fn() -> E + 'static) -> Harness {
    window(move || {
        stack()
            .size(Size {
                width: 204.0,
                height: 20.0,
            })
            .child(build())
    })
}

#[test]
fn a_track_press_and_a_drag_move_the_nearer_thumb() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    let y = middle(&named(&mut harness, "minimum")).y;

    harness.click(Point {
        x: track_x(0.21),
        y,
    });
    harness.frame();
    assert_eq!(low.get(), 20.0, "the press moved the nearer, low thumb");
    assert_eq!(high.get(), 70.0, "the press left the high thumb");

    harness.click(Point {
        x: track_x(0.89),
        y,
    });
    harness.frame();
    assert_eq!(high.get(), 90.0, "the press moved the high thumb");
    assert_eq!(low.get(), 20.0, "the press left the low thumb");

    // The press sits off the thumb's own value, so a write could not hide.
    harness.click(Point {
        x: track_x(0.9) - 5.0,
        y,
    });
    harness.frame();
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("maximum"),
        "a press on a thumb focuses it"
    );
    assert_eq!(high.get(), 90.0, "a press on a thumb writes nothing");

    harness.button_down(Point { x: track_x(0.9), y });
    harness.move_to(Point { x: track_x(0.1), y });
    harness.button_up(Point { x: track_x(0.1), y });
    harness.frame();
    assert_eq!(
        high.get(),
        20.0,
        "the drag stopped the high thumb at the low one"
    );

    // A drag from the track moves the thumb the press grabbed, here the
    // high one, since both thumbs sit at the same value. The press clears
    // the low thumb's right edge by 10 pixels, so it is a track press.
    harness.button_down(Point { x: track_x(0.3), y });
    harness.move_to(Point { x: track_x(0.5), y });
    harness.button_up(Point { x: track_x(0.5), y });
    harness.frame();
    assert_eq!(high.get(), 50.0, "the drag moved the grabbed high thumb");
    assert_eq!(low.get(), 20.0, "the drag left the low thumb");

    // A drag from the track nearer the low thumb grabs the low one and
    // keeps it: re-picking the nearest thumb at every move would hand the
    // drag to the high one once the pointer passes the middle.
    harness.button_down(Point { x: track_x(0.3), y });
    harness.move_to(Point { x: track_x(0.9), y });
    harness.button_up(Point { x: track_x(0.9), y });
    harness.frame();
    assert_eq!(
        low.get(),
        50.0,
        "the drag carried the low thumb to the high one"
    );
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("maximum"),
        "a press on the track leaves the focus"
    );
}

#[test]
fn a_drag_keeps_the_offset_it_grabbed_the_thumb_at() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(1.0)
    });
    let y = middle(&named(&mut harness, "minimum")).y;
    let center = track_x(0.30);

    harness.button_down(Point { x: center - 6.0, y });
    harness.move_to(Point {
        x: center - 6.0,
        y: y + 1.0,
    });
    harness.button_up(Point {
        x: center - 6.0,
        y: y + 1.0,
    });
    harness.frame();
    assert_eq!(
        low.get(),
        30.0,
        "a drag that never moves sideways writes nothing"
    );

    // From the same grab, a drag right keeps the offset: the thumb sits
    // where the pointer is, not six pixels behind it. Six left of center
    // plus 12.4 right of it is 18.4 of travel, a tenth of the 184-pixel
    // track, so the value climbs a clean tenth.
    harness.button_down(Point { x: center - 6.0, y });
    harness.move_to(Point {
        x: center + 12.4,
        y,
    });
    harness.button_up(Point {
        x: center + 12.4,
        y,
    });
    harness.frame();
    assert_eq!(
        low.get(),
        40.0,
        "the thumb follows the pointer from where it was grabbed"
    );
}

#[test]
fn the_keys_move_the_focused_thumb_and_tab_walks_both() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high, mark) = (low.clone(), high.clone(), RwSignal::new(false));
    let mut harness = window(move || {
        column()
            .child(
                stack()
                    .size(Size {
                        width: 204.0,
                        height: 20.0,
                    })
                    .child(
                        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0),
                    ),
            )
            .child(checkbox("Other", mark.clone()))
    });

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("minimum"),
        "Tab enters at the low thumb"
    );
    for (key, expected) in [
        (Key::ArrowRight, 35.0),
        (Key::ArrowLeft, 30.0),
        (Key::ArrowUp, 35.0),
        (Key::ArrowDown, 30.0),
        (Key::PageDown, 0.0),
        (Key::PageUp, 50.0),
        (Key::Home, 0.0),
    ] {
        harness.press_key(key.clone());
        assert_eq!(low.get(), expected, "the low thumb after {key:?}");
    }
    assert_eq!(high.get(), 70.0, "the keys left the high thumb");

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("maximum"),
        "the next Tab reaches the high thumb"
    );
    for (key, expected) in [
        (Key::ArrowLeft, 65.0),
        (Key::ArrowRight, 70.0),
        (Key::ArrowUp, 75.0),
        (Key::ArrowDown, 70.0),
        (Key::PageDown, 20.0),
        (Key::Home, 0.0),
        (Key::End, 100.0),
    ] {
        harness.press_key(key.clone());
        assert_eq!(high.get(), expected, "the high thumb after {key:?}");
    }
    assert_eq!(low.get(), 0.0, "the keys left the low thumb");

    harness.press_key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("minimum"),
        "Shift+Tab walks back to the low thumb"
    );
    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("maximum"),
        "and Tab comes forward again"
    );

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Other"),
        "the next Tab leaves the control"
    );
}

#[test]
fn no_thumb_crosses_the_other() {
    let (low, high) = (RwSignal::new(40.0), RwSignal::new(60.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });

    harness.press_key(Key::Tab);
    for _ in 0..4 {
        harness.press_key(Key::ArrowRight);
    }
    assert_eq!(low.get(), 60.0, "the low thumb stopped at the high one");
    for key in [Key::ArrowRight, Key::PageUp, Key::End] {
        harness.press_key(key.clone());
        assert_eq!(
            low.get(),
            60.0,
            "the low thumb stayed at the high one after {key:?}"
        );
    }

    harness.press_key(Key::Tab);
    for key in [Key::ArrowLeft, Key::Home, Key::PageDown] {
        harness.press_key(key.clone());
        assert_eq!(
            high.get(),
            60.0,
            "the high thumb stayed at the low one after {key:?}"
        );
    }
    assert_eq!(low.get(), 60.0, "the low thumb stopped at the high one");

    let minimum = named(&mut harness, "minimum").id;
    assert!(
        harness.app().step_value(minimum, true),
        "a stopped thumb still handles the increment"
    );
    assert_eq!(low.get(), 60.0, "the handled increment moved nothing");
}

#[test]
fn a_thumb_stops_at_an_off_step_other_thumb() {
    // The low thumb sits off the steps at 27: the high thumb dragged
    // against it stops at 27, not at the step beside it.
    let (low, high) = (RwSignal::new(27.0), RwSignal::new(80.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    let y = middle(&named(&mut harness, "maximum")).y;

    harness.button_down(Point { x: track_x(0.8), y });
    harness.move_to(Point {
        x: track_x(0.25),
        y,
    });
    harness.button_up(Point {
        x: track_x(0.25),
        y,
    });
    harness.frame();
    assert_eq!(
        high.get(),
        27.0,
        "the drag stopped the high thumb at the off-step low one"
    );
    assert_eq!(low.get(), 27.0, "the drag left the low thumb");
}

#[test]
fn a_press_on_the_low_thumb_takes_the_focus_and_writes_nothing() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    let center = middle(&named(&mut harness, "minimum"));

    // Five pixels left of the low thumb's center, still on its circle. The
    // high thumb's lane lies over the low one, so a press here lands on the
    // low thumb itself and not on the surface around it.
    harness.click(Point {
        x: center.x - 5.0,
        y: center.y,
    });
    harness.frame();
    assert_eq!(low.get(), 30.0, "the press wrote nothing");
    assert_eq!(high.get(), 70.0, "the press left the high thumb");
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("minimum"),
        "the press focused the low thumb"
    );
}

#[test]
fn home_and_end_reach_the_ends_whatever_the_step() {
    let (low, high) = (RwSignal::new(40.0), RwSignal::new(60.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(30.0)
    });

    harness.press_key(Key::Tab);
    harness.press_key(Key::Home);
    assert_eq!(low.get(), 0.0, "Home reached the range's start");
    harness.press_key(Key::End);
    assert_eq!(low.get(), 60.0, "End stopped the low thumb at the high one");

    harness.press_key(Key::Tab);
    harness.press_key(Key::End);
    assert_eq!(
        high.get(),
        100.0,
        "End reached the range's end, not its last step"
    );
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        high.get(),
        100.0,
        "an outward arrow keeps the off-step endpoint"
    );
    harness.press_key(Key::ArrowLeft);
    assert_eq!(
        high.get(),
        90.0,
        "an inward arrow lands on the nearest step inside"
    );

    // An outward page keeps the endpoint; an inward page runs ten steps
    // down, past the start, and stops at the low thumb.
    harness.press_key(Key::End);
    assert_eq!(high.get(), 100.0, "End reached the end again");
    harness.press_key(Key::PageUp);
    assert_eq!(
        high.get(),
        100.0,
        "an outward page keeps the off-step endpoint"
    );
    harness.press_key(Key::PageDown);
    assert_eq!(
        high.get(),
        60.0,
        "an inward page ran to the start and stopped at the low thumb"
    );
}

#[test]
fn a_decrement_from_an_on_grid_endpoint_moves_a_step() {
    let (low, high) = (RwSignal::new(0.0), RwSignal::new(100.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowLeft);
    assert_eq!(high.get(), 95.0, "a decrement from the end moves a step");
    harness.press_key(Key::ArrowDown);
    assert_eq!(
        high.get(),
        90.0,
        "the screen reader's decrement moves a step too"
    );
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        high.get(),
        95.0,
        "an increment from a stepped value moves a step"
    );
}

#[test]
fn a_page_never_moves_a_thumb_against_its_direction() {
    // Both thumbs sit off the steps at 95: a page up can only reach the
    // last step, 90, which lies behind them, so both keep their value.
    let (low, high) = (RwSignal::new(95.0), RwSignal::new(95.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(30.0)
    });

    harness.press_key(Key::Tab);
    harness.press_key(Key::PageUp);
    assert_eq!(low.get(), 95.0, "a page up never stepped the thumb down");
    assert_eq!(high.get(), 95.0, "a page up left the high thumb");

    harness.press_key(Key::Tab);
    harness.press_key(Key::PageUp);
    assert_eq!(high.get(), 95.0, "a page up never stepped the thumb down");

    // On the steps, a page runs its ten steps and comes back.
    let (low, high) = (RwSignal::new(60.0), RwSignal::new(90.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::PageDown);
    assert_eq!(low.get(), 10.0, "a page down ran ten steps down");
    assert_eq!(high.get(), 90.0, "a page down left the high thumb");
    harness.press_key(Key::PageUp);
    assert_eq!(low.get(), 60.0, "a page up ran the ten steps back");
}

#[test]
fn a_stopped_thumb_keeps_its_arrows_from_its_container() {
    let (low, high) = (RwSignal::new(50.0), RwSignal::new(50.0));
    let (bound_low, bound_high, mark) = (low.clone(), high.clone(), RwSignal::new(false));
    let mut harness = window(move || {
        column()
            .navigate_with_all_arrows()
            .child(checkbox("Other", mark.clone()))
            .child(range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0))
    });

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Other"),
        "Tab enters at the checkbox before the slider"
    );
    harness.press_key(Key::ArrowDown);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("minimum"),
        "the container walks the arrows its nodes let through"
    );

    harness.press_key(Key::ArrowRight);
    assert_eq!(low.get(), 50.0, "the stopped thumb stayed");
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("minimum"),
        "the arrow the thumb could not use stays with the thumb"
    );
}

#[test]
fn a_disabled_slider_moves_no_thumb_and_takes_no_focus() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high, mark) = (low.clone(), high.clone(), RwSignal::new(false));
    let mut harness = window(move || {
        column()
            .child(
                stack()
                    .size(Size {
                        width: 204.0,
                        height: 20.0,
                    })
                    .child(
                        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0)
                            .step(5.0)
                            .disabled(true),
                    ),
            )
            .child(checkbox("Other", mark.clone()))
    });
    let y = middle(&named(&mut harness, "minimum")).y;
    let maximum = middle(&named(&mut harness, "maximum"));

    harness.click(Point {
        x: track_x(0.21),
        y,
    });
    harness.frame();
    harness.click(maximum);
    harness.frame();
    assert_eq!(low.get(), 30.0, "the track press wrote nothing");
    assert_eq!(high.get(), 70.0, "the thumb press wrote nothing");
    assert_eq!(
        focused(&mut harness),
        None,
        "the disabled thumb took no focus"
    );

    harness.button_down(Point { x: track_x(0.5), y });
    harness.move_to(Point { x: track_x(0.8), y });
    harness.button_up(Point { x: track_x(0.8), y });
    harness.frame();
    assert_eq!(low.get(), 30.0, "the drag wrote nothing");
    assert_eq!(high.get(), 70.0, "the drag moved no thumb");

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Other"),
        "the disabled thumbs are no tab stops"
    );

    let minimum = named(&mut harness, "minimum").id;
    assert!(!harness.app().step_value(minimum, true));
    assert_eq!(low.get(), 30.0, "the increment moved nothing");
}

#[test]
fn every_write_lands_on_a_step_inside_the_range() {
    let (low, high) = (RwSignal::new(27.0), RwSignal::new(80.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    let y = middle(&named(&mut harness, "minimum")).y;

    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        low.get(),
        30.0,
        "a step from an off-step value starts on the steps"
    );

    harness.click(Point {
        x: track_x(0.13),
        y,
    });
    harness.frame();
    assert_eq!(low.get(), 15.0, "a press between steps lands on one");

    harness.press_key(Key::Tab);
    harness.button_down(Point { x: track_x(0.9), y });
    harness.move_to(Point { x: 250.0, y });
    harness.button_up(Point { x: 250.0, y });
    harness.frame();
    assert_eq!(high.get(), 100.0, "a drag past the end stops at the end");

    harness.button_down(Point { x: track_x(0.9), y });
    harness.move_to(Point { x: 2.0, y });
    harness.button_up(Point { x: 2.0, y });
    harness.frame();
    assert_eq!(
        high.get(),
        15.0,
        "a drag past the start stops at the low thumb"
    );

    // Without a step, one is a hundredth of the range.
    let (low, high) = (RwSignal::new(10.0), RwSignal::new(90.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness =
        slider_window(move || range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0));
    let y = middle(&named(&mut harness, "minimum")).y;
    harness.click(Point {
        x: track_x(0.333),
        y,
    });
    harness.frame();
    assert_eq!(low.get(), 33.0, "a press lands on a hundredth of the range");
}

#[test]
fn a_decimal_step_grids_cleanly_and_reaches_its_ends() {
    let (low, high) = (RwSignal::new(0.0), RwSignal::new(0.8));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=1.0).step(0.1)
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(high.get(), 0.9, "a tenth lands on a clean tenth");
    harness.press_key(Key::ArrowRight);
    assert_eq!(high.get(), 1.0, "the arrows reach the upper endpoint");
    harness.press_key(Key::Home);
    assert_eq!(high.get(), 0.0, "Home reaches the lower endpoint");

    // The default step is a hundredth of the range, and its end is
    // reachable too.
    let (low, high) = (RwSignal::new(0.0), RwSignal::new(0.98));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness =
        slider_window(move || range_slider(bound_low.clone(), bound_high.clone(), 0.0..=1.0));
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(high.get(), 0.99, "a hundredth lands on a clean hundredth");
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        high.get(),
        1.0,
        "the default step reaches the upper endpoint"
    );

    // A step whose end counts as a hair under a whole step still reaches
    // that end: 1.015 divides into 29 steps of 0.035, and the hair is
    // rounding, not a whole step short.
    let (low, high) = (RwSignal::new(0.0), RwSignal::new(0.985));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=1.015).step(0.035)
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(high.get(), 1.015, "the grid counts its end as a step");
}

#[test]
fn a_default_step_reaches_a_fractional_end() {
    let (low, high) = (RwSignal::new(0.9), RwSignal::new(0.999));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness =
        slider_window(move || range_slider(bound_low.clone(), bound_high.clone(), 0.9..=1.0));
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        high.get(),
        1.0,
        "the default step reaches a fractional upper endpoint"
    );
}

#[test]
fn a_step_of_no_use_falls_back_to_the_default() {
    let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(0.0)
    });

    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        high.get(),
        81.0,
        "a step of zero leaves the default hundredth"
    );
    harness.press_key(Key::End);
    assert_eq!(high.get(), 100.0, "the default step still reaches the end");
    assert_eq!(low.get(), 20.0, "the keys left the low thumb");

    let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(-5.0)
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        low.get(),
        25.0,
        "a negative step counts as its size, not as none"
    );

    for step in [f64::NAN, f64::INFINITY] {
        let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
        let (bound_low, bound_high) = (low.clone(), high.clone());
        let mut harness = slider_window(move || {
            range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(step)
        });
        harness.press_key(Key::Tab);
        harness.press_key(Key::ArrowRight);
        assert_eq!(
            low.get(),
            21.0,
            "a step of {step} leaves the default hundredth"
        );
    }
}

#[test]
fn a_range_of_one_value_moves_no_thumb() {
    let (low, high) = (RwSignal::new(50.0), RwSignal::new(50.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness =
        slider_window(move || range_slider(bound_low.clone(), bound_high.clone(), 50.0..=50.0));
    let y = middle(&named(&mut harness, "minimum")).y;

    harness.click(Point { x: track_x(0.5), y });
    harness.frame();
    harness.press_key(Key::Tab);
    for key in [Key::ArrowRight, Key::PageUp, Key::Home, Key::End] {
        harness.press_key(key.clone());
        assert_eq!(low.get(), 50.0, "the low thumb after {key:?}");
        assert_eq!(high.get(), 50.0, "the high thumb after {key:?}");
    }
    let minimum = named(&mut harness, "minimum").id;
    assert!(
        harness.app().step_value(minimum, true),
        "the stopped thumb still handles the increment"
    );
    assert_eq!(low.get(), 50.0, "the increment moved nothing");
}

#[test]
fn a_step_wider_than_the_range_collapses_to_the_start() {
    let (low, high) = (RwSignal::new(0.0), RwSignal::new(70.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(200.0)
    });

    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.press_key(Key::End);
    assert_eq!(
        high.get(),
        100.0,
        "End still reaches the end whatever the step"
    );
    harness.press_key(Key::ArrowLeft);
    assert_eq!(
        high.get(),
        0.0,
        "the only step inside the range is its start"
    );
    harness.press_key(Key::ArrowRight);
    assert_eq!(
        high.get(),
        0.0,
        "an arrow from the start finds no step to land on"
    );
}

#[test]
fn an_outside_write_that_crosses_the_thumbs_swaps_back() {
    // The pair arrives crossed: the slider swaps it at mount, so the low
    // thumb holds the lesser value from its first frame.
    let (low, high) = (RwSignal::new(80.0), RwSignal::new(20.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    assert_eq!(low.get(), 20.0, "the crossed pair came back swapped");
    assert_eq!(high.get(), 80.0, "the swap kept both values");

    // A later write from outside that crosses the pair swaps back too.
    low.set(90.0);
    harness.frame();
    assert_eq!(low.get(), 80.0, "the write crossed, and the pair swapped");
    assert_eq!(high.get(), 90.0, "the swap kept both values again");
    let (minimum, maximum) = (
        named(&mut harness, "minimum"),
        named(&mut harness, "maximum"),
    );
    assert_eq!(
        minimum.range.unwrap().max,
        90.0,
        "the low thumb reports the high thumb as its most"
    );
    assert_eq!(
        maximum.range.unwrap().min,
        80.0,
        "the high thumb reports the low thumb as its least"
    );
}

#[test]
fn the_projection_reports_two_sliders_with_their_ranges() {
    let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 10.0..=90.0)
            .step(5.0)
            .accessible_labels("Lowest", "Highest")
    });
    let (lowest, highest) = (
        named(&mut harness, "Lowest"),
        named(&mut harness, "Highest"),
    );
    for (node, value, min, max, name) in [
        (&lowest, 20.0, 10.0, 80.0, "Lowest"),
        (&highest, 80.0, 20.0, 90.0, "Highest"),
    ] {
        assert_eq!(node.role, Role::Slider, "{name} is a slider");
        assert!(node.focusable, "{name} can take the focus");
        let range = node.range.expect("the thumb reports its range");
        assert_eq!(range.value, value, "{name}'s value");
        assert_eq!(range.min, min, "{name}'s least");
        assert_eq!(range.max, max, "{name}'s most");
    }
    let defaults = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter(|node| matches!(node.label.as_deref(), Some("minimum") | Some("maximum")))
        .count();
    assert_eq!(defaults, 0, "the app's names replaced the defaults");
    let groups = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter(|node| node.role == Role::Group)
        .count();
    assert_eq!(groups, 1, "the two thumbs sit in one group");
}

#[test]
fn a_named_group_announces_its_name_before_its_thumbs() {
    let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0)
            .step(5.0)
            .accessible_label("Filter")
    });
    assert_eq!(
        named(&mut harness, "Filter").role,
        Role::Group,
        "the name named the group the thumbs sit in"
    );
    assert_eq!(
        named(&mut harness, "minimum").range.unwrap().min,
        0.0,
        "the thumbs kept their own names and bounds"
    );
}

#[test]
fn a_screen_reader_steps_each_thumb() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    let (minimum, maximum) = (
        named(&mut harness, "minimum").id,
        named(&mut harness, "maximum").id,
    );

    assert!(harness.app().step_value(minimum, true));
    assert_eq!(low.get(), 35.0, "the increment moved the low thumb");
    harness.frame();
    assert_eq!(
        named(&mut harness, "maximum").range.unwrap().min,
        35.0,
        "the high thumb's least follows the low thumb"
    );
    assert!(harness.app().step_value(minimum, false));
    assert_eq!(low.get(), 30.0, "the decrement moved the low thumb");
    assert!(harness.app().step_value(maximum, true));
    assert_eq!(high.get(), 75.0, "the increment moved the high thumb");
    harness.frame();
    assert_eq!(
        named(&mut harness, "minimum").range.unwrap().max,
        75.0,
        "the low thumb's most follows the high thumb"
    );
    assert!(harness.app().step_value(maximum, false));
    assert_eq!(high.get(), 70.0, "the decrement moved the high thumb");
}

#[test]
fn a_thumb_move_repaints_only_the_control() {
    let (low, high) = (RwSignal::new(30.0), RwSignal::new(70.0));
    let (bound_low, bound_high) = (low.clone(), high.clone());
    let mut harness = slider_window(move || {
        range_slider(bound_low.clone(), bound_high.clone(), 0.0..=100.0).step(5.0)
    });
    harness.settle();
    harness.press_key(Key::Tab);
    harness.frame();

    harness.press_key(Key::ArrowRight);
    let report = harness.frame();
    // The damage is in physical pixels and the window runs at scale 1.0, so
    // the two are the same size here.
    let pixels = harness.app().stats().damage_pixels as f32;
    let budget = 204.0 * 20.0;
    assert!(
        pixels <= budget,
        "{pixels} damage pixels stayed within the control's {budget}"
    );
    // The control sits at the window's top left, 204 by 20.
    for rect in &report.damage {
        let (left, top) = (rect.x as f32, rect.y as f32);
        let (right, bottom) = (left + rect.width as f32, top + rect.height as f32);
        assert!(
            left >= -1.0 && right <= 205.0 && top >= -1.0 && bottom <= 21.0,
            "damage stayed on the control: {rect:?}"
        );
    }
    harness.assert_matches_full_repaint();
}

/// A window with a range slider whose low thumb holds the keyboard focus,
/// above an enabled untouched one and a disabled one.
fn states_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let (low, high, idle_low, idle_high, busy_low, busy_high) = (
        RwSignal::new(20.0),
        RwSignal::new(60.0),
        RwSignal::new(20.0),
        RwSignal::new(80.0),
        RwSignal::new(35.0),
        RwSignal::new(55.0),
    );
    let (low, high, idle_low, idle_high, busy_low, busy_high) = (
        low.clone(),
        high.clone(),
        idle_low.clone(),
        idle_high.clone(),
        busy_low.clone(),
        busy_high.clone(),
    );
    let mut harness = Harness::new(220.0, 100.0, 1.0, move || {
        let (low, high, idle_low, idle_high, busy_low, busy_high) = (
            low.clone(),
            high.clone(),
            idle_low.clone(),
            idle_high.clone(),
            busy_low.clone(),
            busy_high.clone(),
        );
        column()
            .padding(px(10.0))
            .gap(Size {
                width: 0.0,
                height: 10.0,
            })
            .align_items(Align::Start)
            .child(
                stack()
                    .size(Size {
                        width: 200.0,
                        height: 20.0,
                    })
                    .child(range_slider(low, high, 10.0..=110.0).step(5.0)),
            )
            .child(
                stack()
                    .size(Size {
                        width: 200.0,
                        height: 20.0,
                    })
                    .child(range_slider(idle_low, idle_high, 10.0..=110.0).step(5.0)),
            )
            .child(
                stack()
                    .size(Size {
                        width: 200.0,
                        height: 20.0,
                    })
                    .child(
                        range_slider(busy_low, busy_high, 10.0..=110.0)
                            .step(5.0)
                            .disabled(true),
                    ),
            )
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness.press_key(Key::Tab);
    harness.frame();
    harness
}

#[test]
fn the_states_of_a_range_slider_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::light()).png());
}

#[test]
fn the_states_of_a_range_slider_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::dark()).png());
}
