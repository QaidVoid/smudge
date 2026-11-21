//! Checkboxes, switches, sliders and number fields, by pointer and by
//! keyboard.

use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;
use smudge_widgets::{checkbox, number_field, slider, switch};

/// Returns the first projected node with `role`.
fn with_role(harness: &mut Harness, role: Role) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == role)
        .unwrap_or_else(|| panic!("the window shows a {role:?}"))
}

/// Returns the middle of `node`.
fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: (node.bounds.x0 + node.bounds.x1) / 2.0,
        y: (node.bounds.y0 + node.bounds.y1) / 2.0,
    }
}

/// A window of 300 by 200 showing `build`'s element at its top left.
fn window<E: smudge_core::element::IntoElement + 'static>(
    build: impl Fn() -> E + 'static,
) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().align_items(Align::Start).child(build())
    });
    harness.frame();
    harness
}

#[test]
fn a_checkbox_and_a_switch_toggle_by_press_and_by_space() {
    for role in [Role::CheckBox, Role::Switch] {
        let on = RwSignal::new(false);
        let bound = on.clone();
        let mut harness = window(move || {
            if role == Role::CheckBox {
                checkbox("Grid", bound.clone())
            } else {
                switch("Grid", bound.clone())
            }
        });
        let node = with_role(&mut harness, role);
        assert_eq!(node.label.as_deref(), Some("Grid"));
        assert_eq!(node.checked, Some(Checked::False));

        harness.click(middle(&node));
        harness.frame();
        assert!(on.get(), "{role:?} toggles on a press");
        assert_eq!(with_role(&mut harness, role).checked, Some(Checked::True));

        harness.press_key(Key::Space);
        harness.frame();
        assert!(!on.get(), "{role:?} toggles on Space");
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn a_slider_follows_the_pointer_in_steps_and_the_keys() {
    let value = RwSignal::new(0.5);
    let bound = value.clone();
    let mut harness = window(move || slider(bound.clone(), 0.0..=1.0, 0.1).width(px(204.0)));
    let node = with_role(&mut harness, Role::Slider);
    assert_eq!(node.range.map(|range| range.value), Some(0.5));

    // The track starts after the focus ring and half the thumb, 10 pixels
    // in, and runs 180 pixels.
    let y = middle(&node).y;
    harness.click(Point {
        x: 10.0 + 0.24 * 180.0,
        y,
    });
    harness.frame();
    assert_eq!(value.get(), 0.2, "on the nearest step");

    harness.button_down(Point { x: 20.0, y });
    harness.move_to(Point { x: 500.0, y });
    harness.button_up(Point { x: 500.0, y });
    harness.frame();
    assert_eq!(value.get(), 1.0, "a drag past the end stops there");

    for (key, expected) in [
        (Key::ArrowLeft, 0.9),
        (Key::ArrowDown, 0.8),
        (Key::ArrowUp, 0.9),
        (Key::Home, 0.0),
        (Key::PageUp, 1.0),
        (Key::End, 1.0),
    ] {
        harness.press_key(key.clone());
        assert_eq!(value.get(), expected, "after {key:?}");
    }
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::Slider)
            .range
            .map(|range| range.value),
        Some(1.0)
    );
    harness.assert_matches_full_repaint();
}

/// Replaces the text of the focused field with `text`.
fn retype(harness: &mut Harness, text: &str) {
    harness.press_key_with(
        Key::Character(String::from("a")),
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    harness.type_text(text);
}

#[test]
fn a_number_field_reads_typed_text_on_enter_and_when_the_focus_leaves() {
    let value = RwSignal::new(12.0);
    let bound = value.clone();
    let mut harness = window(move || {
        smudge_widgets::column()
            .child(number_field("Width", bound.clone(), 1.0..=100.0, 1.0).width(px(120.0)))
            .child(checkbox("Other", RwSignal::new(false)))
    });
    let field = with_role(&mut harness, Role::SpinButton);
    assert_eq!(field.value.as_deref(), Some("12"));
    harness.click(middle(&field));

    retype(&mut harness, "40");
    assert_eq!(value.get(), 12.0, "typing alone commits nothing");
    harness.press_key(Key::Enter);
    assert_eq!(value.get(), 40.0);

    retype(&mut harness, "500");
    harness.press_key(Key::Tab);
    harness.frame();
    assert_eq!(value.get(), 100.0, "leaving the field clamps what it read");
    assert_eq!(
        with_role(&mut harness, Role::SpinButton).value.as_deref(),
        Some("100")
    );

    harness.press_key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    retype(&mut harness, "abc");
    harness.press_key(Key::Enter);
    harness.frame();
    assert_eq!(value.get(), 100.0);
    assert_eq!(
        with_role(&mut harness, Role::SpinButton).value.as_deref(),
        Some("100"),
        "text that is no number goes back to the number"
    );
}

#[test]
fn a_number_field_steps_with_the_arrows_the_wheel_and_its_buttons() {
    let value = RwSignal::new(5.0);
    let bound = value.clone();
    let mut harness =
        window(move || number_field("Size", bound.clone(), 0.0..=10.0, 0.5).width(px(120.0)));
    let field = with_role(&mut harness, Role::SpinButton);
    assert_eq!(field.value.as_deref(), Some("5.0"));

    harness.move_to(middle(&field));
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: -40.0 },
    });
    assert_eq!(
        value.get(),
        5.0,
        "the wheel leaves a field without focus alone"
    );

    harness.click(middle(&field));
    harness.press_key(Key::ArrowUp);
    harness.press_key(Key::ArrowUp);
    assert_eq!(value.get(), 6.0);
    harness.press_key(Key::PageDown);
    assert_eq!(value.get(), 1.0);
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: -40.0 },
    });
    assert_eq!(value.get(), 1.5, "the wheel steps a focused field");

    // The buttons sit in the 18 pixel column at the right of the field's
    // 120 pixel box, which wraps the text 5 pixels above and below, the up
    // button in the top half.
    let (top, bottom) = (field.bounds.y0 - 5.0, field.bounds.y1 + 5.0);
    harness.click(Point {
        x: 110.0,
        y: top + (bottom - top) / 4.0,
    });
    assert_eq!(value.get(), 2.0, "the up button steps");
    harness.click(Point {
        x: 110.0,
        y: bottom - (bottom - top) / 4.0,
    });
    assert_eq!(value.get(), 1.5, "the down button steps");

    value.set(9.0);
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::SpinButton).value.as_deref(),
        Some("9.0"),
        "an outside write shows in the text"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_number_field_keeps_what_is_typed_across_frames_and_leaves_an_unedited_value_alone() {
    let value = RwSignal::new(0.25);
    let bound = value.clone();
    let mut harness = window(move || {
        smudge_widgets::column()
            .child(number_field("Size", bound.clone(), 0.0..=10.0, 0.1).width(px(120.0)))
            .child(checkbox("Other", RwSignal::new(false)))
    });
    harness.settle();
    let field = with_role(&mut harness, Role::SpinButton);
    assert_eq!(
        field.label.as_deref(),
        Some("Size"),
        "its name reaches a screen reader"
    );
    assert_eq!(field.value.as_deref(), Some("0.2"));

    harness.click(middle(&field));
    harness.press_key(Key::Tab);
    harness.settle();
    assert_eq!(
        value.get(),
        0.25,
        "passing through without an edit changes nothing"
    );

    harness.press_key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    retype(&mut harness, "");
    for typed in ["4", "4.", "4.5"] {
        harness.type_text(&typed[typed.len() - 1..]);
        harness.frame();
        assert_eq!(
            with_role(&mut harness, Role::SpinButton).value.as_deref(),
            Some(typed),
            "the text holds what was typed"
        );
    }
    harness.press_key(Key::Enter);
    assert_eq!(value.get(), 4.5);
}

#[test]
fn a_slider_keeps_to_its_steps_and_range_whatever_it_is_given() {
    let value = RwSignal::new(5.0);
    let bound = value.clone();
    let mut harness = window(move || slider(bound.clone(), 0.0..=11.0, -3.0).width(px(204.0)));
    harness.press_key(Key::Tab);
    harness.press_key(Key::End);
    assert_eq!(
        value.get(),
        11.0,
        "End reaches the end whatever the step divides"
    );
    harness.press_key(Key::ArrowLeft);
    assert_eq!(value.get(), 6.0, "a negative step steps down on Left");

    let open = RwSignal::new(f64::NAN);
    let bound = open.clone();
    let mut harness = window(move || {
        slider(bound.clone(), f64::NEG_INFINITY..=f64::INFINITY, 0.5).width(px(204.0))
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowUp);
    assert_eq!(open.get(), 0.5, "an unbounded range steps from zero");
    harness.press_key(Key::End);
    assert_eq!(open.get(), 0.5, "and has no end to go to");
    harness.frame();

    let broken = RwSignal::new(3.0);
    let bound = broken.clone();
    let mut harness = window(move || slider(bound.clone(), f64::NAN..=4.0, 1.0).width(px(204.0)));
    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowUp);
    assert_eq!(broken.get(), 4.0, "a NaN start bounds nothing");
    harness.frame();
}

#[test]
fn a_screen_reader_steps_a_slider_and_a_number_field() {
    let (level, width) = (RwSignal::new(0.5), RwSignal::new(10.0));
    let (bound_level, bound_width) = (level.clone(), width.clone());
    let mut harness = window(move || {
        smudge_widgets::column()
            .child(slider(bound_level.clone(), 0.0..=1.0, 0.1).width(px(204.0)))
            .child(number_field("Width", bound_width.clone(), 0.0..=20.0, 2.0).width(px(120.0)))
    });
    let slider_id = with_role(&mut harness, Role::Slider).id;
    let field_id = with_role(&mut harness, Role::SpinButton).id;

    assert!(harness.app().step_value(slider_id, true));
    assert_eq!(level.get(), 0.6);
    assert!(harness.app().step_value(field_id, false));
    assert_eq!(width.get(), 8.0);
}

#[test]
fn the_wheel_steps_a_number_field_once_per_line_it_turns() {
    let value = RwSignal::new(5.0);
    let bound = value.clone();
    let mut harness =
        window(move || number_field("Size", bound.clone(), 0.0..=10.0, 1.0).width(px(120.0)));
    let field = with_role(&mut harness, Role::SpinButton);
    harness.click(middle(&field));
    for _ in 0..4 {
        harness.event(InputEvent::Wheel {
            delta: Point { x: 0.0, y: -10.0 },
        });
    }
    assert_eq!(value.get(), 6.0, "four small turns make one line");
}
