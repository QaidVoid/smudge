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
///
/// More kinds of input may be added, so a match on it needs a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
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
    /// The pointer moved by this much while a node held the pointer lock, in
    /// the device's own units, which the platforms report unscaled and
    /// about a pixel each on most of them, not clamped to the window and
    /// not moving the pointer's position. Scale it to the sensitivity the
    /// view wants.
    PointerMotion {
        /// How far the pointer moved, positive to the right and down.
        delta: Point,
    },
    /// The mouse wheel or touchpad scrolled.
    ///
    /// The delta is in logical pixels, positive for scrolling right and down.
    /// While Shift is held, a delta with no horizontal part reaches the
    /// wheel handlers as a horizontal one, so a plain mouse wheel scrolls
    /// sideways.
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
    ///
    /// The focused node keeps the keyboard focus and hears no `on_focus`.
    /// The state reaches the application through
    /// [`window::focused`](crate::window::focused).
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
///
/// More buttons may be named, so a match on it needs a wildcard arm.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// The main button, the left button on a right-handed mouse.
    Primary,
    /// The context menu button, the right button on a right-handed mouse.
    Secondary,
    /// The middle button.
    Middle,
    /// The back button, which goes back in a history.
    Back,
    /// The forward button, which goes forward in a history.
    Forward,
    /// Another button, by the number the platform gives it.
    Other(
        /// The platform's number for the button.
        u16,
    ),
}

/// A pointer button pressed, moved while held or released, as a node hears
/// it through [`Element::on_button_down`](crate::element::Element::on_button_down)
/// and the handlers beside it.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ButtonEvent {
    /// The button.
    pub button: PointerButton,
    /// Where the pointer is, in logical pixels relative to the top left of
    /// the node's rectangle. It can lie outside the node while the button is
    /// held.
    pub position: Point,
    /// The keyboard modifiers held at the time.
    pub modifiers: Modifiers,
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
/// reports that has no variant here arrives as [`Key::Other`]. More keys may
/// be named, so a match on it needs a wildcard arm.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The context menu key, which opens the focused node's context menu.
    ContextMenu,
    /// A function key, F1 and up, by its number.
    Function(
        /// The key's number, 1 for F1.
        u8,
    ),
    /// A key with no variant of its own.
    Other,
}

/// One step of a drag that a node hears through
/// [`Element::on_drag`](crate::element::Element::on_drag).
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Drag {
    /// The pointer moved while the press is held, to this position relative
    /// to the top left of the node's rectangle. It can lie outside the node.
    Moved(Point),
    /// The press ended, released or taken away, so the drag is over.
    Ended,
    /// The pointer moved, and an observer inside this node took the move, so
    /// this one should stop whatever the drag had it doing.
    Taken,
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
