//! Whole-frame behavior of the scroll view, driven through the headless
//! harness.
//!
//! The tests give the content rows distinct heights so their rectangles can be
//! told apart. They settle the harness after mounting and after every change,
//! because the view measures its heights when it paints, so the thumb and the
//! clamp follow a size one frame after the frame that painted it.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::input::InputEvent;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;
use smudge_text::{TextSystem, text_input};

use smudge_widgets::{ScrollExt, button, column, scroll};

/// A content row with a distinct height and fill, so its rectangle can be
/// told apart from its neighbors by height alone.
fn row(height: f32, color: Color) -> Element<Panel> {
    panel().height(px(height)).background(color)
}

/// One visible node: its depth, its widget type's short name and its
/// absolute rectangle.
#[derive(Clone, Debug)]
struct Node {
    /// The depth in the tree, with the root at zero.
    depth: usize,
    /// The last path segment of the widget's type name.
    name: String,
    /// The node's absolute rectangle.
    rect: Rect,
}

/// Every visible node, in paint order.
fn nodes(harness: &mut Harness) -> Vec<Node> {
    let mut nodes = Vec::new();
    harness.app().visit_nodes(|depth, type_name, rect| {
        let name = type_name.rsplit("::").next().unwrap_or(type_name);
        nodes.push(Node {
            depth,
            name: name.to_owned(),
            rect,
        });
    });
    nodes
}

/// The content rows of the three-row scene, told apart by height.
fn content_rows(harness: &mut Harness) -> Vec<Rect> {
    let heights = [60.0, 61.0, 62.0];
    let mut rows: Vec<Rect> = nodes(harness)
        .into_iter()
        .filter(|node| node.name == "Panel" && heights.contains(&node.rect.height()))
        .map(|node| node.rect)
        .collect();
    rows.sort_by_key(|rect| rect.y0 as i64);
    rows
}

/// The scrollbar thumb of a view that is a child of the root: the panel that
/// hangs from the track, a child of the view.
fn thumb(harness: &mut Harness) -> Option<Rect> {
    nodes(harness)
        .into_iter()
        .find(|node| node.name == "Panel" && node.depth == 3 && node.rect.width() == 4.0)
        .map(|node| node.rect)
}

/// Reports whether two logical positions are the same point.
fn same_point(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01
}

/// Wheels the view by `down` logical pixels, scrolling down when positive.
fn wheel(harness: &mut Harness, down: f32) {
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: down },
    });
}

/// The three rows 60, 61 and 62 logical pixels tall, 183 in all.
fn three_rows() -> Element<impl smudge_core::widget::Widget> {
    column().children([
        row(60.0, Color::rgb(200, 0, 0)),
        row(61.0, Color::rgb(0, 200, 0)),
        row(62.0, Color::rgb(0, 0, 200)),
    ])
}

/// A scroll view of the three rows in a viewport 100 pixels tall, settled.
///
/// The root is a column so the view stretches to the window's width; in a row
/// parent a view takes its content's width.
fn tall_view() -> Harness {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel()
            .column()
            .child(scroll(three_rows()).height(px(100.0)))
    });
    harness.frame();
    harness.settle();
    harness
}

#[test]
fn wheel_down_moves_the_content_up_without_layout() {
    let mut harness = tall_view();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    let report = harness.frame();

    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -30.0 }),
        "the first row moved up by the wheel delta, at {:?}",
        rows[0]
    );
    assert!(
        !report.layout_ran,
        "a scroll moves the content and the thumb without a layout pass"
    );
    assert_eq!(harness.settle(), 0, "the scroll settled in its own frame");
    harness.assert_matches_full_repaint();
}

#[test]
fn two_wheel_events_before_a_frame_add_up() {
    let mut harness = tall_view();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    wheel(&mut harness, 30.0);
    harness.frame();

    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -60.0 }),
        "both steps moved the content, first row at {:?}",
        rows[0]
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn wheel_clamps_at_the_end_of_the_content() {
    let mut harness = tall_view();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    harness.frame();
    wheel(&mut harness, 1_000.0);
    harness.frame();

    // The content is 183 pixels in a 100 pixel viewport, so the furthest the
    // view can scroll is 83.
    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -83.0 }),
        "the view stopped at the end of the content, first row at {:?}",
        rows[0]
    );
    assert!(
        same_point(rows[2].origin(), Point { x: 0.0, y: 38.0 }),
        "the last row's bottom edge meets the viewport, at {:?}",
        rows[2]
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn an_offset_signal_jumps_to_a_clamped_position() {
    let offset = RwSignal::new(0.0);
    let signal = offset.clone();
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        panel()
            .column()
            .child(scroll(three_rows()).offset(signal).height(px(100.0)))
    });
    harness.frame();
    harness.settle();

    offset.set(1_000.0);
    harness.frame();
    harness.settle();

    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -83.0 }),
        "the jump clamped to the end of the content, first row at {:?}",
        rows[0]
    );
    assert_eq!(
        offset.read(|value| *value),
        83.0,
        "the view wrote the clamped offset back to the signal"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_view_mounted_with_an_offset_opens_at_that_offset() {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(three_rows())
                .offset(RwSignal::new(40.0))
                .height(px(100.0)),
        )
    });
    harness.frame();

    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -40.0 }),
        "the first frame already shows the offset, first row at {:?}",
        rows[0]
    );

    harness.settle();

    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -40.0 }),
        "measuring the heights keeps an offset that fits, first row at {:?}",
        rows[0]
    );
}

#[test]
fn a_view_below_and_beside_other_content_scrolls_its_own_rows() {
    let mut harness = Harness::new(160.0, 200.0, 1.0, || {
        panel()
            .column()
            .child(panel().height(px(30.0)))
            .child(scroll(three_rows()).margin(px(10.0)).height(px(100.0)))
    });
    harness.frame();
    harness.settle();
    let view = nodes(&mut harness)
        .into_iter()
        .find(|node| node.name == "ScrollView")
        .expect("the view is laid out")
        .rect;
    harness.move_to(Point { x: 80.0, y: 80.0 });

    wheel(&mut harness, 30.0);
    harness.frame();

    let after = nodes(&mut harness)
        .into_iter()
        .find(|node| node.name == "ScrollView")
        .expect("the view is laid out")
        .rect;
    assert_eq!(after, view, "the view itself does not move when it scrolls");
    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 10.0, y: 10.0 }),
        "the first row moved up by the delta inside the view, at {:?}",
        rows[0]
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_bordered_and_padded_view_scrolls_its_last_row_to_its_content_box() {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(three_rows())
                .border(3.0)
                .border_color(Color::BLACK)
                .padding(px(5.0))
                .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 1_000.0);
    harness.frame();
    harness.settle();

    // The content box is 100 - 2 * 3 - 2 * 5 = 84 pixels tall and starts 8
    // pixels down, so the last row's bottom edge rests at 92.
    let rows = content_rows(&mut harness);
    assert!(
        (rows[2].y1 - 92.0).abs() < 0.01,
        "the last row ends at the bottom of the content box, at {:?}",
        rows[2]
    );
    let thumb = thumb(&mut harness).expect("the content overflows");
    assert!(
        (thumb.y1 - 97.0).abs() < 0.01,
        "the thumb ends at the inside of the bottom border, at {thumb:?}"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_button_below_the_fold_clicks_after_scrolling_to_it() {
    TextSystem::bundled_only().install();
    let presses = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&presses);
    let offset = RwSignal::new(0.0);
    let signal = offset.clone();
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(
                column()
                    .child(row(80.0, Color::rgb(200, 0, 0)))
                    .child(row(80.0, Color::rgb(0, 200, 0)))
                    .child(button("Press me").on_press(move || {
                        counted.set(counted.get() + 1);
                    })),
            )
            .offset(signal)
            .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();

    // The button sits below the fold. Jumping past its end clamps the offset,
    // and the clamped value is what the button's position is measured with.
    let button_rect = nodes(&mut harness)
        .into_iter()
        .filter(|node| node.name == "Panel" && node.rect.y0 >= 160.0)
        .map(|node| node.rect)
        .next()
        .expect("the button is laid out below the fold");
    offset.set(1_000.0);
    harness.frame();
    harness.settle();
    let scrolled_by = offset.read(|value| *value);

    harness.click(Point {
        x: 60.0,
        y: (button_rect.y0 + button_rect.y1) / 2.0 - scrolled_by,
    });

    assert_eq!(presses.get(), 1, "the press hit the scrolled button");
}

#[test]
fn scrolling_damages_only_the_scroll_viewport() {
    let mut harness = Harness::new(120.0, 100.0, 1.0, || {
        panel()
            .row()
            .child(
                scroll(column().children([
                    row(40.0, Color::rgb(200, 0, 0)),
                    row(40.0, Color::rgb(0, 200, 0)),
                    row(40.0, Color::rgb(0, 0, 200)),
                ]))
                .size(Size {
                    width: 60.0,
                    height: 100.0,
                }),
            )
            .child(
                panel()
                    .width(px(60.0))
                    .height(px(100.0))
                    .background(Color::rgb(255, 220, 0)),
            )
    });
    harness.frame();
    harness.settle();
    harness.move_to(Point { x: 30.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    let report = harness.frame();

    assert!(!report.damage.is_empty(), "the scroll changed pixels");
    assert!(!report.full, "a scroll is never a full repaint");
    for rect in &report.damage {
        assert!(
            rect.x + rect.width <= 60,
            "the damage left the scroll viewport: {rect:?}"
        );
    }
    harness.assert_matches_full_repaint();
}

#[test]
fn the_thumb_hides_while_the_content_fits() {
    let mut harness = Harness::new(120.0, 100.0, 1.0, || {
        panel()
            .column()
            .child(scroll(column().child(row(40.0, Color::rgb(200, 0, 0)))).height(px(100.0)))
    });
    harness.frame();
    harness.settle();

    assert!(
        thumb(&mut harness).is_none(),
        "the thumb is hidden while the content fits"
    );
}

#[test]
fn the_thumb_shows_the_position_at_both_ends() {
    let mut harness = tall_view();
    let at_top = thumb(&mut harness).expect("the thumb is visible when the content overflows");
    assert!(
        same_point(
            at_top.origin(),
            Point {
                x: 120.0 - 2.0 - 4.0,
                y: 0.0,
            }
        ),
        "the thumb starts at the top right, at {at_top:?}"
    );
    let expected_length = 100.0 * 100.0 / 183.0;
    assert!(
        (at_top.height() - expected_length).abs() <= 1.0,
        "the thumb is the viewport's share of the content, at {at_top:?}"
    );

    harness.move_to(Point { x: 60.0, y: 50.0 });
    wheel(&mut harness, 1_000.0);
    harness.frame();

    let at_end = thumb(&mut harness).expect("the thumb is still visible at the end");
    // The offset maps onto the track the thumb's length leaves, so its bottom
    // edge lands exactly on the viewport's bottom edge at the end.
    assert!(
        (at_end.y1 - 100.0).abs() < 0.01,
        "the thumb ended at the bottom of the viewport, at {at_end:?}"
    );
    assert_eq!(
        at_end.x0, at_top.x0,
        "the thumb stays at the right edge while it moves"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn the_thumb_follows_a_viewport_that_grows_past_the_content() {
    let height = RwSignal::new(100.0);
    let bound = height.clone();
    let mut harness = Harness::new(120.0, 300.0, 1.0, move || {
        panel()
            .column()
            .child(scroll(three_rows()).height(move || px(bound.get())))
    });
    harness.frame();
    harness.settle();
    assert!(
        thumb(&mut harness).is_some(),
        "the content overflows at first"
    );

    height.set(250.0);
    harness.frame();
    harness.settle();

    assert!(
        thumb(&mut harness).is_none(),
        "the thumb hides once the content fits the taller view"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn wheel_does_nothing_while_the_content_fits() {
    let mut harness = Harness::new(120.0, 100.0, 1.0, || {
        panel()
            .column()
            .child(scroll(column().child(row(40.0, Color::rgb(200, 0, 0)))).height(px(100.0)))
    });
    harness.frame();
    harness.settle();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    let report = harness.frame();

    assert_eq!(report.nodes_painted, 0, "nothing moved, so nothing paints");
    assert!(report.damage.is_empty(), "no change, so no damage");
    assert!(
        !report.layout_ran,
        "the offset never moved, so no layout runs"
    );
}

#[test]
fn a_wheel_frame_repaints_no_node() {
    let mut harness = tall_view();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    let report = harness.frame();

    assert_eq!(
        report.nodes_painted, 0,
        "the content and the thumb move without any node repainting"
    );
    assert!(
        !report.damage.is_empty(),
        "the scroll damages where the ink moved"
    );
    harness.assert_matches_full_repaint();
}

/// A view of two rows with a badge that hangs off the view itself, outside
/// the flow, settled.
fn badged_view() -> Harness {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(column().children([
                row(60.0, Color::rgb(200, 0, 0)),
                row(61.0, Color::rgb(0, 200, 0)),
            ]))
            .height(px(100.0))
            .child(
                panel()
                    .width(px(16.0))
                    .height(px(16.0))
                    .background(Color::rgb(0, 0, 200))
                    .absolute()
                    .inset(Edges {
                        top: Length::px(4.0),
                        left: Length::px(4.0),
                        right: Length::Auto,
                        bottom: Length::Auto,
                    }),
            ),
        )
    });
    harness.frame();
    harness.settle();
    harness
}

/// The badge of the badged view: the 16 by 16 panel hanging off the view.
fn badge(harness: &mut Harness) -> Rect {
    nodes(harness)
        .into_iter()
        .find(|node| node.name == "Panel" && node.rect.width() == 16.0)
        .expect("the badge is mounted")
        .rect
}

#[test]
fn an_absolute_child_stays_put_while_the_content_scrolls() {
    let mut harness = badged_view();
    let before = badge(&mut harness);
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    let report = harness.frame();

    assert_eq!(
        badge(&mut harness),
        before,
        "the scroll leaves the badge where its insets put it"
    );
    let rows = content_rows(&mut harness);
    assert!(
        same_point(rows[0].origin(), Point { x: 0.0, y: -21.0 }),
        "the content moved under the badge, clamped to the 21 pixels of range, first row at {:?}",
        rows[0]
    );
    assert_eq!(report.nodes_painted, 0, "nothing needed repainting");
    harness.assert_matches_full_repaint();
}

/// A view with a second view nested between its rows, settled.
fn nested_views() -> Harness {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(
                column()
                    .child(row(60.0, Color::rgb(200, 0, 0)))
                    .child(
                        scroll(
                            column()
                                .child(row(40.0, Color::rgb(0, 200, 0)))
                                .child(row(41.0, Color::rgb(0, 0, 200))),
                        )
                        .height(px(50.0)),
                    )
                    .child(row(62.0, Color::rgb(10, 10, 10))),
            )
            .height(px(163.0)),
        )
    });
    harness.frame();
    harness.settle();
    harness
}

/// The rows of the outer and the inner view of the nested scene, by height.
fn nested_rows(harness: &mut Harness) -> Vec<Rect> {
    let heights = [60.0, 62.0, 40.0];
    let mut rows: Vec<Rect> = nodes(harness)
        .into_iter()
        .filter(|node| node.name == "Panel" && heights.contains(&node.rect.height()))
        .map(|node| node.rect)
        .collect();
    rows.sort_by_key(|rect| rect.y0 as i64);
    rows
}

#[test]
fn a_nested_view_scrolls_alone() {
    let mut harness = nested_views();
    // The inner view spans 50 pixels from 60 down, under the first row.
    harness.move_to(Point { x: 60.0, y: 80.0 });

    wheel(&mut harness, 10.0);
    let report = harness.frame();

    let rows = nested_rows(&mut harness);
    // Rows top to bottom: the outer first row at 0, the inner first row at
    // 50 after its scroll, the outer last row at 110, below the inner view.
    assert!(
        same_point(rows[1].origin(), Point { x: 0.0, y: 50.0 }),
        "the inner view's content moved, at {:?}",
        rows[1]
    );
    assert!(
        same_point(rows[2].origin(), Point { x: 0.0, y: 110.0 }),
        "the outer view's content stayed put, at {:?}",
        rows[2]
    );
    assert_eq!(report.nodes_painted, 0, "neither view repainted a node");
    harness.assert_matches_full_repaint();
}

#[test]
fn the_input_method_area_stays_in_window_coordinates_inside_a_scrolled_view() {
    let bound = RwSignal::new(String::from("hello"));
    let for_input = bound.clone();
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(
                column()
                    .child(row(150.0, Color::rgb(200, 0, 0)))
                    .child(text_input(for_input.clone()).height(px(30.0))),
            )
            .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();

    // The input sits at 150, so the view's 80 pixels of range bring it to 70.
    harness.move_to(Point { x: 60.0, y: 50.0 });
    wheel(&mut harness, 80.0);
    harness.frame();
    harness.settle();
    harness.click(Point { x: 60.0, y: 85.0 });
    harness.settle();

    let area = harness.app().ime_area().expect("the input reports an area");
    assert!(
        area.y0 >= 70.0 && area.y1 <= 100.0,
        "the anchor sits inside the input's on-screen band 70 to 100, got {area:?}"
    );
}

#[test]
fn a_press_after_a_sideways_scroll_places_the_caret_under_the_pointer() {
    let bound = RwSignal::new(String::from("abcdef"));
    let for_input = bound.clone();
    let offset = RwSignal::new(0.0_f32);
    let for_view = offset.clone();
    let mut harness = Harness::new(200.0, 60.0, 1.0, move || {
        let for_view = for_view.clone();
        panel().column().child(
            panel()
                .clip(true)
                .scroll_offset(move || Point {
                    x: for_view.get(),
                    y: 0.0,
                })
                .child(
                    text_input(for_input.clone())
                        .width(px(150.0))
                        .height(px(30.0)),
                ),
        )
    });
    harness.frame();
    harness.settle();
    // A negative offset moves the input 30 pixels to the right, so a stale
    // origin would put this press well inside the text.
    offset.set(-30.0);
    harness.frame();
    harness.settle();

    let input = nodes(&mut harness)
        .into_iter()
        .find(|node| node.name == "TextInput")
        .expect("the input is mounted")
        .rect;
    harness.click(Point {
        x: input.x0 + 2.0,
        y: input.y0 + input.height() / 2.0,
    });
    harness.settle();
    harness.type_text("X");
    harness.settle();

    assert_eq!(
        bound.get(),
        "Xabcdef",
        "the press at the input's left edge put the caret before the text"
    );
}

/// Scrolls the settled `harness` by `step` 40 times, checks every frame
/// against a full repaint, and returns how many nodes the frames painted.
fn scroll_steps(harness: &mut Harness, step: f32) -> u32 {
    harness.move_to(Point { x: 60.0, y: 50.0 });
    let mut painted = 0;
    for _ in 0..40 {
        wheel(harness, step);
        painted += harness.frame().nodes_painted;
        harness.assert_matches_full_repaint();
    }
    painted
}

#[test]
fn fractional_wheel_steps_repaint_no_node() {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(column().children(
                (0..20).map(|i| row(10.3 + i as f32 * 0.17, Color::rgb(200, i * 10, 0))),
            ))
            .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();

    assert_eq!(
        scroll_steps(&mut harness, 0.37),
        0,
        "content that only scrolled keeps its operations at any step"
    );
}

#[test]
fn scrolling_clipping_cards_paints_only_the_cards_entering_or_leaving() {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(column().children((0..20).map(|i| {
                panel()
                    .clip(true)
                    .height(px(30.0))
                    .child(row(20.0, Color::rgb(0, i * 10, 200)))
            })))
            .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();

    // 40 steps of 3 move 120 pixels, which takes four cards out of the
    // view and four in, and each of their rows paints once as its clip
    // stops or starts being empty.
    assert_eq!(scroll_steps(&mut harness, 3.0), 8);
}

#[test]
fn a_scroll_and_a_repaint_in_one_frame_match_a_full_repaint() {
    let color = RwSignal::new(Color::rgb(0, 200, 0));
    let for_row = color.clone();
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        let for_row = for_row.clone();
        panel().column().child(
            scroll(
                column().children([
                    row(60.0, Color::rgb(200, 0, 0)),
                    panel()
                        .height(px(61.0))
                        .background(move || for_row.get().into()),
                    row(62.0, Color::rgb(0, 0, 200)),
                ]),
            )
            .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    color.set(Color::rgb(250, 120, 0));
    let report = harness.frame();

    assert_eq!(report.nodes_painted, 1, "only the recolored row paints");
    harness.assert_matches_full_repaint();
}

/// A view whose flowing content is split by a badge out of the flow, with its
/// offset bound to `offset`, settled.
fn split_view(offset: RwSignal<f32>) -> Harness {
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(row(150.0, Color::rgb(200, 0, 0)))
                .offset(offset.clone())
                .height(px(100.0))
                .child(
                    panel()
                        .width(px(16.0))
                        .height(px(16.0))
                        .background(Color::rgb(0, 0, 200))
                        .absolute()
                        .inset(Edges {
                            top: Length::px(4.0),
                            left: Length::px(4.0),
                            right: Length::Auto,
                            bottom: Length::Auto,
                        }),
                )
                .child(row(80.0, Color::rgb(0, 200, 0)).shrink(0.0)),
        )
    });
    harness.frame();
    harness.settle();
    harness
}

#[test]
fn an_absolute_child_between_flowing_ones_stays_put_while_they_scroll() {
    let mut harness = split_view(RwSignal::new(0.0));
    let before = badge(&mut harness);
    harness.move_to(Point { x: 60.0, y: 50.0 });

    wheel(&mut harness, 30.0);
    let report = harness.frame();

    let after = nodes(&mut harness)
        .into_iter()
        .find(|node| node.name == "Panel" && node.rect.height() == 80.0)
        .expect("the flowing row after the badge is mounted")
        .rect;
    assert!(
        same_point(after.origin(), Point { x: 0.0, y: 120.0 }),
        "the flowing row after the badge scrolled with the content, at {after:?}"
    );
    assert_eq!(badge(&mut harness), before);
    assert_eq!(report.nodes_painted, 0);
    harness.assert_matches(&split_view(RwSignal::new(30.0)));
}
