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

/// A page 200 by 200 that scrolls up and down, with an inner view of
/// `inner` 100 tall at its top scrolling along `axis`, and 400 pixels of
/// filler below it. The pointer rests over the inner view. Returns the
/// harness, the page's offset, and the inner view's vertical and horizontal
/// offsets.
fn page_around(
    axis: ScrollAxis,
    inner: impl Fn() -> Element<Panel> + 'static,
) -> (Harness, RwSignal<f32>, RwSignal<f32>, RwSignal<f32>) {
    let (page, down, across) = (RwSignal::new(0.0), RwSignal::new(0.0), RwSignal::new(0.0));
    let (outer, vertical, horizontal) = (page.clone(), down.clone(), across.clone());
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(
                column()
                    .child(
                        scroll(inner())
                            .axis(axis)
                            .offset(vertical.clone())
                            .horizontal_offset(horizontal.clone())
                            .height(px(100.0)),
                    )
                    .child(panel().height(px(400.0)).background(Color::rgb(90, 90, 90))),
            )
            .offset(outer.clone())
            .height(px(200.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 50.0 });
    (harness, page, down, across)
}

/// Sets whether Shift is held.
fn shift(harness: &mut Harness, held: bool) {
    harness.event(InputEvent::ModifiersChanged(
        smudge_core::input::Modifiers {
            shift: held,
            ..smudge_core::input::Modifiers::default()
        },
    ));
}

#[test]
fn shift_turns_the_wheel_sideways_over_a_horizontal_strip() {
    let (mut harness, page, _, across) =
        page_around(ScrollAxis::Horizontal, || strip(20, 30.0, 60.0));

    shift(&mut harness, true);
    wheel(&mut harness, 0.0, 40.0);
    harness.frame();
    assert_eq!(across.get(), 40.0, "the strip scrolls sideways");
    assert_eq!(page.get(), 0.0, "and the page stays");

    shift(&mut harness, false);
    wheel(&mut harness, 0.0, 40.0);
    harness.frame();
    assert_eq!(
        page.get(),
        40.0,
        "without Shift the strip passes it to the page"
    );
    assert_eq!(across.get(), 40.0);
}

#[test]
fn a_view_at_its_end_passes_the_wheel_on_and_one_that_moves_keeps_it() {
    let (mut harness, page, down, _) = page_around(ScrollAxis::Vertical, || {
        column().child(
            panel()
                .height(px(160.0))
                .background(Color::rgb(30, 120, 200)),
        )
    });

    wheel(&mut harness, 0.0, 50.0);
    harness.frame();
    assert_eq!(
        (down.get(), page.get()),
        (50.0, 0.0),
        "the inner view moves"
    );

    wheel(&mut harness, 0.0, 50.0);
    harness.frame();
    assert_eq!(
        (down.get(), page.get()),
        (60.0, 0.0),
        "a step that reaches the end is the inner view's whole"
    );

    wheel(&mut harness, 0.0, 30.0);
    harness.frame();
    assert_eq!(
        (down.get(), page.get()),
        (60.0, 30.0),
        "at its end it passes on"
    );

    wheel(&mut harness, 0.0, -20.0);
    harness.frame();
    assert_eq!(
        (down.get(), page.get()),
        (40.0, 30.0),
        "a step back the inner view can take is its own again"
    );
}

/// A row of `count` focusable cells, each `width` wide and 30 tall.
fn focusable_strip(count: usize, width: f32) -> Element<Panel> {
    (0..count).fold(row(), |strip, index| {
        strip.child(
            panel()
                .width(px(width))
                .height(px(30.0))
                .shrink(0.0)
                .focusable()
                .role(smudge_core::semantics::Role::Button)
                .accessible_label(format!("Cell {index}"))
                .background(Color::rgb((index * 20) as u8, 120, 160)),
        )
    })
}

/// The rectangle of the focused node, as the window shows it.
fn focused_rect(harness: &mut Harness) -> Rect {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
        .expect("a node holds the focus")
        .bounds
}

#[test]
fn tabbing_to_a_cell_past_the_right_edge_scrolls_it_into_view() {
    let (mut harness, _, across) = view(ScrollAxis::Horizontal, || focusable_strip(10, 60.0));

    for _ in 0..5 {
        harness.press_key(smudge_core::input::Key::Tab);
    }
    harness.settle();

    let cell = focused_rect(&mut harness);
    assert_eq!(across.get(), 100.0, "the fifth cell ends 100 past the view");
    assert!(cell.x0 >= 0.0 && cell.x1 <= 200.0, "{cell:?} is in view");
}

#[test]
fn tabbing_to_a_cell_past_both_edges_scrolls_both_axes() {
    let (mut harness, down, across) = view(ScrollAxis::Both, || {
        column()
            .child(panel().height(px(200.0)).width(px(10.0)))
            .child(focusable_strip(10, 60.0))
    });

    for _ in 0..6 {
        harness.press_key(smudge_core::input::Key::Tab);
    }
    harness.settle();

    let cell = focused_rect(&mut harness);
    assert_eq!(
        (across.get(), down.get()),
        (160.0, 130.0),
        "the sixth cell, at 300 to 360 across and 200 to 230 down"
    );
    assert!(cell.x1 <= 200.0 && cell.y1 <= 100.0, "{cell:?} is in view");
}

#[test]
fn a_drag_held_past_the_right_edge_scrolls_sideways_until_the_end() {
    let across = RwSignal::new(0.0f32);
    let offset = across.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        column().on_press(|| {}).child(
            scroll(strip(20, 30.0, 60.0))
                .axis(ScrollAxis::Horizontal)
                .horizontal_offset(offset.clone())
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();

    harness.button_down(Point { x: 100.0, y: 50.0 });
    harness.move_to(Point { x: 260.0, y: 50.0 });
    for _ in 0..10 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    let scrolled = across.get();
    assert!(scrolled > 0.0, "the drag past the right edge scrolled");

    for _ in 0..200 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    assert_eq!(across.get(), 400.0, "and stops at the end");
    harness.button_up(Point { x: 260.0, y: 50.0 });
    harness.assert_matches_full_repaint();
}

#[test]
fn a_drag_held_past_an_inner_view_at_its_end_scrolls_the_view_around_it() {
    let (inner, outer) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (inner_offset, outer_offset) = (inner.clone(), outer.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        column().on_press(|| {}).child(
            scroll(
                row()
                    .child(
                        scroll(strip(20, 30.0, 60.0))
                            .axis(ScrollAxis::Horizontal)
                            .horizontal_offset(inner_offset.clone())
                            .width(px(150.0))
                            .shrink(0.0),
                    )
                    .child(panel().width(px(600.0)).height(px(60.0)).shrink(0.0)),
            )
            .axis(ScrollAxis::Horizontal)
            .horizontal_offset(outer_offset.clone())
            .width(px(200.0))
            .height(px(100.0)),
        )
    });
    harness.settle();

    harness.button_down(Point { x: 50.0, y: 30.0 });
    harness.move_to(Point { x: 280.0, y: 30.0 });
    for _ in 0..400 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    assert_eq!(inner.get(), 450.0, "the inner view reached its end");
    assert_eq!(outer.get(), 0.0);

    // The next move finds the inner view at its end, so the view around it
    // takes the drag.
    harness.move_to(Point { x: 282.0, y: 30.0 });
    for _ in 0..10 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    harness.button_up(Point { x: 282.0, y: 30.0 });
    assert!(outer.get() > 0.0, "and the view around it took over");
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
fn a_touchpad_swipe_that_mostly_goes_down_scrolls_the_page_over_a_strip() {
    let (mut harness, page, _, across) =
        page_around(ScrollAxis::Horizontal, || strip(20, 30.0, 60.0));

    wheel(&mut harness, 0.4, 6.0);
    harness.frame();
    assert_eq!((page.get(), across.get()), (6.0, 0.0));

    wheel(&mut harness, 9.0, 1.5);
    harness.frame();
    assert_eq!(
        (page.get(), across.get()),
        (6.0, 9.0),
        "a swipe that mostly goes sideways is the strip's"
    );
}

#[test]
fn a_step_down_at_the_end_follows_again_and_leaves_the_event_to_the_page() {
    let follow = RwSignal::new(false);
    let pinned = follow.clone();
    let page = RwSignal::new(0.0f32);
    let outer = page.clone();
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(
                column()
                    .child(
                        scroll(column().child(panel().height(px(160.0))))
                            .offset(RwSignal::new(f32::MAX))
                            .follow_end(pinned.clone())
                            .height(px(100.0)),
                    )
                    .child(panel().height(px(400.0))),
            )
            .offset(outer.clone())
            .height(px(200.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 50.0 });

    wheel(&mut harness, 0.0, 30.0);
    harness.frame();
    assert!(follow.get(), "the step at the end engaged following");
    assert_eq!(page.get(), 30.0, "and the page took the step");
}

#[test]
fn a_drag_past_a_strip_and_below_the_page_scrolls_both() {
    let (page, across) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (outer, inner) = (page.clone(), across.clone());
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().column().on_press(|| {}).child(
            scroll(
                column()
                    .child(
                        scroll(strip(20, 30.0, 60.0))
                            .axis(ScrollAxis::Horizontal)
                            .horizontal_offset(inner.clone())
                            .height(px(60.0)),
                    )
                    .child(panel().height(px(600.0))),
            )
            .offset(outer.clone())
            .height(px(200.0)),
        )
    });
    harness.settle();

    harness.button_down(Point { x: 50.0, y: 30.0 });
    harness.move_to(Point { x: 260.0, y: 260.0 });
    for _ in 0..20 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    harness.button_up(Point { x: 260.0, y: 260.0 });
    assert!(across.get() > 0.0, "the strip scrolled across");
    assert!(page.get() > 0.0, "and the page scrolled down with it");
}

#[test]
fn a_reveal_leaves_a_view_that_does_not_scroll_sideways_at_its_left() {
    // An app's own alignment lets the content run wider than the view.
    let mut harness = Harness::new(300.0, 200.0, 1.0, || {
        panel().child(
            scroll(focusable_strip(10, 60.0))
                .align_items(smudge_core::style::Align::Start)
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();
    for _ in 0..6 {
        harness.press_key(smudge_core::input::Key::Tab);
    }
    harness.settle();
    let mut lefts = Vec::new();
    harness.app().visit_nodes(|_, type_name, rect: Rect| {
        if type_name.ends_with("Panel") && rect.width() == 60.0 && rect.height() == 30.0 {
            lefts.push(rect.x0);
        }
    });
    assert_eq!(
        lefts.first(),
        Some(&0.0),
        "the content stays at the view's left"
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
fn a_mostly_sideways_swipe_over_a_list_scrolls_a_sideways_page_around_it() {
    let (page, list) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (outer, inner) = (page.clone(), list.clone());
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().child(
            scroll(
                row()
                    .child(
                        scroll(column().child(panel().height(px(600.0)).width(px(150.0))))
                            .offset(inner.clone())
                            .width(px(150.0))
                            .height(px(150.0))
                            .shrink(0.0),
                    )
                    .child(panel().width(px(500.0)).height(px(150.0)).shrink(0.0)),
            )
            .axis(ScrollAxis::Horizontal)
            .horizontal_offset(outer.clone())
            .width(px(200.0))
            .height(px(150.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 50.0, y: 50.0 });

    wheel(&mut harness, 9.0, 1.5);
    harness.frame();
    assert_eq!((page.get(), list.get()), (9.0, 0.0));
}

#[test]
fn a_drag_out_of_a_list_past_its_side_scrolls_the_list_and_not_the_page_around_it() {
    let (page, list) = (RwSignal::new(0.0f32), RwSignal::new(0.0f32));
    let (outer, inner) = (page.clone(), list.clone());
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().column().on_press(|| {}).child(
            scroll(
                row()
                    .child(panel().width(px(100.0)).height(px(600.0)).shrink(0.0))
                    .child(
                        scroll(column().child(panel().height(px(900.0)).width(px(100.0))))
                            .offset(inner.clone())
                            .width(px(100.0))
                            .height(px(150.0))
                            .shrink(0.0),
                    ),
            )
            .offset(outer.clone())
            .height(px(150.0)),
        )
    });
    harness.settle();

    harness.button_down(Point { x: 150.0, y: 20.0 });
    harness.move_to(Point { x: 20.0, y: 190.0 });
    for _ in 0..20 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    harness.button_up(Point { x: 20.0, y: 190.0 });
    assert!(list.get() > 0.0, "the list scrolled down");
    assert_eq!(page.get(), 0.0, "and the page left that axis to it");
}

#[test]
fn a_view_that_scrolls_both_ways_at_its_end_leaves_a_downward_swipe_to_the_page() {
    let (mut harness, page, down, across) = page_around(ScrollAxis::Both, grid);
    down.set(f32::MAX);
    harness.settle();

    wheel(&mut harness, 0.4, 6.0);
    harness.frame();
    assert_eq!(page.get(), 6.0, "the page scrolled");
    assert_eq!(across.get(), 0.0, "and the grid did not drift sideways");
}

#[test]
fn a_mostly_sideways_swipe_over_a_log_at_its_end_does_not_follow_it() {
    let follow = RwSignal::new(false);
    let pinned = follow.clone();
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().child(
            scroll(
                row()
                    .child(
                        scroll(column().child(panel().height(px(600.0)).width(px(150.0))))
                            .offset(RwSignal::new(f32::MAX))
                            .follow_end(pinned.clone())
                            .width(px(150.0))
                            .height(px(150.0))
                            .shrink(0.0),
                    )
                    .child(panel().width(px(500.0)).height(px(150.0)).shrink(0.0)),
            )
            .axis(ScrollAxis::Horizontal)
            .width(px(200.0))
            .height(px(150.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 50.0, y: 50.0 });

    wheel(&mut harness, 9.0, 1.5);
    harness.frame();
    assert!(!follow.get(), "the swipe was the page's, not a step down");
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

#[test]
fn a_sideways_swipe_over_a_following_log_that_scrolls_both_ways_keeps_it_following() {
    let follow = RwSignal::new(true);
    let (pinned, vertical) = (follow.clone(), RwSignal::new(0.0f32));
    let down = vertical.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            scroll(grid())
                .axis(ScrollAxis::Both)
                .offset(vertical.clone())
                .follow_end(pinned.clone())
                .width(px(200.0))
                .height(px(100.0)),
        )
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 50.0 });

    wheel(&mut harness, 9.0, -0.5);
    harness.settle();
    assert!(follow.get(), "a mostly sideways swipe leaves it following");
    assert_eq!(down.get(), 500.0);

    follow.set(false);
    down.set(497.0);
    harness.settle();
    wheel(&mut harness, 9.0, 0.5);
    harness.settle();
    assert!(!follow.get(), "and does not pin it near the end either");
}

#[test]
fn a_drag_above_a_page_and_a_list_scrolls_the_list_first() {
    let (page, list) = (RwSignal::new(0.0f32), RwSignal::new(f32::MAX));
    let (outer, inner) = (page.clone(), list.clone());
    let mut harness = Harness::new(200.0, 300.0, 1.0, move || {
        panel().column().padding(px(50.0)).on_press(|| {}).child(
            scroll(
                column()
                    .child(panel().height(px(40.0)))
                    .child(
                        scroll(column().child(panel().height(px(400.0)).width(px(100.0))))
                            .offset(inner.clone())
                            .height(px(100.0)),
                    )
                    .child(panel().height(px(600.0))),
            )
            .offset(outer.clone())
            .height(px(200.0)),
        )
    });
    harness.settle();
    let list_end = list.get();

    harness.button_down(Point { x: 100.0, y: 120.0 });
    harness.move_to(Point { x: 100.0, y: 290.0 });
    for _ in 0..5 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    assert!(page.get() > 0.0, "below both, the page scrolls down");
    assert_eq!(list.get(), list_end, "past the list's end");

    let page_then = page.get();
    harness.move_to(Point { x: 100.0, y: 10.0 });
    for _ in 0..5 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    harness.button_up(Point { x: 100.0, y: 10.0 });
    assert!(
        list.get() < list_end,
        "above both, the list scrolls up first"
    );
    assert_eq!(page.get(), page_then, "and the page waits");
}

#[test]
fn a_drag_past_a_grid_at_its_right_end_scrolls_the_sideways_page_around_it() {
    let (page, across) = (RwSignal::new(0.0f32), RwSignal::new(f32::MAX));
    let (outer, inner) = (page.clone(), across.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        column().on_press(|| {}).child(
            scroll(
                row()
                    .child(
                        scroll(grid())
                            .axis(ScrollAxis::Both)
                            .horizontal_offset(inner.clone())
                            .width(px(150.0))
                            .height(px(100.0))
                            .shrink(0.0),
                    )
                    .child(panel().width(px(500.0)).height(px(100.0)).shrink(0.0)),
            )
            .axis(ScrollAxis::Horizontal)
            .horizontal_offset(outer.clone())
            .width(px(200.0))
            .height(px(100.0)),
        )
    });
    harness.settle();

    harness.button_down(Point { x: 50.0, y: 50.0 });
    harness.move_to(Point { x: 260.0, y: 150.0 });
    for _ in 0..10 {
        harness.advance(std::time::Duration::from_millis(16));
    }
    harness.button_up(Point { x: 260.0, y: 150.0 });
    assert!(
        page.get() > 0.0,
        "the grid at its right end left that axis to the page"
    );
}

#[test]
fn an_exact_diagonal_scrolls_a_strip_that_scrolls_only_sideways() {
    let (mut harness, _, across) = view(ScrollAxis::Horizontal, || strip(20, 30.0, 60.0));
    wheel(&mut harness, 10.0, 10.0);
    harness.frame();
    assert_eq!(across.get(), 10.0);
}
