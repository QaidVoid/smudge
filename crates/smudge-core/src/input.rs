//! The platform-independent input events an application receives.
//!
//! [`App::handle_event`](crate::app::App::handle_event) takes these events
//! between frames. The windowing layer translates the events of the operating
//! system into them, and tests inject them directly, so widget and application
//! code never sees a windowing library. Positions are logical pixels, with the
//! origin at the top left of the window.

use crate::geometry::Point;

/// One input event, translated from the windowing system or injected by a
/// test.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// The pointer moved to a new position in logical pixels.
    PointerMoved {
        /// The position of the pointer in logical pixels.
        position: Point,
    },
    /// The pointer left the window.
    PointerLeft,
    /// A pointer button was pressed.
    PointerDown {
        /// The position of the pointer in logical pixels.
        position: Point,
        /// The button that was pressed.
        button: PointerButton,
    },
    /// A pointer button was released.
    PointerUp {
        /// The position of the pointer in logical pixels.
        position: Point,
        /// The button that was released.
        button: PointerButton,
    },
    /// The mouse wheel or touchpad scrolled.
    ///
    /// The delta is in logical pixels, positive for scrolling right and down.
    Wheel {
        /// How far the content scrolls, in logical pixels.
        delta: Point,
    },
    /// A keyboard key was pressed or released.
    Key(KeyEvent),
    /// The input method changed its composing text.
    ///
    /// `cursor` is the byte range of the cursor or selection inside `text`,
    /// or `None` when no cursor is shown.
    ImePreedit {
        /// The composing text.
        text: String,
        /// The byte range of the cursor inside the composing text.
        cursor: Option<(usize, usize)>,
    },
    /// The input method committed text, which the editor inserts.
    ImeCommit {
        /// The committed text.
        text: String,
    },
    /// The window gained or lost the input focus of the operating system.
    WindowFocus(
        /// Whether the window is focused.
        bool,
    ),
    /// The keyboard modifiers changed.
    ///
    /// Some platforms report a change only after the key event that caused
    /// it, so the modifiers a key event carries can lag behind. This event
    /// keeps the current state for whatever reads it next, such as a click.
    ModifiersChanged(
        /// The modifiers held now.
        Modifiers,
    ),
}

/// One of the pointer buttons an event can refer to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// The main button, the left button on a right-handed mouse.
    Primary,
    /// The context menu button, the right button on a right-handed mouse.
    Secondary,
    /// The middle button.
    Middle,
}

/// A keyboard key that was pressed or released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// Which key the event refers to.
    pub key: Key,
    /// Whether the key is being pressed, as opposed to released.
    pub pressed: bool,
    /// Whether this event repeats a key the user holds down.
    pub repeat: bool,
    /// The keyboard modifiers held when the event happened.
    pub modifiers: Modifiers,
    /// The text the keypress produces, when it produces any.
    pub text: Option<String>,
}

/// A key on the keyboard, by meaning rather than by position.
///
/// A printable key arrives as [`Key::Character`]. Everything the platform
/// reports that has no variant here arrives as [`Key::Other`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character key, already affected by the keyboard layout, such as `a`
    /// or `?`.
    Character(String),
    /// The Enter or Return key.
    Enter,
    /// The Tab key.
    Tab,
    /// The Escape key.
    Escape,
    /// The Backspace key.
    Backspace,
    /// The Delete key.
    Delete,
    /// The left arrow key.
    ArrowLeft,
    /// The right arrow key.
    ArrowRight,
    /// The up arrow key.
    ArrowUp,
    /// The down arrow key.
    ArrowDown,
    /// The Home key.
    Home,
    /// The End key.
    End,
    /// The Page Up key.
    PageUp,
    /// The Page Down key.
    PageDown,
    /// The space bar.
    Space,
    /// A key with no variant of its own.
    Other,
}

/// Whether a key event was taken by a handler.
///
/// A key handler returns this to decide whether the event keeps bubbling to
/// the ancestors of the node it reached.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Handled {
    /// A handler acted on the event, so it stops bubbling.
    Yes,
    /// The handler ignored the event, so it bubbles to the parent.
    No,
}

/// The keyboard modifiers held during an input event.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Whether Shift is held.
    pub shift: bool,
    /// Whether Control is held.
    pub control: bool,
    /// Whether Alt is held.
    pub alt: bool,
    /// Whether the Meta key is held, the Command key on macOS and the Super
    /// key elsewhere.
    pub meta: bool,
}

impl Modifiers {
    /// Returns whether the platform's command modifier is held.
    ///
    /// This is `meta` on macOS and `control` everywhere else, so a shortcut
    /// reads the same on every platform.
    ///
    /// # Examples
    ///
    /// ```
    /// use smudge_core::input::Modifiers;
    ///
    /// let modifiers = Modifiers {
    ///     control: true,
    ///     ..Modifiers::default()
    /// };
    /// #[cfg(not(target_os = "macos"))]
    /// assert!(modifiers.command());
    /// ```
    pub fn command(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.meta
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.control
        }
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
