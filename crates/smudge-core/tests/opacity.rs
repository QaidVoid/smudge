//! Whole-frame behavior of group opacity, driven through the headless
//! harness.

use std::time::Duration;

use smudge_core::animation::Spring;
use smudge_core::geometry::{Color, Point};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;

/// The color the faded square paints.
const SQUARE: Color = Color::rgb(200, 40, 40);

/// A point inside the square.
const INSIDE: Point = Point { x: 20.0, y: 20.0 };

/// The surface pixel at `point`.
fn pixel(harness: &Harness, point: Point) -> [u8; 4] {
    harness
        .pixel(point.x as u32, point.y as u32)
        .expect("the point is inside the surface")
}

/// A harness showing a parent with no paint of its own whose child square
/// draws at the parent's opacity, which `opacity` sets.
fn faded(opacity: RwSignal<f32>, spring: Option<Spring>) -> Harness {
    Harness::new(100.0, 100.0, 1.0, move || {
        let read = opacity.clone();
        let parent = panel()
            .width(px(60.0))
            .height(px(60.0))
            .opacity(move || read.get())
            .child(panel().width(px(40.0)).height(px(40.0)).background(SQUARE));
        panel().child(match spring {
            Some(spring) => parent.transition(spring),
            None => parent,
        })
    })
}

#[test]
fn a_change_of_opacity_redraws_children_the_parent_does_not_paint() {
    let opacity = RwSignal::new(1.0);
    let mut harness = faded(opacity.clone(), None);
    harness.frame();
    assert_eq!(pixel(&harness, INSIDE), [200, 40, 40, 255]);

    opacity.set(0.5);
    harness.frame();
    let shown = pixel(&harness, INSIDE);
    assert!(
        shown[0] > 200 && shown[1] > 120,
        "the square drew at half opacity over white: {shown:?}"
    );
    harness.assert_matches_full_repaint();

    opacity.set(0.0);
    harness.frame();
    assert_eq!(pixel(&harness, INSIDE), [255, 255, 255, 255]);
    harness.assert_matches_full_repaint();
}

#[test]
fn a_node_that_draws_nothing_still_takes_a_press() {
    let presses = RwSignal::new(0u32);
    let count = presses.clone();
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let count = count.clone();
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(40.0))
                .background(SQUARE)
                .opacity(0.0)
                .on_press(move || count.update(|count| *count += 1)),
        )
    });
    harness.frame();
    assert_eq!(pixel(&harness, INSIDE), [255, 255, 255, 255]);
    harness.click(INSIDE);
    harness.frame();
    assert_eq!(presses.get(), 1);
}

#[test]
fn a_spring_on_opacity_settles_and_leaves_nothing_to_draw() {
    let opacity = RwSignal::new(1.0);
    let mut harness = faded(opacity.clone(), Some(Spring::SNAPPY));
    harness.frame();
    opacity.set(0.2);
    harness.frame();
    assert!(harness.app().is_animating());
    let mut between = 0;
    for _ in 0..90 {
        harness.advance(Duration::from_millis(16));
        let shown = pixel(&harness, INSIDE);
        if shown != [200, 40, 40, 255] && shown[1] < 190 {
            between += 1;
        }
        harness.assert_matches_full_repaint();
    }
    assert!(
        between >= 3,
        "only {between} frames showed an opacity between"
    );
    assert!(!harness.app().is_animating());
    assert!(!harness.app().needs_frame());
    let report = harness.advance(Duration::from_millis(16));
    assert!(report.damage.is_empty(), "{:?}", report.damage);
}
