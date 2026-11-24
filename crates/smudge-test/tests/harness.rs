//! Behavior tests for the headless harness.
//!
//! Every test drives the same `App::frame` a window does, through the harness
//! that adds a CPU renderer and the damage comparison.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, PixelRect, Point, Size};
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{prelude::*, signal_local};
use smudge_core::style::px;
use smudge_core::widget::{PaintCx, Widget, panel};
use smudge_test::Harness;

/// A widget with a type parameter, which paints nothing.
struct Tagged<T>(PhantomData<T>);

impl<T: 'static> Widget for Tagged<T> {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

#[test]
fn a_bound_box_repaints_its_own_rectangle() {
    let (color, set_color) = signal_local(Color::BLACK);
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().background(Color::WHITE).child(
            panel()
                .width(px(50.0))
                .height(px(20.0))
                .background(move || color.get().into()),
        )
    });

    let first = harness.frame();
    assert!(first.full, "the first frame repaints the whole surface");

    set_color.set(Color::rgb(255, 0, 0));
    let changed = harness.frame();

    assert_eq!(changed.nodes_painted, 1, "one box repaints");
    assert!(!changed.layout_ran, "a color change does not relayout");
    assert_eq!(
        changed.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 50,
            height: 20,
        }],
        "the changed box damages exactly its own rectangle"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn layout_dump_of_a_nested_scene_matches_the_snapshot() {
    let mut harness = Harness::new(120.0, 80.0, 1.0, || {
        panel().padding(px(8.0)).background(Color::WHITE).child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .child(
                    panel()
                        .width(px(10.0))
                        .height(px(10.0))
                        .background(Color::rgb(0, 0, 255)),
                ),
        )
    });
    harness.frame();

    insta::assert_snapshot!(harness.layout_dump());
}

#[test]
fn layout_dump_names_a_generic_widget_without_its_arguments() {
    let mut harness = Harness::new(40.0, 20.0, 1.0, || {
        panel().child(
            Element::new(Tagged::<Vec<String>>(PhantomData))
                .width(px(10.0))
                .height(px(10.0)),
        )
    });
    harness.frame();

    assert_eq!(
        harness.layout_dump(),
        "Panel [0, 0, 40, 20]\n  Tagged [0, 0, 10, 10]\n"
    );
}

#[test]
fn a_viewport_change_resizes_the_surface_and_repaints_it() {
    let mut harness = Harness::new(100.0, 50.0, 1.0, || {
        panel().background(Color::rgb(10, 20, 30)).child(
            panel()
                .width(px(30.0))
                .height(px(10.0))
                .background(Color::BLACK),
        )
    });
    harness.frame();

    harness.app().set_viewport(
        Size {
            width: 160.0,
            height: 90.0,
        },
        1.5,
    );
    let resized = harness.frame();

    assert!(resized.full, "a viewport change repaints the whole surface");
    assert_eq!(
        resized.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 240,
            height: 135,
        }]
    );
    harness.assert_matches_full_repaint();
}

#[test]
#[should_panic(expected = "call Harness::frame first")]
fn comparing_before_any_frame_panics() {
    let harness = Harness::new(10.0, 10.0, 1.0, panel);
    harness.assert_matches_full_repaint();
}

#[test]
fn an_injected_click_delivers_the_pointer_and_the_keys() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, panel);
    harness.frame();

    let position = Point { x: 30.0, y: 20.0 };
    harness.click(position);
    assert_eq!(harness.app().pointer_position(), position);

    harness.press_key(Key::Tab);
    harness.type_text("sm");
    assert_eq!(
        harness.app().modifiers(),
        Modifiers::default(),
        "typed keys carry no modifiers"
    );

    let frame = harness.frame();
    assert_eq!(frame.damage.len(), 0, "input alone repaints nothing");
}

#[test]
fn a_drop_reaches_the_deepest_zone_and_not_its_ancestors() {
    let outer = Rc::new(RefCell::new(Vec::new()));
    let inner = Rc::new(RefCell::new(Vec::new()));
    let (outer_heard, inner_heard) = (Rc::clone(&outer), Rc::clone(&inner));
    let mut harness = Harness::new(120.0, 80.0, 1.0, move || {
        panel()
            .on_file_drop({
                let heard = outer_heard.clone();
                move |paths, _| heard.borrow_mut().push(paths.to_vec())
            })
            // A spacer first, so the zone sits in the second row slot and
            // its own origin differs from the window's.
            .child(panel().width(px(40.0)).height(px(30.0)))
            .child(panel().width(px(40.0)).height(px(30.0)).on_file_drop({
                let heard = inner_heard.clone();
                move |paths, at| heard.borrow_mut().push((paths.to_vec(), at))
            }))
    });
    harness.frame();

    harness.drop_files(
        vec![PathBuf::from("/tmp/report.pdf")],
        Point { x: 50.0, y: 10.0 },
    );

    let heard = inner.borrow();
    assert_eq!(heard.len(), 1, "the deepest zone hears the drop once");
    assert_eq!(
        heard[0],
        (
            vec![PathBuf::from("/tmp/report.pdf")],
            Point { x: 10.0, y: 10.0 }
        ),
        "the zone hears the paths and the position inside it"
    );
    assert!(
        outer.borrow().is_empty(),
        "the zone around it hears nothing"
    );
}

#[test]
fn a_hover_tells_a_zone_when_the_files_enter_and_leave_it() {
    let top = Rc::new(RefCell::new(Vec::new()));
    let bottom = Rc::new(RefCell::new(Vec::new()));
    let (top_heard, bottom_heard) = (Rc::clone(&top), Rc::clone(&bottom));
    // The two zones sit side by side in the root's row: the first spans
    // [0, 0, 40, 30] and the second [40, 0, 40, 30].
    let mut harness = Harness::new(120.0, 80.0, 1.0, move || {
        panel()
            .child(panel().width(px(40.0)).height(px(30.0)).on_file_hover({
                let heard = top_heard.clone();
                move |paths| heard.borrow_mut().push(paths.map(|p| p.to_vec()))
            }))
            .child(panel().width(px(40.0)).height(px(30.0)).on_file_hover({
                let heard = bottom_heard.clone();
                move |paths| heard.borrow_mut().push(paths.map(|p| p.to_vec()))
            }))
    });
    harness.frame();
    let files = || vec![PathBuf::from("/tmp/a.txt")];
    let heard = |log: &Rc<RefCell<Vec<Option<Vec<PathBuf>>>>>| log.borrow().last().cloned();

    harness.hover_files(files(), Point { x: 10.0, y: 10.0 });
    assert_eq!(
        heard(&top),
        Some(Some(vec![PathBuf::from("/tmp/a.txt")])),
        "the files enter the zone under them"
    );
    assert_eq!(heard(&bottom), None, "the other zone hears nothing yet");

    // A report over the zone that already hears them reaches it again, so
    // a list of files that grows arrives.
    harness.hover_files(files(), Point { x: 20.0, y: 10.0 });
    assert_eq!(top.borrow().len(), 2, "the zone hears every report");

    harness.hover_files(files(), Point { x: 50.0, y: 10.0 });
    assert_eq!(heard(&top), Some(None), "the files left the first zone");
    assert_eq!(
        heard(&bottom),
        Some(Some(vec![PathBuf::from("/tmp/a.txt")])),
        "the second zone hears them"
    );

    harness.hover_files(files(), Point { x: 100.0, y: 70.0 });
    assert_eq!(
        heard(&bottom),
        Some(None),
        "a hover that leaves every zone clears the zone it last hovered"
    );
}

#[test]
fn a_cancelled_hover_clears_the_zone() {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&heard);
    let mut harness = Harness::new(120.0, 80.0, 1.0, move || {
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(30.0))
                .on_file_hover(move |paths| recorder.borrow_mut().push(paths.map(|p| p.to_vec()))),
        )
    });
    harness.frame();

    harness.hover_files(
        vec![PathBuf::from("/tmp/a.txt")],
        Point { x: 10.0, y: 10.0 },
    );
    harness.event(InputEvent::FileHoverCancelled);

    assert_eq!(
        heard.borrow().last(),
        Some(&None),
        "a cancelled hover clears the zone that heard it"
    );
}

#[test]
fn a_disabled_zone_hears_nothing() {
    let outer_hover = Rc::new(RefCell::new(Vec::new()));
    let outer_drop = Rc::new(RefCell::new(Vec::new()));
    let inner_hover = Rc::new(RefCell::new(Vec::new()));
    let inner_drop = Rc::new(RefCell::new(Vec::new()));
    let (outer_hover_heard, outer_drop_heard, inner_hover_heard, inner_drop_heard) = (
        Rc::clone(&outer_hover),
        Rc::clone(&outer_drop),
        Rc::clone(&inner_hover),
        Rc::clone(&inner_drop),
    );
    let mut harness = Harness::new(120.0, 80.0, 1.0, move || {
        panel()
            .on_file_drop({
                let heard = outer_drop_heard.clone();
                move |paths, _| heard.borrow_mut().push(paths.to_vec())
            })
            .on_file_hover({
                let heard = outer_hover_heard.clone();
                move |paths| heard.borrow_mut().push(paths.map(|p| p.to_vec()))
            })
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(30.0))
                    .disabled(|| true)
                    .on_file_drop({
                        let heard = inner_drop_heard.clone();
                        move |paths, _| heard.borrow_mut().push(paths.to_vec())
                    })
                    .on_file_hover({
                        let heard = inner_hover_heard.clone();
                        move |paths| heard.borrow_mut().push(paths.map(|p| p.to_vec()))
                    }),
            )
    });
    harness.frame();

    harness.hover_files(
        vec![PathBuf::from("/tmp/a.txt")],
        Point { x: 10.0, y: 10.0 },
    );
    harness.drop_files(
        vec![PathBuf::from("/tmp/a.txt")],
        Point { x: 10.0, y: 10.0 },
    );

    assert!(
        inner_hover.borrow().is_empty(),
        "a disabled zone hears no hover"
    );
    assert!(
        inner_drop.borrow().is_empty(),
        "a disabled zone hears no drop"
    );
    assert_eq!(
        outer_hover.borrow().as_slice(),
        [Some(vec![PathBuf::from("/tmp/a.txt")]), None],
        "the enabled zone around it hears the hover and its clear"
    );
    assert_eq!(
        outer_drop.borrow().as_slice(),
        [vec![PathBuf::from("/tmp/a.txt")]],
        "the enabled zone around it hears the drop"
    );
}

#[test]
fn a_drop_clears_the_hovered_zone_and_delivers_to_its_own() {
    let left = Rc::new(RefCell::new(Vec::new()));
    let right = Rc::new(RefCell::new(Vec::new()));
    let (left_heard, right_heard) = (Rc::clone(&left), Rc::clone(&right));
    // Two zones side by side: the left one only hears a hover, the right
    // one hears a hover and a drop. Each logs the order it hears things in.
    let mut harness = Harness::new(120.0, 80.0, 1.0, move || {
        panel()
            .child(panel().width(px(40.0)).height(px(30.0)).on_file_hover({
                let heard = left_heard.clone();
                move |paths| {
                    heard
                        .borrow_mut()
                        .push(if paths.is_some() { "hover" } else { "clear" })
                }
            }))
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(30.0))
                    .on_file_hover({
                        let heard = right_heard.clone();
                        move |paths| {
                            heard
                                .borrow_mut()
                                .push(if paths.is_some() { "hover" } else { "clear" })
                        }
                    })
                    .on_file_drop({
                        let heard = right_heard.clone();
                        move |paths, _| {
                            heard.borrow_mut().push(if paths.is_empty() {
                                "empty drop"
                            } else {
                                "drop"
                            })
                        }
                    }),
            )
    });
    harness.frame();

    harness.hover_files(
        vec![PathBuf::from("/tmp/a.txt")],
        Point { x: 10.0, y: 10.0 },
    );
    harness.hover_files(
        vec![PathBuf::from("/tmp/a.txt")],
        Point { x: 50.0, y: 10.0 },
    );
    harness.drop_files(
        vec![PathBuf::from("/tmp/b.txt")],
        Point { x: 50.0, y: 10.0 },
    );

    assert_eq!(
        left.borrow().as_slice(),
        ["hover", "clear"],
        "the hover moving to the other zone clears the first"
    );
    assert_eq!(
        right.borrow().as_slice(),
        ["hover", "clear", "drop"],
        "the zone hears the hover's clear before the drop"
    );
}
