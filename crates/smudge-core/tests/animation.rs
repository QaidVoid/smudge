//! Whole-frame behavior of springs on paint props, driven through the
//! headless harness on its virtual clock.

use std::time::Duration;

use smudge_core::animation::Spring;
use smudge_core::geometry::{Color, Point};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;

/// The color the panel starts with.
const RESTING: Color = Color::rgb(200, 40, 40);

/// The color the panel takes while hovered.
const HOVERED: Color = Color::rgb(40, 40, 200);

/// The interval between frames, about one at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

/// A point inside the panel.
const INSIDE: Point = Point { x: 50.0, y: 50.0 };

/// A point outside the panel.
const OUTSIDE: Point = Point { x: 150.0, y: 150.0 };

/// A harness showing one panel, in the corner of a window-filling root,
/// that moves from [`RESTING`] to [`HOVERED`] under `spring` while hovered.
fn hover_panel(spring: Spring) -> Harness {
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().child(
            panel()
                .width(px(100.0))
                .height(px(100.0))
                .background(RESTING)
                .hover(|style| style.background(HOVERED))
                .transition(spring),
        )
    });
    harness.frame();
    harness
}

/// The panel's color, read from the middle of it.
fn shown(harness: &Harness) -> [u8; 4] {
    harness
        .pixel(INSIDE.x as u32, INSIDE.y as u32)
        .expect("the point is inside the surface")
}

/// The color `color` as the pixels show it.
fn rgba(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

#[test]
fn a_hover_moves_the_background_through_colors_between_the_two() {
    let mut harness = hover_panel(Spring::SMOOTH);
    assert_eq!(shown(&harness), rgba(RESTING));
    harness.move_to(INSIDE);
    harness.frame();
    let mut between = 0;
    for _ in 0..60 {
        harness.advance(FRAME);
        let color = shown(&harness);
        if color != rgba(RESTING) && color != rgba(HOVERED) {
            between += 1;
        }
    }
    assert!(between >= 5, "only {between} frames showed a color between");
    assert_eq!(shown(&harness), rgba(HOVERED));
}

#[test]
fn a_settled_spring_leaves_nothing_to_draw() {
    let mut harness = hover_panel(Spring::SNAPPY);
    harness.move_to(INSIDE);
    harness.frame();
    assert!(
        harness.app().is_animating(),
        "the hover set the spring moving"
    );
    for _ in 0..120 {
        harness.advance(FRAME);
    }
    assert!(!harness.app().is_animating());
    assert_eq!(harness.app().next_deadline(), None);
    assert!(!harness.app().needs_frame());
    let report = harness.advance(FRAME);
    assert!(report.damage.is_empty(), "{:?}", report.damage);
    assert_eq!(report.nodes_painted, 0);
}

#[test]
fn a_hover_that_ends_halfway_turns_back_without_a_jump() {
    let mut harness = hover_panel(Spring::SMOOTH);
    harness.move_to(INSIDE);
    harness.frame();
    for _ in 0..8 {
        harness.advance(FRAME);
    }
    let before = shown(&harness);
    assert!(before[2] > RESTING.b && before[2] < HOVERED.b, "{before:?}");

    harness.move_to(OUTSIDE);
    harness.frame();
    let mut previous = before;
    let mut turned = false;
    for _ in 0..90 {
        harness.advance(FRAME);
        let color = shown(&harness);
        let step = (i16::from(color[2]) - i16::from(previous[2])).abs();
        assert!(
            step <= 24,
            "the blue channel jumped from {previous:?} to {color:?}"
        );
        if color[2] < before[2] {
            turned = true;
        }
        previous = color;
    }
    assert!(turned, "the color turned back toward the resting one");
    assert_eq!(shown(&harness), rgba(RESTING));
}

#[test]
fn reduced_motion_shows_the_hovered_color_in_the_first_frame() {
    let mut harness = hover_panel(Spring::SMOOTH);
    harness.app().set_reduced_motion(true);
    harness.move_to(INSIDE);
    harness.frame();
    assert_eq!(shown(&harness), rgba(HOVERED));
    assert!(!harness.app().is_animating());
}

#[test]
fn moving_frames_repaint_without_layout_and_match_a_full_repaint() {
    let mut harness = hover_panel(Spring::BOUNCY);
    harness.move_to(INSIDE);
    harness.frame();
    for _ in 0..30 {
        let report = harness.advance(FRAME);
        assert!(!report.layout_ran, "a moving color ran the layout engine");
        assert!(!report.full);
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn a_fill_that_appears_on_hover_fades_in() {
    let mut harness = Harness::new(200.0, 200.0, 1.0, || {
        panel().child(
            panel()
                .width(px(100.0))
                .height(px(100.0))
                .hover(|style| style.background(HOVERED))
                .transition(Spring::SMOOTH),
        )
    });
    harness.frame();
    let empty = shown(&harness);
    harness.move_to(INSIDE);
    harness.frame();
    let mut between = 0;
    for _ in 0..60 {
        harness.advance(FRAME);
        let color = shown(&harness);
        if color != empty && color != rgba(HOVERED) {
            between += 1;
        }
    }
    assert!(
        between >= 5,
        "only {between} frames showed the fill fading in"
    );
    assert_eq!(shown(&harness), rgba(HOVERED));
}
