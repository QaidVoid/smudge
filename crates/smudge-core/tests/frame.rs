//! Behavior tests for the frame pipeline.
//!
//! Every test drives the same `App::frame` a window does, so what it observes
//! is what a running application shows.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::{Element, show};
use smudge_core::geometry::{Color, Corners, PixelRect, Rect, Size};
use smudge_core::paint::{DisplayItem, PaintOp};
use smudge_core::reactive::{prelude::*, signal_local};
use smudge_core::style::px;
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};

/// The viewport every test lays its tree out for, at a scale of one.
const VIEWPORT: Size = Size {
    width: 200.0,
    height: 100.0,
};

/// A configuration for the test viewport.
fn config(repaint: RepaintMode) -> AppConfig {
    AppConfig {
        viewport: VIEWPORT,
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint,
    }
}

/// The frame's observable result, copied out so the app can be read afterwards.
struct Frame {
    /// Whether the frame repaints the whole surface.
    full: bool,
    /// The physical damage rectangles.
    damage: Vec<PixelRect>,
    /// The display list items in paint order.
    items: Vec<DisplayItem>,
}

/// Runs one frame and copies its output out of the app.
fn frame(app: &mut App) -> Frame {
    let output = app.frame();
    Frame {
        full: output.full,
        damage: output.damage.to_vec(),
        items: output.list.items().to_vec(),
    }
}

/// The damage rectangle covering the whole test surface.
fn full_surface() -> PixelRect {
    PixelRect {
        x: 0,
        y: 0,
        width: 200,
        height: 100,
    }
}

/// A filled rectangle operation.
fn fill(rect: Rect, color: Color) -> PaintOp {
    PaintOp::FillRect {
        rect,
        radius: Corners::ZERO,
        color,
    }
}

/// A root with a background and two smaller boxes, so every node paints one
/// rectangle.
fn scene() -> Element<Panel> {
    panel()
        .background(Color::rgb(200, 200, 200))
        .child(
            panel()
                .width(px(50.0))
                .height(px(20.0))
                .background(Color::BLACK),
        )
        .child(
            panel()
                .width(px(30.0))
                .height(px(10.0))
                .background(Color::WHITE),
        )
}

/// A widget that paints a fixed mark at its origin whatever its size, the way
/// text paints from the top left of its node.
struct Mark;

impl Widget for Mark {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        out.push(fill(
            Rect::new(cx.rect.x0, cx.rect.y0, cx.rect.x0 + 30.0, cx.rect.y0 + 10.0),
            Color::rgb(0, 0, 255),
        ));
    }
}

/// A widget that writes a signal from its bind function, which is one of the
/// two places user code runs while queued updates are applied.
struct Writer(Rc<dyn Fn(u32)>);

impl Widget for Writer {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// Builds an app whose two slots mount in turn and each one's build closure
/// flips the flag the other one reads, so the queue never settles.
fn cycling_app() -> App {
    let (flag, set_flag) = signal_local(true);
    App::new(config(RepaintMode::Damage), move || {
        panel()
            .child(show(
                move || flag.get(),
                move || {
                    set_flag.set(false);
                    panel()
                },
            ))
            .child(show(
                move || !flag.get(),
                move || {
                    set_flag.set(true);
                    panel()
                },
            ))
    })
}

#[test]
fn the_first_frame_is_full_and_lists_every_visible_box() {
    let mut app = App::new(config(RepaintMode::Damage), scene);

    let first = frame(&mut app);

    assert!(first.full, "the first frame repaints the whole surface");
    assert_eq!(first.damage, vec![full_surface()]);
    assert_eq!(
        first.items.len(),
        3,
        "the root and both boxes paint one operation each"
    );
    assert!(app.stats().layout_ran, "the first frame lays the tree out");
    assert_eq!(app.stats().nodes_painted, 3);
    assert_eq!(app.stats().damage_rects, 1);
    assert_eq!(app.stats().damage_pixels, 200 * 100);
}

#[test]
fn a_second_frame_with_no_changes_does_nothing() {
    let mut app = App::new(config(RepaintMode::Damage), scene);
    frame(&mut app);

    let second = frame(&mut app);

    assert!(!second.full);
    assert!(
        second.damage.is_empty(),
        "nothing changed, so nothing repaints, got {:?}",
        second.damage
    );
    assert!(!app.stats().layout_ran, "a clean tree does not relayout");
    assert_eq!(app.stats().nodes_painted, 0, "a clean tree paints nothing");
    assert_eq!(app.stats().damage_rects, 0);
    assert_eq!(app.stats().flush_rounds, 1, "one flush found no work");
}

#[test]
fn a_background_binding_repaints_one_box() {
    let (color, set_color) = signal_local(Color::BLACK);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        panel().background(Color::WHITE).child(
            panel()
                .width(px(50.0))
                .height(px(20.0))
                .background(move || color.get().into()),
        )
    });
    frame(&mut app);

    set_color.set(Color::rgb(255, 0, 0));
    let changed = frame(&mut app);

    assert_eq!(
        changed.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 50,
            height: 20,
        }],
        "the changed box repaints exactly its own rectangle"
    );
    assert_eq!(app.stats().nodes_painted, 1, "one box repaints");
    assert!(!app.stats().layout_ran, "a color change does not relayout");
}

#[test]
fn a_repaint_in_place_adds_its_rectangle_once() {
    let (color, set_color) = signal_local(Color::BLACK);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        panel().child(
            panel()
                .width(px(50.0))
                .height(px(20.0))
                .background(move || color.get().into()),
        )
    });
    frame(&mut app);

    set_color.set(Color::rgb(255, 0, 0));
    frame(&mut app);

    assert_eq!(
        app.stats().damage_pixels_before_merge,
        50 * 20,
        "the old and new bounds are the same rectangle, so the merge sees it once"
    );
}

#[test]
fn full_repaint_mode_damages_the_whole_surface_and_lists_the_same_items() {
    let mut damage_mode = App::new(config(RepaintMode::Damage), scene);
    let mut full_mode = App::new(config(RepaintMode::Full), scene);

    let damage_first = frame(&mut damage_mode);
    let full_first = frame(&mut full_mode);
    assert_eq!(
        full_first.items, damage_first.items,
        "the display list does not depend on the repaint mode"
    );

    let damage_second = frame(&mut damage_mode);
    let full_second = frame(&mut full_mode);

    assert!(
        damage_second.damage.is_empty(),
        "damage mode repaints nothing when nothing changed"
    );
    assert!(
        full_second.full,
        "full mode reports a full frame on every frame"
    );
    assert_eq!(full_second.damage, vec![full_surface()]);
    assert_eq!(
        full_second.items, damage_second.items,
        "both modes build the same list"
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "did not settle")]
fn a_binding_cycle_stops_at_the_round_cap() {
    cycling_app().frame();
}

#[cfg(not(debug_assertions))]
#[test]
fn a_binding_cycle_stops_at_the_round_cap() {
    let mut app = cycling_app();

    app.frame();

    assert_eq!(
        app.stats().flush_rounds,
        16,
        "the frame stops after the round cap instead of running forever"
    );
}

/// A release build stops at the round cap instead, as
/// `a_binding_cycle_stops_at_the_round_cap` shows for the mounting route.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "did not settle")]
fn a_bind_function_that_writes_its_own_signal_hits_the_round_cap() {
    let (value, set_value) = signal_local(0u32);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        let set = Rc::new(move |next: u32| set_value.set(next));
        panel().child(Element::new(Writer(set)).bind(
            move || value.get(),
            |writer, seen| {
                (writer.0)(seen.wrapping_add(1));
                Dirty::Paint
            },
        ))
    });

    app.frame();
}

/// A release build cannot test this case, because the frame never returns.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "part of a cycle")]
fn a_binding_that_writes_what_it_reads_panics_in_debug_builds() {
    let (shade, set_shade) = signal_local(0u8);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        panel().background(move || {
            let value = shade.get();
            set_shade.set(value.wrapping_add(1));
            Color::rgb(value, 0, 0).into()
        })
    });

    app.frame();
}

/// A release build cannot test this case, because the frame never returns.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "part of a cycle")]
fn two_bindings_that_write_each_others_signals_panic_in_debug_builds() {
    let (left, set_left) = signal_local(0u8);
    let (right, set_right) = signal_local(0u8);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        panel()
            .background(move || {
                let value = left.get();
                set_right.set(value.wrapping_add(1));
                Color::rgb(value, 0, 0).into()
            })
            .child(panel().background(move || {
                let value = right.get();
                set_left.set(value.wrapping_add(1));
                Color::rgb(0, value, 0).into()
            }))
    });

    app.frame();
}

#[test]
fn a_binding_that_corrects_what_it_reads_settles() {
    let (shade, set_shade) = signal_local(0u8);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        panel().background(move || {
            let value = shade.get();
            if value > 100 {
                set_shade.set(100);
            }
            Color::rgb(value, 0, 0).into()
        })
    });
    frame(&mut app);

    set_shade.set(250);
    let clamped = frame(&mut app);

    assert_eq!(
        clamped.items,
        vec![DisplayItem::Op(fill(
            Rect::new(0.0, 0.0, 200.0, 100.0),
            Color::rgb(100, 0, 0)
        ))],
        "a write that stops changing the value ends the binding's runs"
    );
}

#[test]
fn bindings_see_the_context_the_build_closure_provides() {
    #[derive(Clone)]
    struct Accent(Color);

    let mut app = App::new(config(RepaintMode::Damage), || {
        provide_context(Accent(Color::BLACK));
        panel().background(|| {
            use_context::<Accent>()
                .map_or(Color::WHITE, |accent| accent.0)
                .into()
        })
    });

    let first = frame(&mut app);

    assert_eq!(
        first.items,
        vec![DisplayItem::Op(fill(
            Rect::new(0.0, 0.0, 200.0, 100.0),
            Color::BLACK
        ))],
        "the binding reads the accent provided while the tree was built"
    );
}

#[test]
fn set_viewport_forces_a_full_frame_and_relayout() {
    let mut app = App::new(config(RepaintMode::Damage), scene);
    frame(&mut app);

    app.set_viewport(
        Size {
            width: 320.0,
            height: 240.0,
        },
        1.0,
    );
    let resized = frame(&mut app);

    assert!(resized.full, "a viewport change repaints everything");
    assert_eq!(
        resized.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        }]
    );
    assert!(app.stats().layout_ran, "the root is laid out again");
    assert_eq!(app.surface_size(), (320, 240));
}

#[test]
fn set_repaint_mode_forces_a_full_frame() {
    let mut app = App::new(config(RepaintMode::Damage), scene);
    frame(&mut app);

    app.set_repaint_mode(RepaintMode::Full);
    let changed = frame(&mut app);

    assert!(
        changed.full,
        "switching the mode repaints the whole surface"
    );
    assert_eq!(changed.damage, vec![full_surface()]);
}

#[test]
fn a_hidden_node_contributes_no_items() {
    let (hidden, set_hidden) = signal_local(false);
    let mut app = App::new(config(RepaintMode::Damage), move || {
        panel().background(Color::WHITE).child(
            Element::new(Mark)
                .width(px(30.0))
                .height(px(10.0))
                .hidden(move || hidden.get()),
        )
    });
    let visible = frame(&mut app);
    assert_eq!(
        visible.items.len(),
        2,
        "the mark paints while it is visible"
    );

    set_hidden.set(true);
    let hidden = frame(&mut app);

    assert_eq!(
        hidden.items,
        vec![DisplayItem::Op(fill(
            Rect::new(0.0, 0.0, 200.0, 100.0),
            Color::WHITE
        ))],
        "the hidden mark contributes no operation, even though its widget \
         would paint at the window origin"
    );
}

#[test]
fn a_clipping_parent_wraps_its_children_in_a_clip() {
    let mut app = App::new(config(RepaintMode::Damage), || {
        panel().background(Color::WHITE).child(
            panel()
                .width(px(80.0))
                .height(px(40.0))
                .clip(true)
                .background(Color::BLACK)
                .child(
                    panel()
                        .width(px(200.0))
                        .height(px(200.0))
                        .shrink(0.0)
                        .background(Color::WHITE),
                ),
        )
    });

    let first = frame(&mut app);

    assert_eq!(
        first.items,
        vec![
            DisplayItem::Op(fill(Rect::new(0.0, 0.0, 200.0, 100.0), Color::WHITE)),
            DisplayItem::Op(fill(Rect::new(0.0, 0.0, 80.0, 40.0), Color::BLACK)),
            DisplayItem::PushClip(Rect::new(0.0, 0.0, 80.0, 40.0)),
            DisplayItem::Op(fill(Rect::new(0.0, 0.0, 200.0, 200.0), Color::WHITE)),
            DisplayItem::PopClip,
        ],
        "the clipping parent puts a clip around its child's operation"
    );
}

#[test]
fn the_tick_callback_runs_once_per_frame() {
    let ticks = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&ticks);
    let mut app = App::new(config(RepaintMode::Damage), panel);
    assert!(!app.has_tick(), "no tick is set yet");

    app.set_tick(move |_now| counter.set(counter.get() + 1));
    assert!(app.has_tick());

    frame(&mut app);
    frame(&mut app);

    assert_eq!(ticks.get(), 2, "the tick runs at the start of each frame");
}

#[test]
fn a_scaled_viewport_reports_physical_pixels() {
    let mut app = App::new(config(RepaintMode::Damage), panel);
    app.set_viewport(
        Size {
            width: 100.0,
            height: 50.0,
        },
        2.0,
    );

    assert_eq!(app.surface_size(), (200, 100));

    let first = frame(&mut app);
    assert_eq!(
        first.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        }],
        "damage leaves the app in physical pixels"
    );
}
