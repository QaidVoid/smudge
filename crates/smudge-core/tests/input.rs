//! Behavior tests for the input event model.
//!
//! Events are handled between frames, so the test injects them and reads the
//! state the app records for its routing.

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::widget::panel;

/// An app with a one-panel tree, at a scale of one.
fn app() -> App {
    App::new(
        AppConfig {
            viewport: Size {
                width: 200.0,
                height: 100.0,
            },
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        panel,
    )
}

#[test]
fn pointer_events_move_the_recorded_position() {
    let mut app = app();
    assert_eq!(app.pointer_position(), Point::default());

    let position = Point { x: 12.5, y: 30.0 };
    app.handle_event(InputEvent::PointerMoved { position });
    assert_eq!(app.pointer_position(), position);

    let release = Point { x: 40.0, y: 8.0 };
    app.handle_event(InputEvent::PointerUp {
        position: release,
        button: PointerButton::Primary,
    });
    assert_eq!(app.pointer_position(), release);
}

#[test]
fn key_events_move_the_recorded_modifiers() {
    let mut app = app();
    assert_eq!(app.modifiers(), Modifiers::default());

    let modifiers = Modifiers {
        shift: true,
        alt: true,
        ..Modifiers::default()
    };
    app.handle_event(InputEvent::Key(KeyEvent {
        key: Key::Character(String::from("A")),
        pressed: true,
        repeat: false,
        modifiers,
        text: Some(String::from("A")),
    }));
    assert_eq!(app.modifiers(), modifiers);

    app.handle_event(InputEvent::Key(KeyEvent {
        key: Key::Character(String::from("a")),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::default(),
        text: Some(String::from("a")),
    }));
    assert_eq!(app.modifiers(), Modifiers::default());
}

#[test]
fn an_app_frame_runs_after_an_event() {
    let mut app = app();
    app.frame();
    assert_eq!(app.stats().damage_rects, 1, "the first frame is full");
    app.frame();
    assert_eq!(app.stats().damage_rects, 0);

    app.handle_event(InputEvent::PointerMoved {
        position: Point { x: 5.0, y: 5.0 },
    });
    app.frame();
    assert_eq!(
        app.stats().damage_rects,
        0,
        "an event alone changes no pixels"
    );
}

#[test]
fn wheel_and_focus_events_change_nothing_recorded() {
    let mut app = app();
    let position = Point { x: 7.0, y: 9.0 };
    app.handle_event(InputEvent::PointerMoved { position });

    app.handle_event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: 120.0 },
    });
    app.handle_event(InputEvent::PointerLeft);
    app.handle_event(InputEvent::WindowFocus(false));
    app.handle_event(InputEvent::ImeCommit {
        text: String::from("smudge"),
    });

    assert_eq!(app.pointer_position(), position);
    assert_eq!(app.modifiers(), Modifiers::default());
}

#[test]
fn events_between_frames_do_not_disturb_the_frame() {
    let mut app = app();
    app.handle_event(InputEvent::PointerDown {
        position: Point { x: 3.0, y: 3.0 },
        button: PointerButton::Primary,
    });
    let first = app.frame();
    assert!(first.full, "the first frame repaints the whole surface");
    assert!(app.frame().damage.is_empty());
}

#[test]
fn a_modifiers_event_changes_the_recorded_modifiers() {
    let mut app = app();
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };

    app.handle_event(InputEvent::ModifiersChanged(shift));

    assert_eq!(
        app.modifiers(),
        shift,
        "a click after pressing Shift must see Shift held, even when the \
         platform reported Shift after its key event"
    );
}
