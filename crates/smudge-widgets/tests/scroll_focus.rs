//! Whole-frame behavior of a scroll view when the keyboard focus moves to a
//! node inside it, driven through the headless harness.
//!
//! The focusable rows have distinct heights, 40, 41 and 42 logical pixels,
//! so their rectangles can be told apart, and tall spacers that take no focus
//! sit between them, so a Tab lands on a row that is wholly out of view.

use smudge_core::element::{Element, IntoElement, for_each};
use smudge_core::geometry::{Color, Point, Rect};
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::{Panel, Widget, panel};
use smudge_test::Harness;

use smudge_widgets::{ScrollExt, column, scroll};

/// A focusable row with a distinct height and fill.
fn row(height: f32, color: Color) -> Element<Panel> {
    panel()
        .height(px(height))
        .shrink(0.0)
        .background(color)
        .focusable()
}

/// A spacer that takes no focus.
fn spacer(height: f32) -> Element<Panel> {
    panel()
        .height(px(height))
        .shrink(0.0)
        .background(Color::rgb(230, 230, 230))
}

/// The rows A, B and C, 40, 41 and 42 pixels tall, with 100 pixel spacers
/// between them: A spans 0 to 40, B 140 to 181 and C 281 to 323 of the
/// content, so a view 100 pixels tall scrolls up to 223.
fn spaced_rows(c: Element<Panel>) -> Element<impl Widget> {
    column().children([
        row(40.0, Color::rgb(200, 0, 0)),
        spacer(100.0),
        row(41.0, Color::rgb(0, 200, 0)),
        spacer(100.0),
        c,
    ])
}

/// The row C of [`spaced_rows`] with nothing added.
fn plain_c() -> Element<Panel> {
    row(42.0, Color::rgb(0, 0, 200))
}

/// A settled harness with a view 100 pixels tall over the spaced rows, its
/// offset bound to `offset`.
fn spaced_view(offset: RwSignal<f32>) -> Harness {
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(spaced_rows(plain_c()))
                .offset(offset)
                .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();
    harness
}

/// Every visible node's widget type name and absolute rectangle, in paint
/// order.
fn nodes(harness: &mut Harness) -> Vec<(String, Rect)> {
    let mut nodes = Vec::new();
    harness.app().visit_nodes(|_, type_name, rect| {
        let path = type_name.split('<').next().unwrap_or(type_name);
        let name = path.rsplit("::").next().unwrap_or(path);
        nodes.push((name.to_owned(), rect));
    });
    nodes
}

/// The top edge of the panel `height` pixels tall.
fn top_of(harness: &mut Harness, height: f32) -> f32 {
    nodes(harness)
        .into_iter()
        .find(|(name, rect)| name == "Panel" && rect.height() == height)
        .map(|(_, rect)| rect.y0)
        .unwrap_or_else(|| panic!("a panel {height} pixels tall is laid out"))
}

/// The rectangles of the scroll views, outermost first.
fn views(harness: &mut Harness) -> Vec<Rect> {
    nodes(harness)
        .into_iter()
        .filter(|(name, _)| name == "ScrollView")
        .map(|(_, rect)| rect)
        .collect()
}

/// Presses Tab, backwards when `back` is set, and runs the frame it asks for.
fn tab(harness: &mut Harness, back: bool) {
    let modifiers = Modifiers {
        shift: back,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::Tab, modifiers);
    harness.frame();
}

#[test]
fn tab_scrolls_each_row_into_view_at_the_nearest_edge() {
    let offset = RwSignal::new(0.0);
    let mut harness = spaced_view(offset.clone());

    tab(&mut harness, false);
    assert_eq!(
        top_of(&mut harness, 40.0),
        0.0,
        "the top of A, already in view"
    );

    tab(&mut harness, false);
    assert_eq!(
        top_of(&mut harness, 41.0),
        59.0,
        "the top of B, at the bottom edge"
    );

    tab(&mut harness, false);
    assert_eq!(
        top_of(&mut harness, 42.0),
        58.0,
        "the top of C, at the bottom edge"
    );
    harness.settle();
    assert_eq!(offset.read(|value| *value), 223.0, "the view is at its end");

    tab(&mut harness, true);
    assert_eq!(
        top_of(&mut harness, 41.0),
        0.0,
        "the top of B, at the top edge"
    );

    tab(&mut harness, true);
    assert_eq!(
        top_of(&mut harness, 40.0),
        0.0,
        "the top of A, at the top edge"
    );
    harness.settle();
    assert_eq!(
        offset.read(|value| *value),
        0.0,
        "the view is back at its top"
    );
}

#[test]
fn the_scroll_lands_in_the_frame_of_the_tab_without_a_layout_pass() {
    let mut harness = spaced_view(RwSignal::new(0.0));
    tab(&mut harness, false);

    harness.press_key(Key::Tab);
    assert!(
        harness.app().needs_frame(),
        "the focus move asks for a frame"
    );
    let report = harness.frame();

    assert_eq!(top_of(&mut harness, 41.0), 59.0, "the top of B");
    assert!(!report.layout_ran, "a scroll offset needs no layout pass");
    assert_eq!(report.nodes_painted, 0, "the moved rows keep their paint");
}

#[test]
fn focusing_rows_already_in_view_scrolls_nothing() {
    let offset = RwSignal::new(0.0);
    let mut harness = Harness::new(120.0, 200.0, 1.0, {
        let offset = offset.clone();
        move || {
            panel().column().child(
                scroll(column().children([
                    spacer(20.0),
                    row(40.0, Color::rgb(200, 0, 0)),
                    row(41.0, Color::rgb(0, 200, 0)),
                    spacer(200.0),
                ]))
                .offset(offset)
                .height(px(100.0)),
            )
        }
    });
    harness.frame();
    harness.settle();
    offset.set(10.0);
    harness.frame();
    harness.settle();

    for _ in 0..2 {
        tab(&mut harness, false);
        assert!(
            !harness.woken(),
            "no reveal handler wrote the offset signal"
        );
    }

    assert_eq!(offset.read(|value| *value), 10.0);
    assert_eq!(top_of(&mut harness, 40.0), 10.0, "the top of A");
    assert_eq!(top_of(&mut harness, 41.0), 50.0, "the top of B");
}

#[test]
fn a_press_that_focuses_a_row_cut_by_the_edge_scrolls_nothing() {
    let offset = RwSignal::new(20.0);
    let mut harness = spaced_view(offset.clone());
    assert_eq!(
        top_of(&mut harness, 40.0),
        -20.0,
        "A is cut by the top edge"
    );
    // Two Tabs before the press ask to reveal B. The press moves the focus
    // on, so that request no longer stands.
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);

    harness.click(Point { x: 60.0, y: 10.0 });
    harness.frame();
    harness.settle();

    assert!(harness.app().focused().is_some(), "the press focused A");
    assert_eq!(offset.read(|value| *value), 20.0);
    assert_eq!(top_of(&mut harness, 40.0), -20.0, "the top of A");
}

#[test]
fn an_autofocused_row_below_the_fold_shows_on_its_first_frame() {
    let offset = RwSignal::new(0.0);
    let mut harness = Harness::new(120.0, 200.0, 1.0, {
        let offset = offset.clone();
        move || {
            panel().column().child(
                scroll(spaced_rows(plain_c().autofocus()))
                    .offset(offset)
                    .height(px(100.0)),
            )
        }
    });

    harness.frame();
    assert_eq!(
        top_of(&mut harness, 42.0),
        58.0,
        "the top of C on the first frame"
    );

    harness.settle();
    assert_eq!(
        top_of(&mut harness, 42.0),
        58.0,
        "the top of C once settled"
    );
    assert_eq!(offset.read(|value| *value), 223.0);
    harness.assert_matches_full_repaint();
}

#[test]
fn a_focus_style_that_grows_an_autofocused_row_still_ends_in_view() {
    let mut harness = Harness::new(120.0, 200.0, 1.0, || {
        panel().column().child(
            scroll(spaced_rows(
                plain_c()
                    .autofocus()
                    .focused(|patch| patch.height(px(60.0))),
            ))
            .height(px(100.0)),
        )
    });
    // The focus style shows only while the keyboard navigates.
    harness.press_key(Key::ArrowDown);

    harness.frame();

    assert_eq!(top_of(&mut harness, 60.0), 40.0, "the top of the grown C");
    harness.settle();
    assert_eq!(top_of(&mut harness, 60.0), 40.0, "the top of the grown C");
}

#[test]
fn a_remount_that_restores_the_focus_below_the_fold_shows_it() {
    // The view owns its offset, so the new build mounts at the top and only
    // the restored focus can move it.
    let build = || {
        panel()
            .column()
            .child(scroll(spaced_rows(plain_c())).height(px(100.0)))
    };
    let mut harness = Harness::new(120.0, 200.0, 1.0, build);
    harness.frame();
    harness.settle();
    for _ in 0..3 {
        tab(&mut harness, false);
    }
    harness.settle();
    assert_eq!(
        top_of(&mut harness, 42.0),
        58.0,
        "the top of C before the remount"
    );

    harness.app().remount(build);
    harness.frame();

    assert!(harness.app().focused().is_some(), "the focus was restored");
    assert_eq!(
        top_of(&mut harness, 42.0),
        58.0,
        "the top of C after the remount"
    );
    harness.settle();
    assert_eq!(
        top_of(&mut harness, 42.0),
        58.0,
        "the top of C once settled"
    );
}

/// A settled harness with a view 100 pixels tall whose content holds a 150
/// pixel spacer, a nested view 60 pixels tall and another 150 pixel spacer.
/// The nested view holds a 100 pixel spacer, a focusable row `height` pixels
/// tall and a 50 pixel spacer. The offsets are bound to `outer` and `inner`.
fn nested_view(height: f32, outer: RwSignal<f32>, inner: RwSignal<f32>) -> Harness {
    let mut harness = Harness::new(120.0, 200.0, 1.0, move || {
        panel().column().child(
            scroll(
                column().children([
                    spacer(150.0).into_any(),
                    scroll(column().children([
                        spacer(100.0),
                        row(height, Color::rgb(0, 0, 200)),
                        spacer(50.0),
                    ]))
                    .offset(inner)
                    .height(px(60.0))
                    .shrink(0.0)
                    .into_any(),
                    spacer(150.0).into_any(),
                ]),
            )
            .offset(outer)
            .height(px(100.0)),
        )
    });
    harness.frame();
    harness.settle();
    harness
}

#[test]
fn nested_views_both_scroll_to_show_the_focus() {
    let outer = RwSignal::new(0.0);
    let inner = RwSignal::new(0.0);
    let mut harness = nested_view(30.0, outer.clone(), inner.clone());

    tab(&mut harness, false);

    // The inner view shows the row at its bottom edge, and the outer view
    // shows the inner view's bottom edge, where the row is.
    let views = views(&mut harness);
    assert_eq!(views[1].y0, 40.0, "the top of the inner view");
    assert_eq!(top_of(&mut harness, 30.0), 70.0, "the top of the row");
    harness.assert_matches_full_repaint();
    harness.settle();
    assert_eq!(inner.read(|value| *value), 70.0);
    assert_eq!(outer.read(|value| *value), 110.0);
    harness.assert_matches_full_repaint();
}

#[test]
fn the_outer_view_reveals_the_part_of_a_tall_row_the_inner_view_shows() {
    let outer = RwSignal::new(0.0);
    let inner = RwSignal::new(0.0);
    let mut harness = nested_view(80.0, outer.clone(), inner.clone());

    tab(&mut harness, false);

    // The row is taller than the inner view, which aligns its top. The
    // outer view then shows the whole inner view, not the part of the row
    // the inner view clips.
    let views = views(&mut harness);
    assert_eq!(views[1].y0, 40.0, "the top of the inner view");
    assert_eq!(top_of(&mut harness, 80.0), 40.0, "the top of the row");
    harness.settle();
    assert_eq!(inner.read(|value| *value), 100.0);
    assert_eq!(outer.read(|value| *value), 110.0);
}

#[test]
fn a_wheel_step_after_a_reveal_continues_from_the_revealed_offset() {
    let offset = RwSignal::new(0.0);
    let mut harness = spaced_view(offset.clone());
    tab(&mut harness, false);
    tab(&mut harness, false);
    harness.settle();
    assert_eq!(
        offset.read(|value| *value),
        81.0,
        "the signal took the offset"
    );

    harness.move_to(Point { x: 60.0, y: 50.0 });
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: 10.0 },
    });
    harness.frame();

    assert_eq!(offset.read(|value| *value), 91.0);
    assert_eq!(top_of(&mut harness, 41.0), 49.0, "the top of B");
}

#[test]
fn the_frames_of_a_reveal_match_a_full_repaint() {
    let mut harness = spaced_view(RwSignal::new(0.0));
    tab(&mut harness, false);
    harness.assert_matches_full_repaint();

    tab(&mut harness, false);
    harness.assert_matches_full_repaint();
    // The thumb follows the offset signal one frame later.
    harness.frame();
    harness.assert_matches_full_repaint();

    tab(&mut harness, true);
    harness.assert_matches_full_repaint();
    harness.frame();
    harness.assert_matches_full_repaint();
}

#[test]
fn a_reveal_that_scrolls_up_releases_following() {
    let following = RwSignal::new(true);
    let heights = RwSignal::new(vec![40u64, 41, 42]);
    let mut harness = Harness::new(120.0, 200.0, 1.0, {
        let following = following.clone();
        let heights = heights.clone();
        move || {
            panel().column().child(
                scroll(
                    column().children([
                        row(39.0, Color::rgb(200, 0, 0)).into_any(),
                        for_each(
                            move || heights.get(),
                            |height| *height,
                            |height| spacer(height as f32 * 2.0),
                        )
                        .column()
                        .into_any(),
                    ]),
                )
                .follow_end(following)
                .height(px(100.0)),
            )
        }
    });
    harness.frame();
    harness.settle();
    assert_eq!(views(&mut harness)[0].y0, 0.0, "the top of the view");

    tab(&mut harness, false);
    harness.settle();

    assert_eq!(
        top_of(&mut harness, 39.0),
        0.0,
        "the top of the focused row"
    );
    assert!(
        !following.read(|active| *active),
        "the view let go of the end"
    );

    heights.update(|heights| heights.push(43));
    harness.frame();
    harness.settle();
    assert_eq!(
        top_of(&mut harness, 39.0),
        0.0,
        "the top of the row after an append"
    );
}
