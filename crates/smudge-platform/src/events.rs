//! Translation of windowing system events into Smudge input events.
//!
//! The winit event loop hands every window event to [`EventTranslator`],
//! which converts the events that map to an [`InputEvent`] and keeps the
//! state the conversion needs across events: the pointer position, which
//! button events do not carry, and the keyboard modifiers, which winit
//! reports separately from key events.

use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use winit::dpi::PhysicalPosition;
use winit::event::{
    ElementState, Ime, KeyEvent as WinitKeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

/// How many logical pixels one line of a line-based wheel delta scrolls.
const LINE_HEIGHT: f32 = 40.0;

/// Translates the window events of one window into input events.
pub(crate) struct EventTranslator {
    /// The modifiers of the last modifiers event.
    modifiers: ModifiersState,
    /// The pointer position of the last cursor move, in logical pixels.
    ///
    /// Button events carry no position of their own, so they reuse this one.
    position: Option<Point>,
}

impl EventTranslator {
    /// Creates a translator that reports no pointer position and no
    /// modifiers.
    pub(crate) fn new() -> EventTranslator {
        EventTranslator {
            modifiers: ModifiersState::empty(),
            position: None,
        }
    }

    /// Translates one window event into an input event.
    ///
    /// Returns `None` when the event maps to no input event, either because
    /// the platform handles it elsewhere, such as a resize, or because it
    /// carries no position yet: a button event before the first cursor move
    /// is dropped instead of guessed. `scale` is the number of physical
    /// pixels per logical pixel and must be positive.
    pub(crate) fn translate(&mut self, event: &WindowEvent, scale: f32) -> Option<InputEvent> {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let position = logical_point(*position, scale);
                self.position = Some(position);
                Some(InputEvent::PointerMoved { position })
            }
            WindowEvent::CursorLeft { .. } => Some(InputEvent::PointerLeft),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = pointer_button(*button)?;
                let position = self.position?;
                Some(match state {
                    ElementState::Pressed => InputEvent::PointerDown { position, button },
                    ElementState::Released => InputEvent::PointerUp { position, button },
                })
            }
            WindowEvent::MouseWheel { delta, .. } => Some(InputEvent::Wheel {
                delta: wheel_delta(*delta, scale),
            }),
            WindowEvent::KeyboardInput { event, .. } => Some(InputEvent::Key(key_event(
                event,
                translate_modifiers(self.modifiers),
            ))),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                Some(InputEvent::ModifiersChanged(translate_modifiers(
                    self.modifiers,
                )))
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => Some(InputEvent::ImePreedit {
                text: text.clone(),
                cursor: *cursor,
            }),
            WindowEvent::Ime(Ime::Commit(text)) => {
                Some(InputEvent::ImeCommit { text: text.clone() })
            }
            WindowEvent::Ime(Ime::Enabled | Ime::Disabled) => None,
            WindowEvent::Focused(focused) => Some(InputEvent::WindowFocus(*focused)),
            _ => None,
        }
    }
}

/// Converts a physical position to logical pixels for `scale`.
fn logical_point(position: PhysicalPosition<f64>, scale: f32) -> Point {
    Point {
        x: position.x as f32 / scale,
        y: position.y as f32 / scale,
    }
}

/// Converts a wheel delta to logical pixels for `scale`, positive for
/// scrolling right and down.
///
/// The window system's sign is the opposite: a positive delta moves the
/// content right and down, which scrolls towards its left and top. A line
/// delta is a number of text lines, which scrolls at a fixed height of
/// [`LINE_HEIGHT`] logical pixels per line. A pixel delta is already in
/// physical pixels and only loses the scale.
fn wheel_delta(delta: MouseScrollDelta, scale: f32) -> Point {
    let moved = match delta {
        MouseScrollDelta::LineDelta(x, y) => Point {
            x: x * LINE_HEIGHT,
            y: y * LINE_HEIGHT,
        },
        MouseScrollDelta::PixelDelta(position) => logical_point(position, scale),
    };
    Point {
        x: -moved.x,
        y: -moved.y,
    }
}

/// Converts a mouse button, or `None` for one with no variant of its own.
fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Other(_) | MouseButton::Back | MouseButton::Forward => None,
    }
}

/// Converts a logical winit key by its meaning.
fn key_name(key: &WinitKey) -> Key {
    match key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        WinitKey::Named(named) => match named {
            NamedKey::Enter => Key::Enter,
            NamedKey::Tab => Key::Tab,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::ArrowLeft => Key::ArrowLeft,
            NamedKey::ArrowRight => Key::ArrowRight,
            NamedKey::ArrowUp => Key::ArrowUp,
            NamedKey::ArrowDown => Key::ArrowDown,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Space => Key::Space,
            _ => Key::Other,
        },
        WinitKey::Dead(_) | WinitKey::Unidentified(_) => Key::Other,
    }
}

/// Converts a winit modifier state to the platform-independent one.
fn translate_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        shift: state.contains(ModifiersState::SHIFT),
        control: state.contains(ModifiersState::CONTROL),
        alt: state.contains(ModifiersState::ALT),
        meta: state.contains(ModifiersState::SUPER),
    }
}

/// Converts a winit key event, with the modifiers of the last modifiers
/// event, which winit reports separately.
fn key_event(event: &WinitKeyEvent, modifiers: Modifiers) -> KeyEvent {
    KeyEvent {
        key: key_name(&event.logical_key),
        pressed: event.state == ElementState::Pressed,
        repeat: event.repeat,
        modifiers,
        text: event.text.as_ref().map(|text| text.to_string()),
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
