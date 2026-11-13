//! The operations a widget draws inside its own border: they never cover the
//! border or reach past it, and the damage they leave matches a full repaint.

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Corners, Point, Rect};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::style::px;
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_test::Harness;

/// A box that draws one fill inside its border, at `content` relative to its
/// top left corner, or nothing when `content` is `None`.
struct Spill {
    content: Option<Rect>,
}

impl Widget for Spill {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        Panel.paint(cx, out);
    }

    fn paint_inside(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        if let Some(content) = self.content {
            out.push(PaintOp::FillRect {
                rect: Rect::new(
                    cx.rect.x0 + content.x0,
                    cx.rect.y0 + content.y0,
                    cx.rect.x0 + content.x1,
                    cx.rect.y0 + content.y1,
                ),
                radius: Corners::all(0.0),
                color: Color::rgb(0, 0, 255),
            });
        }
    }
}

/// The key of a [`Spill`]'s content.
struct Content;

/// Builds a [`Spill`] whose content follows `content`.
fn spill(content: RwSignal<Option<Rect>>) -> Element<Spill> {
    Element::new(Spill { content: None }).bind(
        Content,
        move || content.get(),
        |spill, content| {
            spill.content = content;
            Dirty::Paint
        },
    )
}

#[test]
fn content_that_overflows_a_box_that_does_not_clip_stays_inside_its_border() {
    let border = Color::rgb(200, 30, 30);
    let content = RwSignal::new(Some(Rect::new(-10.0, -10.0, 50.0, 30.0)));
    let bound = content.clone();
    let mut harness = Harness::new(80.0, 60.0, 1.0, move || {
        panel().padding(px(10.0)).child(
            spill(bound.clone())
                .width(px(40.0))
                .height(px(20.0))
                .border(1.0)
                .border_color(border),
        )
    });
    harness.frame();

    let (left, top, right, bottom) = (10, 10, 50, 30);
    for y in 0..60 {
        for x in 0..80 {
            let inside = (left..right).contains(&x) && (top..bottom).contains(&y);
            let ring = inside && (x == left || x == right - 1 || y == top || y == bottom - 1);
            let pixel = harness.pixel(x, y).expect("the pixel lies on the surface");
            if !inside {
                assert_eq!(pixel, [255; 4], "the content drew outside at {x}, {y}");
            } else if ring {
                let [r, g, b, _] = pixel;
                assert_eq!(
                    (r, g, b),
                    (200, 30, 30),
                    "the content covered the border at {x}, {y}"
                );
            } else {
                let [r, g, b, _] = pixel;
                assert_eq!((r, g, b), (0, 0, 255), "the content is missing at {x}, {y}");
            }
        }
    }

    content.set(Some(Rect::new(5.0, 5.0, 15.0, 15.0)));
    harness.frame();
    harness.assert_matches_full_repaint();

    content.set(None);
    harness.frame();
    harness.assert_matches_full_repaint();
}

#[test]
fn content_touching_the_border_edge_at_a_fractional_scale_is_repaired() {
    let content = RwSignal::new(Some(Rect::new(0.0, 2.0, 8.0, 4.0)));
    let bound = content.clone();
    let mut harness = Harness::new(20.0, 10.0, 1.25, move || {
        panel().child(spill(bound.clone()).width(px(8.0)).height(px(2.0)))
    });
    harness.frame();
    let [r, g, b, _] = harness.pixel(1, 2).expect("the pixel lies on the surface");
    assert_ne!(
        (r, g, b),
        (255, 255, 255),
        "the clip edge passes through physical row 2, so the content draws there"
    );

    content.set(None);
    harness.frame();
    harness.assert_matches_full_repaint();
}

#[test]
fn a_border_that_takes_the_whole_box_at_a_fractional_scale_leaves_nothing_behind() {
    let content = RwSignal::new(Some(Rect::new(0.0, 0.0, 2.0, 20.0)));
    let bound = content.clone();
    let mut harness = Harness::new(20.0, 30.0, 1.25, move || {
        panel().child(
            spill(bound.clone())
                .width(px(2.0))
                .height(px(20.0))
                .border(1.0),
        )
    });
    harness.frame();

    content.set(None);
    harness.frame();
    harness.assert_matches_full_repaint();
}

#[test]
fn content_of_a_box_inside_a_scrolled_view_is_repaired() {
    let content = RwSignal::new(Some(Rect::new(2.0, 2.0, 30.0, 18.0)));
    let offset = RwSignal::new(Point::default());
    let bound = content.clone();
    let scrolled = offset.clone();
    let mut harness = Harness::new(80.0, 80.0, 1.0, move || {
        panel()
            .width(px(60.0))
            .height(px(60.0))
            .clip(true)
            .scroll_offset(move || scrolled.get())
            .child(
                spill(bound.clone())
                    .width(px(40.0))
                    .height(px(20.0))
                    .border(1.0),
            )
            .child(panel().width(px(40.0)).height(px(100.0)))
    });
    harness.frame();

    offset.set(Point { x: 0.0, y: 15.0 });
    harness.frame();
    harness.assert_matches_full_repaint();

    content.set(Some(Rect::new(5.0, 3.0, 12.0, 6.0)));
    harness.frame();
    harness.assert_matches_full_repaint();

    content.set(None);
    harness.frame();
    harness.assert_matches_full_repaint();
}
