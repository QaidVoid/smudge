//! Behavior tests for the headless harness.
//!
//! Every test drives the same `App::frame` a window does, through the harness
//! that adds a CPU renderer and the damage comparison.

use std::marker::PhantomData;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, PixelRect, Point, Size};
use smudge_core::input::{Key, Modifiers};
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
                .background(move || color.get()),
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
