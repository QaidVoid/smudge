//! Commands an application sends to its window, and the window state it
//! reads.
//!
//! An application that draws its own title bar asks the window system to move,
//! resize, maximize or close the window: [`start_move`] and
//! [`start_resize`] from the pointer-down handler of the area that starts the
//! gesture, [`toggle_maximize`] from a maximize button, [`close`] from a
//! close button. The commands queue on the application the running code
//! belongs to, and the windowing layer takes them with
//! [`App::take_window_commands`](crate::App::take_window_commands) and applies
//! them to the window. A headless application has no windowing layer, so the
//! queue keeps the commands for a test to inspect.
//!
//! The windowing layer applies the commands after the input event whose
//! handler queued them, while that event is still being handled. An
//! interactive move or resize is only honored while the press that started it
//! is being reported, which is why a command belongs in the pointer-down
//! handler of the area that starts it, not in its release handler.
//!
//! [`maximized`] reports whether the window is maximized. It is a reactive
//! read: a binding that reads it reruns when the windowing layer reports a
//! new state through
//! [`App::set_window_state`](crate::App::set_window_state). [`focused`]
//! reports whether the window holds the operating system's input focus, and
//! reruns its readers when the windowing layer reports a
//! [`WindowFocus`](crate::input::InputEvent::WindowFocus) event.
//!
//! [`Cursor`] chooses the pointer's look over a node, set with the
//! [`cursor`](crate::element::Element::cursor) builder. The cursor shows
//! wherever the pointer is over the node, and setting one does not change
//! which node a press or hover lands on.
//!
//! Code that runs with no live application above it, such as an owner whose
//! application was dropped, queues nothing: the commands do nothing there,
//! [`maximized`] reads `false` and [`focused`] reads `true`.
//!
//! On the web target the windowing layer applies no commands and reports the
//! state as never maximized, so these functions queue and read without
//! effect. That rests on winit's web backend, which refuses the move and
//! resize calls and treats maximize as a no-op through winit 0.30; a winit
//! upgrade that changes this changes the behavior here.
//!
//! # Examples
//!
//! A title bar area that moves the window while it is pressed, and a binding
//! that shows a restore hint while the window is maximized:
//!
//! ```
//! use smudge_core::geometry::Color;
//! use smudge_core::style::px;
//! use smudge_core::widget::panel;
//! use smudge_core::window;
//!
//! let header = panel()
//!     .width(px(320.0))
//!     .height(px(36.0))
//!     .background(Color::rgb(24, 26, 32))
//!     .on_pointer_down(|_| window::start_move())
//!     .child(panel().height(px(8.0)).hidden(|| window::maximized()));
//! ```

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::reactive::{Owner, RwSignal, prelude::*, runtime};

/// The edge or corner of a window that an interactive resize pulls.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    /// The top edge.
    North,
    /// The bottom edge.
    South,
    /// The right edge.
    East,
    /// The left edge.
    West,
    /// The top right corner.
    NorthEast,
    /// The top left corner.
    NorthWest,
    /// The bottom right corner.
    SouthEast,
    /// The bottom left corner.
    SouthWest,
}

/// One request to change the window, as
/// [`App::take_window_commands`](crate::App::take_window_commands) hands it
/// to the windowing layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowCommand {
    /// Start an interactive move, as if the user grabbed the window's title
    /// bar with the press that is being handled.
    Move,
    /// Start an interactive resize from an edge or corner, as if the user
    /// grabbed that edge with the press that is being handled.
    Resize(ResizeEdge),
    /// Restore the window if it is maximized, or maximize it if it is not.
    ToggleMaximize,
    /// Close the window and end the run, as a close button does.
    Close,
}

/// The window state the windowing layer reports to the application.
///
/// More fields may follow, so a state is built from the default and the
/// `with_` methods rather than as a struct literal:
///
/// ```
/// use smudge_core::window::WindowState;
///
/// let state = WindowState::default().with_maximized(true);
/// assert!(state.maximized);
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct WindowState {
    /// Whether the window is maximized.
    pub maximized: bool,
}

impl WindowState {
    /// Returns this state with its maximized flag set to `maximized`.
    pub fn with_maximized(mut self, maximized: bool) -> WindowState {
        self.maximized = maximized;
        self
    }
}

/// The look of the pointer over a node.
///
/// The windowing layer shows the cursor of the deepest enabled node under the
/// pointer that sets one, and the platform default when none does. The resize
/// variants name the direction the resize would pull, so an edge handle
/// shows the same look the window system uses when it resizes that edge.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cursor {
    /// The platform default, usually an arrow.
    #[default]
    Default,
    /// A clickable target, usually a pointing hand.
    Pointer,
    /// A place where text can be selected or a caret put, usually an I
    /// beam.
    Text,
    /// A window edge or corner that resizes in that direction.
    Resize(ResizeEdge),
}

/// Queues a command to start an interactive move of the window.
///
/// Call it from the pointer-down handler of the area that starts the move,
/// because the window system honors the request only while the press that
/// started it is being handled. With no application above the running code
/// the call does nothing.
pub fn start_move() {
    queue(WindowCommand::Move);
}

/// Queues a command to start an interactive resize of the window from
/// `edge`.
///
/// Call it from the pointer-down handler of the handle that starts the
/// resize, because the window system honors the request only while the press
/// that started it is being handled. With no application above the running
/// code the call does nothing.
pub fn start_resize(edge: ResizeEdge) {
    queue(WindowCommand::Resize(edge));
}

/// Queues a command to maximize the window, or to restore it when it is
/// already maximized.
///
/// With no application above the running code the call does nothing.
pub fn toggle_maximize() {
    queue(WindowCommand::ToggleMaximize);
}

/// Queues a command to close the window and end the run.
///
/// With no application above the running code the call does nothing.
pub fn close() {
    queue(WindowCommand::Close);
}

/// Returns whether the window is maximized, as the windowing layer last
/// reported it.
///
/// This is a reactive read: a binding that calls it reruns when the state
/// changes, including a rerun or a memo recomputation that runs with no
/// owner current. A windowing layer that reports nothing, such as a headless
/// test, leaves it at `false`, and so does code with no live application
/// above it.
pub fn maximized() -> bool {
    let Some(book) = running_book() else {
        return false;
    };
    book.borrow().maximized.get()
}

/// Returns whether the window holds the operating system's input focus, as
/// the windowing layer last reported it.
///
/// This is a reactive read, like [`maximized`], so a binding or an effect
/// that calls it reruns when the window comes to the front or goes to the
/// background. The keyboard focus inside the window does not move with it,
/// so a node can hold the focus while the window is in the background. A
/// windowing layer that reports nothing, such as a headless test, leaves it
/// at `true`, and so does code with no live application above it.
///
/// # Examples
///
/// A banner that shows only while the window is in the background:
///
/// ```
/// use smudge_core::geometry::Color;
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_core::window;
///
/// let banner = panel()
///     .height(px(24.0))
///     .background(Color::rgb(64, 64, 72))
///     .hidden(|| window::focused());
/// ```
pub fn focused() -> bool {
    let Some(book) = running_book() else {
        return true;
    };
    book.borrow().focused.get()
}

/// Appends `command` to the command queue of the application the running
/// code belongs to, or drops it when no live application is above the code.
fn queue(command: WindowCommand) {
    if let Some(book) = running_book() {
        book.borrow_mut().commands.push(command);
    }
}

/// Returns the window book of the application the running code belongs to.
///
/// A running effect or memo names its application through the owner it was
/// created under, because its reruns run with no owner current. Other code,
/// such as an input handler, names it through the current owner.
fn running_book() -> Option<Rc<RefCell<WindowBook>>> {
    book_of(&runtime::running_owner()?)
}

/// Replaces the window state the application's [`maximized`] reads, but only
/// when it changed, so an equal report wakes no binding.
pub(crate) fn set_state(book: &Rc<RefCell<WindowBook>>, state: WindowState) {
    let maximized = book.borrow().maximized.clone();
    if maximized.get() != state.maximized {
        maximized.set(state.maximized);
    }
}

/// Records whether the window holds the input focus for the application's
/// [`focused`], but only when it changed, so an equal report wakes no
/// binding.
pub(crate) fn set_focused(book: &Rc<RefCell<WindowBook>>, focused: bool) {
    let signal = book.borrow().focused.clone();
    if signal.get() != focused {
        signal.set(focused);
    }
}

/// Takes every command queued so far, in the order they were queued.
pub(crate) fn take_commands(book: &Rc<RefCell<WindowBook>>) -> Vec<WindowCommand> {
    std::mem::take(&mut book.borrow_mut().commands)
}

/// Creates the book of a new application, whose [`maximized`] and
/// [`focused`] signals are created under the application's root owner so
/// they die with it.
pub(crate) fn new_book(root: &Owner) -> Rc<RefCell<WindowBook>> {
    let (maximized, focused) = root.with(|| (RwSignal::new(false), RwSignal::new(true)));
    Rc::new(RefCell::new(WindowBook {
        commands: Vec::new(),
        maximized,
        focused,
    }))
}

/// The queued commands and the window state of one application.
///
/// The application owns the book. Window commands reach it from any owner
/// under the application's root, and the windowing layer drains it and feeds
/// the state signal back.
pub(crate) struct WindowBook {
    /// The commands queued since the windowing layer last took them.
    commands: Vec<WindowCommand>,
    /// The maximized state the windowing layer last reported.
    maximized: RwSignal<bool>,
    /// Whether the window held the input focus when the windowing layer
    /// last reported it.
    focused: RwSignal<bool>,
}

/// Returns the window book of the application `owner` belongs to, walking up
/// the owner chain to the root that holds it.
fn book_of(owner: &Rc<runtime::OwnerData>) -> Option<Rc<RefCell<WindowBook>>> {
    let mut current = Rc::clone(owner);
    loop {
        let book = current
            .window_book
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade);
        if book.is_some() {
            return book;
        }
        let parent = current.parent.as_ref()?.upgrade()?;
        current = parent;
    }
}
