//! Translation of windowing system events into Smudge input events.
//!
//! The winit event loop hands every window event to [`EventTranslator`],
//! which converts the events that map to an [`InputEvent`] and keeps the
//! state the conversion needs across events: the pointer position, which
//! button events do not carry, and the keyboard modifiers, which winit
//! reports separately from key events.

use std::path::Path;
use std::path::PathBuf;

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
    /// So do the file events, which the platforms report without a position
    /// of their own.
    position: Option<Point>,
    /// The per-file events of one drag gesture, gathered so far.
    ///
    /// The platforms report a drag one file at a time, so consecutive
    /// per-file events gather into one input event, which the event of
    /// another kind that interrupts the run or the end of the event batch
    /// reports through [`EventTranslator::flush_files`].
    files: Option<GatheredFiles>,
    /// Whether the app heard the hover of the drag still in the air.
    ///
    /// The platforms report a cancellation of a drag whose hover never
    /// reached the app, and hearing a cancellation for a hover the app
    /// never saw would leave it unbalanced, so only a reported hover
    /// arms the cancellation.
    hovering: bool,
}

/// The per-file window events gathered into one input event.
struct GatheredFiles {
    /// Whether the files are a drop, as opposed to a hover.
    dropped: bool,
    /// The gathered paths, in the order the window reported them.
    paths: Vec<PathBuf>,
}

impl EventTranslator {
    /// Creates a translator that reports no pointer position and no
    /// modifiers.
    pub(crate) fn new() -> EventTranslator {
        EventTranslator {
            modifiers: ModifiersState::empty(),
            position: None,
            files: None,
            hovering: false,
        }
    }

    /// Translates one window event into the input events it is.
    ///
    /// Most events translate into at most one. The per-file events of a
    /// drag, which the platforms report one file at a time, gather until an
    /// event of another kind or the end of the event batch reports what
    /// gathered, so an interrupted run reports before the event that
    /// interrupts it reaches the app. `scale` is the number of physical
    /// pixels per logical pixel and must be positive.
    pub(crate) fn window_event(&mut self, event: &WindowEvent, scale: f32) -> Vec<InputEvent> {
        match event {
            WindowEvent::HoveredFile(path) => return self.gather_file(path, false),
            WindowEvent::DroppedFile(path) => return self.gather_file(path, true),
            // A drag that never reported files reaches no app, and neither
            // does its cancellation, while one whose hover was reported
            // hears this cancellation.
            WindowEvent::HoveredFileCancelled => {
                let reported = self.hovering;
                self.hovering = false;
                self.files = None;
                return if reported {
                    vec![InputEvent::FileHoverCancelled]
                } else {
                    Vec::new()
                };
            }
            _ => {}
        }
        let mut reported = Vec::new();
        if let Some(gathered) = self.files.take() {
            reported.push(self.file_event(gathered));
        }
        if let Some(input) = self.translate(event, scale) {
            reported.push(input);
        }
        reported
    }

    /// Reports the files gathered since the last event, when there are any.
    ///
    /// The event loop calls this once an event batch ends, which is when a
    /// run of per-file events with nothing between them is complete.
    pub(crate) fn flush_files(&mut self) -> Option<InputEvent> {
        let gathered = self.files.take()?;
        Some(self.file_event(gathered))
    }

    /// Gathers one per-file event into the run it belongs to.
    ///
    /// A file of the other kind ends the run before it: a hover that a drop
    /// interrupts is reported first, and the drop gathers from scratch.
    fn gather_file(&mut self, path: &Path, dropped: bool) -> Vec<InputEvent> {
        match self.files.take() {
            Some(mut gathered) if gathered.dropped == dropped => {
                gathered.paths.push(path.to_owned());
                self.files = Some(gathered);
                Vec::new()
            }
            gathered => {
                let mut reported = Vec::new();
                if let Some(gathered) = gathered {
                    reported.push(self.file_event(gathered));
                }
                self.files = Some(GatheredFiles {
                    dropped,
                    paths: vec![path.to_owned()],
                });
                reported
            }
        }
    }

    /// Builds the input event the gathered files are.
    ///
    /// The platforms report no position with a file drag, so the files sit
    /// at the last position a cursor move reported, which is where the
    /// pointer last entered the window. A window that never saw a cursor
    /// move reports them at the origin.
    fn file_event(&mut self, gathered: GatheredFiles) -> InputEvent {
        let position = self.position.unwrap_or_default();
        self.hovering = !gathered.dropped;
        if gathered.dropped {
            InputEvent::FileDropped {
                paths: gathered.paths,
                position,
            }
        } else {
            InputEvent::FileHovered {
                paths: gathered.paths,
                position,
            }
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
                let button = pointer_button(*button);
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
            // Enabled and disabled carry nothing to act on: an enabled input
            // method with no preedit changes nothing, and a disabled one has
            // already lost the composition, which the focused editor drops
            // when it loses the focus and the app drops when the window
            // loses the input focus.
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

/// Converts a mouse button.
fn pointer_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Back,
        MouseButton::Forward => PointerButton::Forward,
        MouseButton::Other(number) => PointerButton::Other(number),
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
            NamedKey::ContextMenu => Key::ContextMenu,
            NamedKey::F1 => Key::Function(1),
            NamedKey::F2 => Key::Function(2),
            NamedKey::F3 => Key::Function(3),
            NamedKey::F4 => Key::Function(4),
            NamedKey::F5 => Key::Function(5),
            NamedKey::F6 => Key::Function(6),
            NamedKey::F7 => Key::Function(7),
            NamedKey::F8 => Key::Function(8),
            NamedKey::F9 => Key::Function(9),
            NamedKey::F10 => Key::Function(10),
            NamedKey::F11 => Key::Function(11),
            NamedKey::F12 => Key::Function(12),
            NamedKey::F13 => Key::Function(13),
            NamedKey::F14 => Key::Function(14),
            NamedKey::F15 => Key::Function(15),
            NamedKey::F16 => Key::Function(16),
            NamedKey::F17 => Key::Function(17),
            NamedKey::F18 => Key::Function(18),
            NamedKey::F19 => Key::Function(19),
            NamedKey::F20 => Key::Function(20),
            NamedKey::F21 => Key::Function(21),
            NamedKey::F22 => Key::Function(22),
            NamedKey::F23 => Key::Function(23),
            NamedKey::F24 => Key::Function(24),
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
