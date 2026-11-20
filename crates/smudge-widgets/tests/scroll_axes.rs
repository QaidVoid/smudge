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

/// A grid of 20 by 20 cells, 30 pixels square, each a color of its own
/// row and column, 600 by 600 in all.
fn grid() -> Element<Panel> {
    (0..20).fold(column(), |grid, row_index| {
        grid.child((0..20).fold(row(), |cells, column_index| {
            cells.child(
                panel()
                    .width(px(30.0))
                    .height(px(30.0))
                    .shrink(0.0)
                    .background(Color::rgb(
                        (column_index * 12) as u8,
                        (row_index * 12) as u8,
                        150,
                    )),
            )
        }))
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
    let (mut harness, down, across) = view(ScrollAxis::Both, grid);

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

/// The thumbs that show, as rectangles: the vertical one is four pixels
/// wide, the horizontal one four tall.
fn thumbs(harness: &mut Harness) -> (Option<Rect>, Option<Rect>) {
    let (mut vertical, mut horizontal) = (None, None);
    harness.app().visit_nodes(|_, type_name, rect: Rect| {
        if !type_name.ends_with("Panel") {
            return;
        }
        if rect.width() == 4.0 && rect.height() > 4.0 {
            vertical = Some(rect);
        } else if rect.height() == 4.0 && rect.width() > 4.0 {
            horizontal = Some(rect);
        }
    });
    (vertical, horizontal)
}

/// The middle of `rect`.
fn middle(rect: Rect) -> Point {
    Point {
        x: (rect.x0 + rect.x1) / 2.0,
        y: (rect.y0 + rect.y1) / 2.0,
    }
}

/// Drags from `from` by `by`, a frame a move, and lets go.
fn drag(harness: &mut Harness, from: Point, by: Point) {
    let to = Point {
        x: from.x + by.x,
        y: from.y + by.y,
    };
    harness.button_down(from);
    harness.frame();
    harness.move_to(to);
    harness.frame();
    harness.button_up(to);
    harness.settle();
}

/// A sideways view of a strip 600 wide, whose thumb is 67 long and travels
/// 133 along the 200 the view is wide, for 400 of offset.
fn wide_strip() -> (Harness, RwSignal<f32>) {
    let (harness, _, across) = view(ScrollAxis::Horizontal, || strip(20, 30.0, 60.0));
    (harness, across)
}

#[test]
fn dragging_the_horizontal_thumb_scrolls_by_the_content_share_of_the_drag() {
    let (mut harness, across) = wide_strip();
    let (vertical, horizontal) = thumbs(&mut harness);
    assert!(
        vertical.is_none(),
        "a sideways view shows no vertical thumb"
    );
    let thumb = horizontal.expect("the horizontal thumb shows");
    assert_eq!(
        (thumb.width(), thumb.y1),
        (67.0, 98.0),
        "along the bottom edge"
    );

    drag(&mut harness, middle(thumb), Point { x: 20.0, y: 0.0 });

    let expected = 20.0 / 133.0 * 400.0;
    assert!((across.get() - expected).abs() < 0.01, "{}", across.get());
}

#[test]
fn the_horizontal_thumb_pages_and_holds_at_both_ends() {
    let (mut harness, across) = wide_strip();
    harness.click(Point { x: 150.0, y: 96.0 });
    harness.settle();
    assert_eq!(
        across.get(),
        200.0,
        "a press after the thumb pages by the view's width"
    );
    harness.click(Point { x: 5.0, y: 96.0 });
    harness.settle();
    assert_eq!(across.get(), 0.0, "and one before it pages back");

    let thumb = thumbs(&mut harness).1.expect("the thumb shows");
    drag(&mut harness, middle(thumb), Point { x: 900.0, y: 0.0 });
    assert_eq!(
        across.get(),
        400.0,
        "a drag past the track stops at the end"
    );
}

#[test]
fn each_thumb_of_a_view_that_scrolls_both_ways_moves_its_own_axis() {
    let (mut harness, down, across) = view(ScrollAxis::Both, grid);
    let (vertical, horizontal) = thumbs(&mut harness);
    let (vertical, horizontal) = (
        vertical.expect("the vertical thumb shows"),
        horizontal.expect("the horizontal thumb shows"),
    );
    assert!(
        vertical.y1 <= 92.0 && horizontal.x1 <= 192.0,
        "neither track reaches the corner"
    );
    harness.click(Point { x: 196.0, y: 96.0 });
    harness.settle();
    assert_eq!(
        (across.get(), down.get()),
        (0.0, 0.0),
        "a press in the corner pages neither axis"
    );

    drag(&mut harness, middle(horizontal), Point { x: 30.0, y: 0.0 });
    assert!(across.get() > 0.0);
    assert_eq!(
        down.get(),
        0.0,
        "the horizontal thumb leaves the other axis"
    );

    let before = across.get();
    drag(&mut harness, middle(vertical), Point { x: 0.0, y: 20.0 });
    assert!(down.get() > 0.0);
    assert_eq!(across.get(), before, "and so does the vertical one");
}

#[test]
fn a_view_that_scrolls_both_ways_draws_both_thumbs() {
    let (mut harness, down, across) = view(ScrollAxis::Both, grid);
    down.set(120.0);
    across.set(150.0);
    harness.settle();
    insta::assert_binary_snapshot!(".png", harness.png());
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
fn a_view_that_scrolls_both_ways_keeps_the_corner_only_while_both_thumbs_show() {
    let (mut harness, _, across) = view(ScrollAxis::Both, || strip(20, 30.0, 60.0));
    across.set(f32::MAX);
    harness.settle();
    let (vertical, horizontal) = thumbs(&mut harness);
    assert!(vertical.is_none(), "the content fits up and down");
    assert_eq!(
        horizontal.expect("the horizontal thumb shows").x1,
        200.0,
        "with one scrollbar, its thumb runs to the corner"
    );
}

#[test]
fn following_the_end_stays_on_the_vertical_axis_of_a_view_that_scrolls_both_ways() {
    let follow = RwSignal::new(true);
    let (down, across) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (pinned, vertical, horizontal) = (follow.clone(), down.clone(), across.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            scroll(grid())
                .axis(ScrollAxis::Both)
                .offset(vertical.clone())
                .horizontal_offset(horizontal.clone())
                .follow_end(pinned.clone())
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();
    assert_eq!((down.get(), across.get()), (500.0, 0.0));

    harness.move_to(Point { x: 100.0, y: 50.0 });
    wheel(&mut harness, 120.0, 0.0);
    harness.settle();
    assert_eq!((down.get(), across.get()), (500.0, 120.0));
    assert!(follow.get(), "a sideways scroll keeps following the end");
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

#[test]
fn the_vertical_track_makes_room_for_the_corner_once_the_horizontal_one_shows() {
    let width = RwSignal::new(150.0f32);
    let wide = width.clone();
    let (mut harness, down, _) = view(ScrollAxis::Both, move || {
        let wide = wide.clone();
        column().child(
            panel()
                .width(move || px(wide.get()))
                .height(px(600.0))
                .background(Color::rgb(40, 120, 90)),
        )
    });
    down.set(f32::MAX);
    harness.settle();
    let at_end = |harness: &mut Harness| thumbs(harness).0.expect("the vertical thumb").y1;
    assert_eq!(
        at_end(&mut harness),
        100.0,
        "alone, the thumb runs to the bottom"
    );

    width.set(600.0);
    harness.settle();
    down.set(f32::MAX);
    harness.settle();
    assert_eq!(
        at_end(&mut harness),
        92.0,
        "with the other bar, it stops at the corner"
    );
}
