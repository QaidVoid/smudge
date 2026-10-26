//! Tests for the clipboard types and how an app's handlers reach them.

use std::cell::RefCell;
use std::rc::Rc;

use crate::app::{App, AppConfig, RepaintMode};
use crate::clipboard::{self, Clipboard, MemoryClipboard};
use crate::geometry::{Color, Size};
use crate::input::{Handled, Key, KeyEvent};
use crate::widget::panel;

/// Returns an app config for a small surface.
fn config() -> AppConfig {
    AppConfig {
        viewport: Size {
            width: 120.0,
            height: 60.0,
        },
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint: RepaintMode::Damage,
    }
}

/// Builds a panel whose key handler reads the clipboard on a `c` press and
/// stores the result in `seen`.
fn clipboard_reader(seen: Rc<RefCell<Option<Option<String>>>>) -> impl FnMut(&KeyEvent) -> Handled {
    move |event: &KeyEvent| {
        if event.pressed && event.key == Key::Character(String::from("c")) {
            *seen.borrow_mut() = clipboard::with(|clipboard| clipboard.get());
            Handled::Yes
        } else {
            Handled::No
        }
    }
}

/// The key event a reader responds to.
fn read_key() -> KeyEvent {
    KeyEvent {
        key: Key::Character(String::from("c")),
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
        text: Some(String::from("c")),
    }
}

/// Focuses the panel, which is the first focusable node, with a Tab press.
fn focus_panel(app: &mut App) {
    app.handle_event(crate::input::InputEvent::Key(KeyEvent {
        key: Key::Tab,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
        text: None,
    }));
}

/// The result the reader recorded.
fn recorded(result: &Rc<RefCell<Option<Option<String>>>>) -> Option<String> {
    result.borrow().clone().flatten()
}

#[test]
fn a_memory_clipboard_round_trips_one_text() {
    let mut clipboard = MemoryClipboard::default();
    assert_eq!(clipboard.get(), None, "a fresh clipboard holds no text");

    clipboard.set(String::from("first"));
    assert_eq!(clipboard.get().as_deref(), Some("first"));

    clipboard.set(String::from("second"));
    assert_eq!(clipboard.get().as_deref(), Some("second"), "a set replaces");
}

#[test]
fn clones_of_a_memory_clipboard_share_the_text() {
    let mut clipboard = MemoryClipboard::default();
    let mut clone = clipboard.clone();

    clipboard.set(String::from("shared"));
    assert_eq!(clone.get().as_deref(), Some("shared"));

    clone.set(String::from("back"));
    assert_eq!(clipboard.get().as_deref(), Some("back"));
}

#[test]
fn a_key_handler_reaches_the_clipboard_of_its_app() {
    let seen = Rc::new(RefCell::new(None));
    let reader = clipboard_reader(Rc::clone(&seen));
    let mut app = App::new(config(), move || panel().on_key(reader));
    app.set_clipboard(Box::new({
        let mut clipboard = MemoryClipboard::default();
        clipboard.set(String::from("seeded"));
        clipboard
    }));
    focus_panel(&mut app);

    app.handle_event(crate::input::InputEvent::Key(read_key()));

    assert_eq!(
        recorded(&seen).as_deref(),
        Some("seeded"),
        "the handler found the app's clipboard"
    );
}

#[test]
fn installing_again_replaces_the_previous_clipboard() {
    let seen = Rc::new(RefCell::new(None));
    let reader = clipboard_reader(Rc::clone(&seen));
    let mut app = App::new(config(), move || panel().on_key(reader));
    app.set_clipboard(Box::new({
        let mut clipboard = MemoryClipboard::default();
        clipboard.set(String::from("first"));
        clipboard
    }));
    focus_panel(&mut app);
    app.handle_event(crate::input::InputEvent::Key(read_key()));
    app.set_clipboard(Box::new({
        let mut clipboard = MemoryClipboard::default();
        clipboard.set(String::from("latest"));
        clipboard
    }));

    app.handle_event(crate::input::InputEvent::Key(read_key()));

    assert_eq!(
        recorded(&seen).as_deref(),
        Some("latest"),
        "the second clipboard is the one handlers reach"
    );
}

#[test]
fn without_an_installed_clipboard_a_handler_gets_none() {
    let seen = Rc::new(RefCell::new(None));
    let reader = clipboard_reader(Rc::clone(&seen));
    let mut app = App::new(config(), move || panel().on_key(reader));

    app.handle_event(crate::input::InputEvent::Key(read_key()));

    assert_eq!(
        recorded(&seen),
        None,
        "an app that installed no clipboard answers no clipboard access"
    );
}

#[test]
fn code_outside_any_app_gets_none() {
    assert_eq!(clipboard::with(|_: &mut dyn Clipboard| ()), None);
}

#[test]
fn a_nested_clipboard_access_gets_none_instead_of_panicking() {
    let seen = Rc::new(RefCell::new(None));
    let inner = Rc::clone(&seen);
    let mut app = App::new(config(), move || {
        panel().on_key(move |_| {
            *inner.borrow_mut() = clipboard::with(|_| clipboard::with(|_| ()).is_some());
            Handled::Yes
        })
    });
    app.set_clipboard(Box::new(MemoryClipboard::default()));
    focus_panel(&mut app);

    app.handle_event(crate::input::InputEvent::Key(read_key()));

    assert_eq!(
        *seen.borrow(),
        Some(false),
        "the outer access holds the clipboard and the inner one finds it busy"
    );
}
