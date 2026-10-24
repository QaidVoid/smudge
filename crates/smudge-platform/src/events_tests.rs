//! Tests for the translation of windowing system events into input events.
//!
//! Most winit events cannot be constructed outside winit, so key and modifier
//! translation is tested on the pure functions, and the events winit does
//! allow constructing go through the translator itself.

use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key, Modifiers, PointerButton};
use winit::dpi::PhysicalPosition;
use winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

use super::{EventTranslator, key_name, translate_modifiers};

#[test]
fn cursor_position_at_scale_two_is_logical() {
    let mut translator = EventTranslator::new();
    let event = WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(20.0, 40.0),
    };

    let translated = translator.translate(&event, 2.0);

    assert_eq!(
        translated,
        Some(InputEvent::PointerMoved {
            position: Point { x: 10.0, y: 20.0 },
        })
    );
}

#[test]
fn a_line_wheel_delta_becomes_logical_pixels() {
    let mut translator = EventTranslator::new();
    let event = WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::LineDelta(0.0, 3.0),
        phase: TouchPhase::Moved,
    };

    let translated = translator.translate(&event, 1.0);

    assert_eq!(
        translated,
        Some(InputEvent::Wheel {
            delta: Point { x: 0.0, y: 120.0 },
        })
    );
}

#[test]
fn a_pixel_wheel_delta_loses_the_scale() {
    let mut translator = EventTranslator::new();
    let event = WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(40.0, -20.0)),
        phase: TouchPhase::Moved,
    };

    let translated = translator.translate(&event, 2.0);

    assert_eq!(
        translated,
        Some(InputEvent::Wheel {
            delta: Point { x: 20.0, y: -10.0 },
        })
    );
}

#[test]
fn a_button_press_carries_the_last_cursor_position() {
    let mut translator = EventTranslator::new();
    let moved = WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(10.0, 12.0),
    };
    translator.translate(&moved, 2.0);
    let pressed = WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button: MouseButton::Left,
    };

    let translated = translator.translate(&pressed, 2.0);

    assert_eq!(
        translated,
        Some(InputEvent::PointerDown {
            position: Point { x: 5.0, y: 6.0 },
            button: PointerButton::Primary,
        })
    );
}

#[test]
fn a_button_press_without_a_cursor_position_is_dropped() {
    let mut translator = EventTranslator::new();
    let pressed = WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button: MouseButton::Left,
    };

    assert_eq!(translator.translate(&pressed, 1.0), None);
}

#[test]
fn a_side_mouse_button_is_dropped() {
    let mut translator = EventTranslator::new();
    let moved = WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(1.0, 1.0),
    };
    translator.translate(&moved, 1.0);
    let pressed = WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button: MouseButton::Other(8),
    };

    assert_eq!(translator.translate(&pressed, 1.0), None);
}

#[test]
fn the_pointer_leaving_the_window_is_an_event() {
    let mut translator = EventTranslator::new();

    assert_eq!(
        translator.translate(
            &WindowEvent::CursorLeft {
                device_id: DeviceId::dummy()
            },
            1.0
        ),
        Some(InputEvent::PointerLeft)
    );
}

#[test]
fn window_focus_and_ime_commit_reach_the_app() {
    let mut translator = EventTranslator::new();

    assert_eq!(
        translator.translate(&WindowEvent::Focused(true), 1.0),
        Some(InputEvent::WindowFocus(true))
    );
    assert_eq!(
        translator.translate(
            &WindowEvent::Ime(winit::event::Ime::Commit(String::from("smudge"))),
            1.0,
        ),
        Some(InputEvent::ImeCommit {
            text: String::from("smudge"),
        })
    );
}

#[test]
fn keys_translate_by_meaning() {
    assert_eq!(
        key_name(&WinitKey::Character(String::from("x").into())),
        Key::Character(String::from("x"))
    );
    assert_eq!(key_name(&WinitKey::Named(NamedKey::Enter)), Key::Enter);
    assert_eq!(
        key_name(&WinitKey::Named(NamedKey::Backspace)),
        Key::Backspace
    );
    assert_eq!(key_name(&WinitKey::Named(NamedKey::PageUp)), Key::PageUp);
    assert_eq!(key_name(&WinitKey::Named(NamedKey::F5)), Key::Other);
    assert_eq!(key_name(&WinitKey::Dead(None)), Key::Other);
}

#[test]
fn modifiers_translate_bit_for_bit() {
    let state = ModifiersState::SHIFT | ModifiersState::CONTROL | ModifiersState::SUPER;

    assert_eq!(
        translate_modifiers(state),
        Modifiers {
            shift: true,
            control: true,
            alt: false,
            meta: true,
        }
    );
}

#[test]
fn a_modifiers_change_reaches_the_app_as_its_own_event() {
    let mut translator = EventTranslator::new();
    let event = WindowEvent::ModifiersChanged(winit::event::Modifiers::from(ModifiersState::SHIFT));

    assert_eq!(
        translator.translate(&event, 1.0),
        Some(InputEvent::ModifiersChanged(Modifiers {
            shift: true,
            ..Modifiers::default()
        }))
    );
}
