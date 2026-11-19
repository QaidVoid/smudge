//! The devtools overlay a window draws over its app, with the `_devtools`
//! feature, which the `devtools` feature of `smudge` turns on.
//!
//! The overlay is an [`App`] of its own, which builds the panel of
//! `smudge-devtools`. It runs on the window's thread with its own root
//! owner, so a remount of the app never touches it, and the two trees never
//! wake each other. The window sends each input event to one of the two, as
//! [`Devtools::dispatch`] decides, and draws the overlay's frame over the
//! app's without letting it reach the app's own frames or numbers.

use std::time::Duration;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::theme::Theme;
use smudge_core::window::Cursor;

/// The environment variable that opens the panel when the app starts, when
/// it is `1`.
pub(crate) const OPEN_VARIABLE: &str = "SMUDGE_DEVTOOLS";

/// The environment variable that names the key that shows and hides the
/// panel, such as `ctrl+shift+d`.
pub(crate) const KEY_VARIABLE: &str = "SMUDGE_DEVTOOLS_KEY";

/// The devtools of one window: the overlay's app, whether it shows, and
/// where input goes.
pub(crate) struct Devtools {
    /// The overlay's own app, which builds the devtools panel.
    pub(crate) app: App,
    /// Whether the panel shows.
    open: bool,
    /// Whether a primary press began over the panel, so the pointer stays
    /// with the overlay until the press ends.
    captured: bool,
    /// Whether a primary press began on the app, so the pointer stays with
    /// the app until the press ends, over the panel too.
    app_pressed: bool,
    /// Whether the pointer is over the panel, as the last move found.
    over: bool,
    /// Whether keys go to the overlay, after a press on the panel and until
    /// a press on the app.
    keys: bool,
    /// Whether the window holds the input focus of the operating system,
    /// which the side with the keys hears.
    window_focused: bool,
    /// The key that shows and hides the panel.
    toggle: Shortcut,
    /// How long the overlay's last presented frame took.
    pub(crate) timing: OverlayTiming,
}

/// The overlay's own time in one present, which the app's frame statistics
/// and present statistics leave out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OverlayTiming {
    /// How long the overlay's app took to run its frame.
    pub(crate) frame: Duration,
    /// How long drawing the overlay and compositing it over the app took.
    /// The GPU renderer draws both lists in one render, so there it is zero
    /// and the overlay's drawing counts in the frame's raster time.
    pub(crate) draw: Duration,
}

impl Devtools {
    /// Creates the devtools of a window whose app shows `viewport` logical
    /// pixels at `scale`, open when [`OPEN_VARIABLE`] is `1`, with the
    /// toggle key [`KEY_VARIABLE`] names.
    pub(crate) fn new(viewport: Size, scale: f32) -> Devtools {
        let open = std::env::var(OPEN_VARIABLE).is_ok_and(|value| value == "1");
        let toggle = std::env::var(KEY_VARIABLE)
            .ok()
            .and_then(|value| Shortcut::parse(&value))
            .unwrap_or_default();
        Devtools::with(viewport, scale, open, toggle)
    }

    /// Creates the devtools with the panel `open` or not and the `toggle`
    /// key, whatever the environment says.
    pub(crate) fn with(viewport: Size, scale: f32, open: bool, toggle: Shortcut) -> Devtools {
        let mut app = App::new(
            AppConfig {
                viewport,
                scale,
                clear_color: Color::TRANSPARENT,
                repaint: RepaintMode::Damage,
            },
            smudge_devtools::overlay,
        );
        app.set_theme(Theme::dark());
        // Keys start with the app, so the overlay starts unfocused.
        app.handle_event(InputEvent::WindowFocus(false));
        Devtools {
            app,
            open,
            captured: false,
            app_pressed: false,
            over: false,
            keys: false,
            window_focused: true,
            toggle,
            timing: OverlayTiming::default(),
        }
    }

    /// Returns whether the panel shows.
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Hands `event` to the overlay or to `app`, and returns whether it
    /// showed or hid the panel.
    ///
    /// A press of the toggle key, not a repeat, shows and hides the panel,
    /// and neither app hears the key. While the panel shows:
    /// - the pointer goes to the overlay while it is over the panel, and
    ///   stays with the side a primary press began on until the press ends;
    ///   the side the pointer leaves hears it leave;
    /// - the wheel goes where the pointer is;
    /// - keys and the input method go to the side of the last press, and
    ///   that side alone counts the window as focused, so the other shows no
    ///   caret or focus ring;
    /// - the modifiers reach both.
    ///
    /// While it is hidden, everything goes to the app.
    pub(crate) fn dispatch(&mut self, app: &mut App, event: InputEvent) -> bool {
        if let InputEvent::Key(key) = &event
            && self.toggle.matches(key)
        {
            if key.pressed && !key.repeat {
                self.show(app, !self.open);
                return true;
            }
            return false;
        }
        if !self.open {
            match &event {
                InputEvent::PointerDown {
                    button: PointerButton::Primary,
                    ..
                } => self.app_pressed = true,
                InputEvent::PointerUp {
                    button: PointerButton::Primary,
                    ..
                } => self.app_pressed = false,
                InputEvent::WindowFocus(focused) => self.window_focused = *focused,
                _ => {}
            }
            app.handle_event(event);
            return false;
        }
        match event {
            InputEvent::PointerMoved { position } => {
                let over = self.takes_pointer(position);
                if over != self.over {
                    if over {
                        app.handle_event(InputEvent::PointerLeft);
                    } else {
                        self.app.handle_event(InputEvent::PointerLeft);
                    }
                    self.over = over;
                }
                self.target(app, over).handle_event(event);
            }
            InputEvent::PointerLeft => {
                let over = self.over;
                self.target(app, over).handle_event(event);
                self.over = false;
            }
            InputEvent::PointerDown { position, button } => {
                let over = self.takes_pointer(position);
                if button == PointerButton::Primary {
                    self.captured |= over;
                    self.app_pressed |= !over;
                }
                self.focus_keys(app, over);
                self.target(app, over).handle_event(event);
            }
            InputEvent::PointerUp { button, .. } => {
                let over = self.captured || (!self.app_pressed && self.over);
                if button == PointerButton::Primary {
                    self.captured = false;
                    self.app_pressed = false;
                }
                self.target(app, over).handle_event(event);
            }
            InputEvent::Wheel { .. } => {
                let over = self.over;
                self.target(app, over).handle_event(event);
            }
            InputEvent::Key(_) | InputEvent::ImePreedit { .. } | InputEvent::ImeCommit { .. } => {
                let keys = self.keys;
                self.target(app, keys).handle_event(event);
            }
            InputEvent::WindowFocus(focused) => {
                self.window_focused = focused;
                let keys = self.keys;
                self.target(app, keys).handle_event(event);
                self.target(app, !keys)
                    .handle_event(InputEvent::WindowFocus(false));
            }
            InputEvent::ModifiersChanged(_) => {
                self.app.handle_event(event.clone());
                app.handle_event(event);
            }
        }
        false
    }

    /// Shows or hides the panel. A press held on the panel ends there, the
    /// pointer leaves it, and the keys go back to `app`.
    fn show(&mut self, app: &mut App, open: bool) {
        if !open {
            if self.captured {
                let position = self.app.pointer_position();
                self.app.handle_event(InputEvent::PointerUp {
                    position,
                    button: PointerButton::Primary,
                });
            }
            if self.over {
                self.app.handle_event(InputEvent::PointerLeft);
            }
            self.focus_keys(app, false);
        }
        self.open = open;
        self.captured = false;
        self.over = false;
    }

    /// Returns whether the pointer at `position` goes to the overlay: a press
    /// that began on it holds it, one that began on the app keeps it, and
    /// otherwise it goes where the panel is.
    fn takes_pointer(&self, position: Point) -> bool {
        !self.app_pressed && (self.captured || self.over_panel(position))
    }

    /// Sends the keys to the overlay when `overlay` holds, and to `app`
    /// otherwise, and tells the side that loses them that the window lost
    /// its focus, and the side that gains them whether the window has it.
    fn focus_keys(&mut self, app: &mut App, overlay: bool) {
        if overlay == self.keys {
            return;
        }
        self.keys = overlay;
        let focused = self.window_focused;
        self.target(app, !overlay)
            .handle_event(InputEvent::WindowFocus(false));
        self.target(app, overlay)
            .handle_event(InputEvent::WindowFocus(focused));
    }

    /// Returns the cursor the window shows: the overlay's while the pointer
    /// is over the panel, and `app`'s otherwise.
    pub(crate) fn cursor(&self, app: &App) -> Cursor {
        if self.open && self.over {
            self.app.cursor()
        } else {
            app.cursor()
        }
    }

    /// Returns the area the input method anchors to: the overlay's while
    /// the keys go to it, and `app`'s otherwise, so composed text reaches
    /// the side that hears the keys.
    pub(crate) fn ime_area(&self, app: &App) -> Option<Rect> {
        if self.open && self.keys {
            self.app.ime_area()
        } else {
            app.ime_area()
        }
    }

    /// Returns the overlay when `overlay` holds, and `app` otherwise.
    fn target<'a>(&'a mut self, app: &'a mut App, overlay: bool) -> &'a mut App {
        if overlay { &mut self.app } else { app }
    }

    /// Returns whether `position` lies over the panel: on a node of the
    /// overlay below its root, which covers the window and takes nothing.
    fn over_panel(&self, position: Point) -> bool {
        let mut over = false;
        self.app.visit_nodes(|depth, _, rect| {
            over |= depth > 0 && rect.contains(position);
        });
        over
    }
}

/// A key with the modifiers that must be held with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shortcut {
    /// The character the key types, in lower case.
    character: String,
    /// The modifiers held with it, exactly.
    modifiers: Modifiers,
}

impl Default for Shortcut {
    /// Control, Shift and D, or Command, Shift and D on macOS, since a
    /// browser takes F12 for its own devtools.
    fn default() -> Self {
        Shortcut {
            character: String::from("d"),
            modifiers: Modifiers {
                shift: true,
                control: !cfg!(target_os = "macos"),
                meta: cfg!(target_os = "macos"),
                alt: false,
            },
        }
    }
}

impl Shortcut {
    /// Reads a key such as `ctrl+shift+d`: modifiers named `ctrl` or
    /// `control`, `shift`, `alt` or `option`, and `cmd`, `meta` or `super`,
    /// then one character, joined by `+` in any case. Anything else names no
    /// key, and `None` is returned.
    pub(crate) fn parse(text: &str) -> Option<Shortcut> {
        let mut modifiers = Modifiers::default();
        let mut character = None;
        for part in text.split('+') {
            let part = part.trim().to_lowercase();
            match part.as_str() {
                "ctrl" | "control" => modifiers.control = true,
                "shift" => modifiers.shift = true,
                "alt" | "option" => modifiers.alt = true,
                "cmd" | "meta" | "super" => modifiers.meta = true,
                _ if character.is_none() && part.chars().count() == 1 => character = Some(part),
                _ => return None,
            }
        }
        Some(Shortcut {
            character: character?,
            modifiers,
        })
    }

    /// Returns whether `key` is this shortcut, pressed or released.
    fn matches(&self, key: &KeyEvent) -> bool {
        let Key::Character(character) = &key.key else {
            return false;
        };
        character.to_lowercase() == self.character && key.modifiers == self.modifiers
    }
}

#[cfg(test)]
#[path = "devtools_tests.rs"]
mod tests;
