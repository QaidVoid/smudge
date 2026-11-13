//! Behavior tests for scrolling the focused node into view, through the
//! element builder a scrolling node opts in with.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Point, Rect};
use smudge_core::input::Key;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, Overflow, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// A node 100 pixels tall that clips and scrolls a 300 pixel spacer and a
/// focusable panel 20 pixels tall below it, at the offset `offset` holds.
fn scroller(offset: RwSignal<f32>) -> Element<Panel> {
    Element::new(Panel)
        .column()
        .height(px(100.0))
        .overflow(Overflow::Scroll)
        .scroll_offset(move || Point {
            x: 0.0,
            y: offset.get(),
        })
        .child(panel().height(px(300.0)).shrink(0.0))
        .child(
            panel()
                .height(px(20.0))
                .shrink(0.0)
                .background(Color::rgb(0, 0, 200))
                .focusable(),
        )
}

/// The rectangle of the focusable panel, the only one 20 pixels tall.
fn target(harness: &mut Harness) -> Rect {
    let mut found = None;
    harness.app().visit_nodes(|_, _, rect| {
        if rect.height() == 20.0 {
            found = Some(rect);
        }
    });
    found.expect("the focusable panel is laid out")
}

#[test]
fn a_scrolling_node_without_a_reveal_handler_stays_put() {
    let offset = RwSignal::new(0.0);
    let mut harness = Harness::new(100.0, 100.0, 1.0, {
        let offset = offset.clone();
        move || scroller(offset)
    });
    harness.frame();

    harness.press_key(Key::Tab);
    harness.frame();

    assert!(
        harness.app().focused().is_some(),
        "the panel took the focus"
    );
    assert_eq!(target(&mut harness).y0, 300.0, "nothing scrolled");
}

#[test]
fn a_focus_request_scrolls_the_node_back_into_view() {
    let offset = RwSignal::new(0.0);
    let calls = Rc::new(Cell::new(0u32));
    let mut harness = Harness::new(100.0, 100.0, 1.0, {
        let offset = offset.clone();
        let calls = Rc::clone(&calls);
        move || {
            let write = offset.clone();
            scroller(offset).on_reveal(move |revealed| {
                calls.set(calls.get() + 1);
                write.set(revealed.y);
            })
        }
    });
    harness.frame();
    harness.press_key(Key::Tab);
    harness.frame();
    assert_eq!(target(&mut harness).y0, 80.0, "Tab showed the panel");
    assert_eq!(calls.get(), 1);

    offset.set(0.0);
    harness.frame();
    assert_eq!(target(&mut harness).y0, 300.0, "the offset moved it away");

    let focused = harness.app().focused().expect("the panel holds the focus");
    assert!(harness.app().request_focus(focused));
    harness.frame();

    assert_eq!(target(&mut harness).y0, 80.0, "the request showed it again");
    assert_eq!(calls.get(), 2);
    assert_eq!(offset.read(|value| *value), 220.0);
    harness.assert_matches_full_repaint();
}

/// Builds the [`scroller`] with a reveal handler that stores the offset.
fn revealing(offset: RwSignal<f32>) -> Element<Panel> {
    let write = offset.clone();
    scroller(offset).on_reveal(move |revealed| write.set(revealed.y))
}

#[test]
fn a_non_finite_offset_on_an_ancestor_does_not_panic_a_reveal() {
    let offset = RwSignal::new(0.0);
    let mut harness = Harness::new(100.0, 100.0, 1.0, {
        let offset = offset.clone();
        move || {
            Element::new(Panel)
                .on_reveal(|_| {})
                .scroll_offset(|| Point {
                    x: 0.0,
                    y: f32::NAN,
                })
                .child(revealing(offset))
        }
    });
    harness.frame();

    harness.press_key(Key::Tab);
    harness.frame();

    assert!(
        harness.app().focused().is_some(),
        "the frame after the Tab ran to the end and the panel holds the focus"
    );
}

#[test]
fn a_press_on_the_node_a_pending_tab_focused_scrolls_nothing() {
    let offset = RwSignal::new(0.0);
    let mut harness = Harness::new(100.0, 100.0, 1.0, {
        let offset = offset.clone();
        move || {
            Element::new(Panel)
                .column()
                .height(px(100.0))
                .overflow(Overflow::Scroll)
                .on_reveal({
                    let write = offset.clone();
                    move |revealed| write.set(revealed.y)
                })
                .scroll_offset(move || Point {
                    x: 0.0,
                    y: offset.get(),
                })
                .child(panel().height(px(90.0)).shrink(0.0))
                .child(panel().height(px(20.0)).shrink(0.0).focusable())
        }
    });
    harness.frame();

    harness.press_key(Key::Tab);
    harness.click(Point { x: 50.0, y: 95.0 });
    harness.frame();

    assert!(harness.app().focused().is_some(), "the row holds the focus");
    assert_eq!(
        offset.read(|value| *value),
        0.0,
        "the press scrolled nothing"
    );
}

#[test]
fn an_absolutely_positioned_focus_does_not_scroll_its_parent() {
    let offset = RwSignal::new(0.0);
    let mut harness = Harness::new(100.0, 100.0, 1.0, {
        let offset = offset.clone();
        move || {
            Element::new(Panel)
                .column()
                .height(px(100.0))
                .overflow(Overflow::Scroll)
                .on_reveal({
                    let write = offset.clone();
                    move |revealed| write.set(revealed.y)
                })
                .scroll_offset(move || Point {
                    x: 0.0,
                    y: offset.get(),
                })
                .child(panel().height(px(300.0)).shrink(0.0))
                .child(
                    panel()
                        .absolute()
                        .inset(Edges {
                            top: px(150.0),
                            left: px(0.0),
                            right: Length::auto(),
                            bottom: Length::auto(),
                        })
                        .width(px(20.0))
                        .height(px(20.0))
                        .focusable(),
                )
        }
    });
    harness.frame();

    harness.press_key(Key::Tab);
    harness.frame();

    assert!(
        harness.app().focused().is_some(),
        "the panel took the focus"
    );
    assert_eq!(
        offset.read(|value| *value),
        0.0,
        "the view's scroll never moves an absolute child, so it stays put"
    );
}
