//! Behavior tests for style variants.
//!
//! Every test drives the same frame pipeline a window does, through the
//! harness, and checks what the surface shows against a harness that mounts
//! the expected scene plainly.

use smudge_core::element::IntoElement;
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{InputEvent, PointerButton};
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::panel;
use smudge_test::Harness;

/// The background a button rests in.
const IDLE: Color = Color::rgb(50, 50, 50);

/// The background the hovered patch asks for.
const HOVER: Color = Color::rgb(90, 90, 90);

/// The background the pressed patch asks for.
const PRESSED: Color = Color::rgb(30, 30, 30);

/// The background the disabled patch asks for.
const SWITCHED_OFF: Color = Color::rgb(35, 35, 35);

/// Wraps a node under test in a plain parent.
///
/// The layout pass sizes the root to the whole viewport, whatever its style
/// says, so a node with a size of its own has to sit under the root.
fn scene(child: impl IntoElement) -> impl IntoElement {
    panel().child(child)
}

/// A harness that shows `background` on a plainly mounted button of the size
/// the tests use, for pixel comparisons.
fn button_in_color(background: Color) -> Harness {
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(background),
        )
    });
    harness.frame();
    harness
}

/// Moves the pointer off every control and lets the change settle.
///
/// The pointer starts at the origin, and the first layout pass hovers a
/// control that sits there, so a test that asks for a specific hover change
/// parks the pointer on neutral ground first.
fn park_pointer(harness: &mut Harness) {
    harness.move_to(Point { x: 80.0, y: 50.0 });
    harness.frame();
}

#[test]
fn hovering_a_button_repaints_its_background_without_layout() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(IDLE)
                .hover(|p| p.background(HOVER)),
        )
    });
    assert!(harness.frame().full, "the first frame repaints everything");
    park_pointer(&mut harness);

    harness.move_to(Point { x: 10.0, y: 10.0 });
    let frame = harness.frame();
    assert_eq!(frame.nodes_painted, 1, "the hovered button repaints");
    assert!(
        !frame.layout_ran,
        "a patch that touches only paint skips layout"
    );
    assert!(!frame.full);

    harness.assert_matches(&button_in_color(HOVER));
}

#[test]
fn a_hover_patch_that_changes_the_border_width_triggers_layout() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .border_color(Color::BLACK)
                .hover(|p| p.border(2.0)),
        )
    });
    harness.frame();
    park_pointer(&mut harness);

    harness.move_to(Point { x: 10.0, y: 10.0 });
    let frame = harness.frame();
    assert!(
        frame.layout_ran,
        "a border width takes space, so the node is laid out again"
    );
    assert_eq!(frame.nodes_painted, 1);

    let mut expected = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .border(2.0)
                .border_color(Color::BLACK),
        )
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn a_background_binding_change_while_hovered_keeps_the_hover_background() {
    let (color, set_color) = reactive::signal_local(IDLE);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(move || color.get().into())
                .hover(|p| p.background(HOVER)),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.frame();
    harness.assert_matches(&button_in_color(HOVER));

    set_color.set(Color::rgb(140, 60, 60));
    let frame = harness.frame();
    assert_eq!(
        frame.nodes_painted, 0,
        "the patch wins over the base style, so nothing repaints"
    );
    harness.assert_matches(&button_in_color(HOVER));

    harness.move_to(Point { x: 80.0, y: 50.0 });
    let frame = harness.frame();
    assert_eq!(
        frame.nodes_painted, 1,
        "when the hover ends the binding's color shows"
    );
    harness.assert_matches(&button_in_color(Color::rgb(140, 60, 60)));
}

#[test]
fn a_hover_style_reaches_a_node_without_handlers_while_an_ancestor_has_one() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .background(IDLE)
                .hover(|p| p.background(Color::rgb(70, 70, 70)))
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .background(IDLE)
                        .hover(|p| p.background(HOVER)),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.frame();

    let mut expected = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .background(Color::rgb(70, 70, 70))
                .child(panel().width(px(20.0)).height(px(20.0)).background(HOVER)),
        )
    });
    expected.frame();
    harness.assert_matches(&expected);
}

/// A harness that shows a plainly mounted knob at `left`, for pixel
/// comparisons with the moving-knob scene. `None` leaves the knob
/// unpainted, as the scene's knob is when it is not hovered.
fn knob_at(left: f32, background: Option<Color>) -> Harness {
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let mut knob = panel()
            .absolute()
            .width(px(20.0))
            .height(px(20.0))
            .inset(Edges {
                left: Length::px(left),
                right: Length::auto(),
                top: Length::auto(),
                bottom: Length::auto(),
            });
        if let Some(color) = background {
            knob = knob.background(color);
        }
        scene(panel().width(px(60.0)).height(px(40.0)).child(knob))
    });
    harness.frame();
    harness
}

#[test]
fn a_node_moved_under_a_still_pointer_becomes_hovered() {
    let (left, set_left) = reactive::signal_local(30.0f32);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel().width(px(60.0)).height(px(40.0)).child(
                panel()
                    .absolute()
                    .width(px(20.0))
                    .height(px(20.0))
                    .inset(move || Edges {
                        left: Length::px(left.get()),
                        right: Length::auto(),
                        top: Length::auto(),
                        bottom: Length::auto(),
                    })
                    .hover(|p| p.background(HOVER)),
            ),
        )
    });
    harness.frame();
    harness.move_to(Point { x: 55.0, y: 10.0 });
    harness.frame();
    harness.assert_matches(&knob_at(30.0, None));

    set_left.set(45.0);
    let frame = harness.frame();
    assert_eq!(
        frame.nodes_painted, 1,
        "the knob moved and no pointer event announced it"
    );
    harness.assert_matches(&knob_at(45.0, Some(HOVER)));
}

#[test]
fn a_hovered_node_that_becomes_disabled_paints_its_base_style_again() {
    let (disabled, set_disabled) = reactive::signal_local(false);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(IDLE)
                .hover(|p| p.background(HOVER))
                .disabled(move || disabled.get()),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.frame();
    harness.assert_matches(&button_in_color(HOVER));

    set_disabled.set(true);
    let frame = harness.frame();
    assert!(
        frame.nodes_painted >= 1,
        "the hovered node is switched off, so its hover style goes"
    );
    harness.assert_matches(&button_in_color(IDLE));

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.frame();
    harness.assert_matches(&button_in_color(IDLE));
}

#[test]
fn a_pressed_patch_applies_while_the_press_is_held() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(IDLE)
                .pressed(|p| p.background(PRESSED)),
        )
    });
    harness.frame();

    harness.event(InputEvent::PointerDown {
        position: Point { x: 10.0, y: 10.0 },
        button: PointerButton::Primary,
    });
    harness.frame();
    harness.assert_matches(&button_in_color(PRESSED));

    harness.event(InputEvent::PointerUp {
        position: Point { x: 10.0, y: 10.0 },
        button: PointerButton::Primary,
    });
    harness.frame();
    harness.assert_matches(&button_in_color(IDLE));
}

#[test]
fn a_disabled_patch_applies_while_the_node_is_disabled() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(IDLE)
                .disabled(true)
                .disabled_style(|p| p.background(SWITCHED_OFF)),
        )
    });
    harness.frame();
    harness.assert_matches(&button_in_color(SWITCHED_OFF));

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.frame();
    // A disabled node is never hovered, so its patch stands alone.
    harness.assert_matches(&button_in_color(SWITCHED_OFF));
}

#[test]
fn a_focus_style_draws_for_keyboard_focus_alone_and_drops_with_a_disabled_node() {
    let (disabled, set_disabled) = reactive::signal_local(false);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(IDLE)
                .focusable()
                .focused(|p| p.border(1.0).border_color(Color::rgb(0, 120, 215)))
                .disabled(move || disabled.get()),
        )
    });
    harness.frame();

    // A focus that a press moved draws no focus style.
    harness.click(Point { x: 10.0, y: 10.0 });
    harness.frame();
    harness.assert_matches(&button_in_color(IDLE));

    // Tab moves the focus by keyboard, and the ring appears.
    harness.press_key(smudge_core::input::Key::Tab);
    let frame = harness.frame();
    assert!(
        frame.layout_ran,
        "the ring's border takes space, so the node is laid out again"
    );
    let mut focused = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(IDLE)
                .border(1.0)
                .border_color(Color::rgb(0, 120, 215)),
        )
    });
    focused.frame();
    harness.assert_matches(&focused);

    // Switching the focused node off drops the focus, and the ring with it.
    set_disabled.set(true);
    harness.frame();
    assert!(harness.app().focused().is_none());
    harness.assert_matches(&button_in_color(IDLE));
}

#[test]
fn a_hover_style_that_resizes_a_node_moved_under_a_still_pointer_applies_in_one_frame() {
    let (grow, set_grow) = reactive::signal_local(false);
    let mut harness = Harness::new(200.0, 60.0, 1.0, move || {
        panel().child(
            panel()
                .row()
                .child(
                    panel()
                        .width(move || if grow.get() { px(50.0) } else { px(0.0) })
                        .height(px(20.0)),
                )
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .background(Color::BLACK)
                        .hover(|p| p.width(px(70.0))),
                ),
        )
    });
    harness.frame();
    harness.move_to(Point { x: 60.0, y: 10.0 });
    harness.frame();

    set_grow.set(true);
    harness.frame();

    let mut widths = Vec::new();
    harness.app().visit_nodes(|depth, _name, rect| {
        if depth == 2 {
            widths.push(rect.width());
        }
    });
    assert_eq!(
        widths,
        [50.0, 70.0],
        "the node moved under the pointer has its hovered width after one frame"
    );
}

#[test]
fn an_event_that_changes_a_variant_asks_for_a_frame_and_a_plain_move_does_not() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK)
                .hover(|p| p.background(Color::WHITE)),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 80.0, y: 50.0 });
    assert!(
        !harness.app().needs_frame(),
        "a move over nothing changes nothing"
    );

    harness.move_to(Point { x: 10.0, y: 10.0 });
    assert!(
        harness.app().needs_frame(),
        "hovering a styled node needs a frame"
    );

    harness.frame();
    assert!(!harness.app().needs_frame(), "the frame finished the work");
}

#[test]
fn a_layout_change_after_the_pointer_left_hovers_nothing() {
    let (grow, set_grow) = reactive::signal_local(false);
    let (log, on_hover) = {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let handler = {
            let log = std::rc::Rc::clone(&log);
            move |hovered: bool| log.borrow_mut().push(hovered)
        };
        (log, handler)
    };
    let mut harness = Harness::new(200.0, 60.0, 1.0, move || {
        panel().child(
            panel()
                .row()
                .child(
                    panel()
                        .width(move || if grow.get() { px(50.0) } else { px(0.0) })
                        .height(px(20.0)),
                )
                .child(panel().width(px(40.0)).height(px(20.0)).on_hover(on_hover)),
        )
    });
    harness.frame();
    harness.move_to(Point { x: 60.0, y: 10.0 });
    harness.event(InputEvent::PointerLeft);

    set_grow.set(true);
    harness.frame();

    assert!(
        log.borrow().is_empty(),
        "no pointer is in the window to hover with"
    );
}
