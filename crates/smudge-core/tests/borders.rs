//! Whole-frame behavior of borders with a width of their own on each side,
//! drawn through the headless harness.

use smudge_core::geometry::Color;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Align, Edges, px};
use smudge_core::widget::panel;
use smudge_test::Harness;

/// The color the bordered panels draw their border in.
const BORDER: Color = Color::rgb(30, 60, 140);

/// The surface pixel at `(x, y)`.
fn pixel(harness: &Harness, x: u32, y: u32) -> [u8; 4] {
    harness
        .pixel(x, y)
        .expect("the point is inside the surface")
}

/// Edges of `bottom` width at the bottom and nothing elsewhere.
fn bottom_only(bottom: f32) -> Edges<f32> {
    Edges {
        top: 0.0,
        right: 0.0,
        bottom,
        left: 0.0,
    }
}

#[test]
fn a_panel_draws_only_the_sides_its_border_edges_give() {
    let mut harness = Harness::new(100.0, 100.0, 1.0, || {
        panel().child(
            panel()
                .width(px(60.0))
                .height(px(30.0))
                .border_edges(bottom_only(2.0))
                .border_color(BORDER),
        )
    });
    harness.frame();
    for y in [28, 29] {
        assert_eq!(
            pixel(&harness, 30, y),
            [30, 60, 140, 255],
            "the bottom band at row {y}"
        );
    }
    assert_eq!(pixel(&harness, 30, 27), [255, 255, 255, 255], "above it");
    assert_eq!(
        pixel(&harness, 30, 0),
        [255, 255, 255, 255],
        "no top border"
    );
    assert_eq!(
        pixel(&harness, 0, 15),
        [255, 255, 255, 255],
        "no left border"
    );
    assert_eq!(
        pixel(&harness, 59, 15),
        [255, 255, 255, 255],
        "no right border"
    );
}

#[test]
fn a_border_width_change_lays_the_node_out_again() {
    let widths = RwSignal::new(bottom_only(2.0));
    let read = widths.clone();
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let read = read.clone();
        panel().align_items(Align::Start).child(
            panel()
                .column()
                .border_edges(move || read.get())
                .border_color(BORDER)
                .child(panel().width(px(40.0)).height(px(20.0))),
        )
    });
    harness.frame();
    let first = harness.layout_dump();
    assert!(first.contains("[0, 0, 40, 22]"), "{first}");

    widths.set(Edges {
        top: 6.0,
        right: 0.0,
        bottom: 2.0,
        left: 4.0,
    });
    harness.frame();
    let dump = harness.layout_dump();
    assert!(
        dump.contains("[0, 0, 44, 28]"),
        "the bordered panel grew around its content: {dump}"
    );
    assert!(
        dump.contains("[4, 6, 44, 26]"),
        "the content moved inside the new border: {dump}"
    );
    harness.assert_matches_full_repaint();
    assert_eq!(
        pixel(&harness, 2, 15),
        [30, 60, 140, 255],
        "the new left band"
    );
}
