//! Scroll views that scroll sideways or both ways.

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Point, Rect};
use smudge_core::input::InputEvent;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{pct, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;
use smudge_widgets::{
    ScrollAxis, ScrollExt, column, row, scroll, virtual_list, virtual_list_keyed,
};

/// Wheels the view under the pointer by `delta`.
fn wheel(harness: &mut Harness, x: f32, y: f32) {
    harness.event(InputEvent::Wheel {
        delta: Point { x, y },
    });
}

/// A strip of colored cells, `count` of them, each `width` wide and
/// `height` tall, so a scroll moves ink a repaint can check.
fn strip(count: usize, width: f32, height: f32) -> Element<Panel> {
    (0..count).fold(row(), |strip, index| {
        let shade = (index * 37 % 200) as u8;
        strip.child(
            panel()
                .width(px(width))
                .height(px(height))
                .shrink(0.0)
                .background(Color::rgb(shade, 255 - shade, 90)),
        )
    })
}

/// A window with a view of `content` scrolling along `axis`, 200 by 100,
/// settled with the pointer over it, and the offset signals it scrolls
/// with.
fn view(
    axis: ScrollAxis,
    content: impl Fn() -> Element<Panel> + 'static,
) -> (Harness, RwSignal<f32>, RwSignal<f32>) {
    let (down, across) = (RwSignal::new(0.0), RwSignal::new(0.0));
    let (vertical, horizontal) = (down.clone(), across.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            scroll(content())
                .axis(axis)
                .offset(vertical.clone())
                .horizontal_offset(horizontal.clone())
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 50.0 });
    (harness, down, across)
}

#[test]
fn a_sideways_wheel_scrolls_wide_content_and_clamps_at_both_ends() {
    let (mut harness, down, across) = view(ScrollAxis::Horizontal, || {
        panel()
            .width(px(1_000_000.0))
            .height(px(40.0))
            .background(Color::rgb(30, 90, 200))
    });

    wheel(&mut harness, 300.0, 0.0);
    harness.frame();
    assert_eq!(across.get(), 300.0);

    wheel(&mut harness, -1_000.0, 0.0);
    harness.frame();
    assert_eq!(across.get(), 0.0, "the start holds it");

    across.set(f32::MAX);
    harness.settle();
    assert_eq!(
        across.get(),
        999_800.0,
        "the end is the content's width past the view's"
    );
    wheel(&mut harness, 50.0, 0.0);
    harness.frame();
    assert_eq!(across.get(), 999_800.0);

    wheel(&mut harness, 0.0, 40.0);
    harness.frame();
    assert_eq!(down.get(), 0.0, "a sideways view never scrolls up and down");
}

#[test]
fn a_sideways_scroll_moves_the_content_without_repainting_a_node() {
    let (mut harness, _, across) = view(ScrollAxis::Horizontal, || strip(40, 30.0, 60.0));

    wheel(&mut harness, 45.0, 0.0);
    let report = harness.frame();

    assert_eq!(across.get(), 45.0);
    assert_eq!(
        report.nodes_painted, 0,
        "the content moves under its offset, as a vertical scroll does"
    );
    assert!(!report.damage.is_empty());
    harness.assert_matches_full_repaint();
}

#[test]
fn a_view_that_scrolls_both_ways_follows_a_diagonal_wheel() {
    let (mut harness, down, across) = view(ScrollAxis::Both, || {
        (0..20).fold(column(), |grid, _| grid.child(strip(20, 30.0, 30.0)))
    });

    for (x, y) in [(37.0, 53.0), (120.0, 9.0), (-15.0, 200.0), (500.0, 500.0)] {
        wheel(&mut harness, x, y);
        harness.frame();
        harness.assert_matches_full_repaint();
    }

    assert_eq!(
        (across.get(), down.get()),
        (400.0, 500.0),
        "each axis stops at its own end"
    );
}

#[test]
fn narrow_content_fills_a_sideways_view_and_a_percent_width_resolves_against_it() {
    let (mut harness, _, across) = view(ScrollAxis::Horizontal, || {
        column().child(
            panel()
                .width(pct(100.0))
                .height(px(20.0))
                .background(Color::rgb(200, 40, 40)),
        )
    });

    let mut widths = Vec::new();
    harness.app().visit_nodes(|_, type_name, rect: Rect| {
        if type_name.ends_with("Panel") && rect.height() == 20.0 {
            widths.push(rect.width());
        }
    });
    assert!(
        !widths.is_empty() && widths.iter().all(|width| *width == 200.0),
        "the full-width child spans the view: {widths:?}"
    );

    wheel(&mut harness, 80.0, 0.0);
    harness.frame();
    assert_eq!(across.get(), 0.0, "content that fits does not scroll");
}

#[test]
fn a_virtual_list_given_a_sideways_axis_keeps_scrolling_up_and_down() {
    let (down, across) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (vertical, horizontal) = (down.clone(), across.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            virtual_list(
                || 100,
                20.0,
                |index| {
                    panel()
                        .width(px(900.0))
                        .height(px(20.0))
                        .background(Color::rgb((index * 2) as u8, 90, 140))
                },
            )
            .axis(ScrollAxis::Both)
            .offset(vertical.clone())
            .horizontal_offset(horizontal.clone())
            .width(px(200.0))
            .height(px(100.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 50.0 });

    wheel(&mut harness, 0.0, 60.0);
    harness.frame();
    wheel(&mut harness, 80.0, 0.0);
    harness.frame();
    assert_eq!((down.get(), across.get()), (60.0, 0.0));
}

#[test]
fn a_horizontal_offset_moves_nothing_in_a_view_that_scrolls_up_and_down() {
    let across = RwSignal::new(0.0f32);
    let horizontal = across.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            scroll(strip(20, 30.0, 300.0))
                .horizontal_offset(horizontal.clone())
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();
    across.set(150.0);
    harness.settle();
    let mut lefts = Vec::new();
    harness.app().visit_nodes(|_, type_name, rect: Rect| {
        if type_name.ends_with("Panel") && rect.height() == 300.0 && rect.width() == 30.0 {
            lefts.push(rect.x0);
        }
    });
    assert_eq!(
        lefts.first(),
        Some(&0.0),
        "the first cell stays at the left"
    );
}

#[test]
fn a_padded_and_bordered_sideways_view_scrolls_its_content_box() {
    let across = RwSignal::new(0.0f32);
    let horizontal = across.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            scroll(strip(20, 30.0, 60.0))
                .axis(ScrollAxis::Horizontal)
                .horizontal_offset(horizontal.clone())
                .padding(px(10.0))
                .border(2.0)
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();
    across.set(f32::MAX);
    harness.settle();
    assert_eq!(
        across.get(),
        424.0,
        "600 of content in the 176 the content box shows"
    );
}

#[test]
fn a_keyed_list_given_a_sideways_axis_keeps_scrolling_up_and_down() {
    let (down, across) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (vertical, horizontal) = (down.clone(), across.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let items: std::rc::Rc<[usize]> = (0..100).collect();
        panel().child(
            virtual_list_keyed(
                move || std::rc::Rc::clone(&items),
                |item| *item,
                20.0,
                |item| {
                    panel()
                        .width(px(900.0))
                        .height(px(20.0))
                        .shrink(0.0)
                        .background(Color::rgb((*item * 2) as u8, 90, 140))
                },
            )
            .axis(ScrollAxis::Both)
            .offset(vertical.clone())
            .horizontal_offset(horizontal.clone())
            .width(px(200.0))
            .height(px(100.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 50.0 });

    wheel(&mut harness, 80.0, 0.0);
    harness.frame();
    wheel(&mut harness, 0.0, 60.0);
    harness.frame();
    assert_eq!((down.get(), across.get()), (60.0, 0.0));
}
