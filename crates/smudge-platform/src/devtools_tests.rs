use std::cell::Cell;
use std::rc::Rc;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::Element;
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::input::{Handled, InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::pct;
use smudge_core::widget::Panel;
use smudge_text::TextSystem;

use super::{Devtools, Shortcut};

/// The window both apps fill, in logical pixels.
const VIEWPORT: Size = Size {
    width: 800.0,
    height: 600.0,
};

/// A point on the panel's title bar, where it opens.
const ON_PANEL: Point = Point { x: 40.0, y: 26.0 };

/// A point on the panel's body, which takes no presses and moves nothing.
const ON_BODY: Point = Point { x: 40.0, y: 120.0 };

/// A point well away from the panel.
const OFF_PANEL: Point = Point { x: 700.0, y: 500.0 };

/// What the app under the overlay heard.
#[derive(Default)]
struct Heard {
    /// Its presses.
    presses: Cell<u32>,
    /// Its keys.
    keys: Cell<u32>,
}

/// An app that fills the window and counts the presses and keys it hears,
/// and the devtools over it, open or not, both laid out.
fn apps(open: bool) -> (App, Devtools, Rc<Heard>) {
    TextSystem::bundled_only().install();
    let heard = Rc::new(Heard::default());
    let (pressed, keyed) = (Rc::clone(&heard), Rc::clone(&heard));
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || {
            Element::new(Panel)
                .width(pct(100.0))
                .height(pct(100.0))
                .focusable()
                .on_pointer_down(move |_| pressed.presses.set(pressed.presses.get() + 1))
                .on_key(move |_| {
                    keyed.keys.set(keyed.keys.get() + 1);
                    Handled::Yes
                })
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, open, Shortcut::default());
    devtools.app.frame();
    (app, devtools, heard)
}

/// A press and a release of the primary button at `position`.
fn click(devtools: &mut Devtools, app: &mut App, position: Point) {
    devtools.dispatch(app, InputEvent::PointerMoved { position });
    devtools.dispatch(
        app,
        InputEvent::PointerDown {
            position,
            button: PointerButton::Primary,
        },
    );
    devtools.dispatch(
        app,
        InputEvent::PointerUp {
            position,
            button: PointerButton::Primary,
        },
    );
}

/// A press of `key` typing `character` with `modifiers`.
fn key(character: &str, modifiers: Modifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        key: Key::Character(character.to_owned()),
        pressed: true,
        repeat: false,
        modifiers,
        text: None,
    })
}

#[test]
fn the_toggle_key_shows_and_hides_the_panel_and_reaches_neither_app() {
    let (mut app, mut devtools, heard) = apps(false);
    let toggle = Shortcut::default().modifiers;
    assert!(devtools.dispatch(&mut app, key("D", toggle)));
    assert!(devtools.is_open());
    assert!(devtools.dispatch(&mut app, key("d", toggle)));
    assert!(!devtools.is_open());
    assert_eq!(heard.keys.get(), 0, "the app never hears the toggle key");
    assert!(!devtools.dispatch(&mut app, key("d", Modifiers::default())));
    assert_eq!(heard.keys.get(), 1, "any other key reaches the app");
}

#[test]
fn a_press_over_the_panel_does_not_reach_the_app() {
    let (mut app, mut devtools, heard) = apps(true);
    click(&mut devtools, &mut app, ON_PANEL);
    assert_eq!(heard.presses.get(), 0);
    click(&mut devtools, &mut app, OFF_PANEL);
    assert_eq!(
        heard.presses.get(),
        1,
        "a press beside the panel reaches the app"
    );
}

#[test]
fn a_press_that_began_on_the_panel_keeps_the_pointer_until_it_ends() {
    let (mut app, mut devtools, heard) = apps(true);
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_BODY });
    devtools.dispatch(
        &mut app,
        InputEvent::PointerDown {
            position: ON_BODY,
            button: PointerButton::Primary,
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerMoved {
            position: OFF_PANEL,
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerUp {
            position: OFF_PANEL,
            button: PointerButton::Primary,
        },
    );
    devtools.app.frame();
    assert_eq!(heard.presses.get(), 0);
    click(&mut devtools, &mut app, OFF_PANEL);
    assert_eq!(
        heard.presses.get(),
        1,
        "after the release the app has the pointer again"
    );
}

#[test]
fn keys_follow_the_last_press() {
    let (mut app, mut devtools, heard) = apps(true);
    click(&mut devtools, &mut app, OFF_PANEL);
    devtools.dispatch(&mut app, key("a", Modifiers::default()));
    assert_eq!(
        heard.keys.get(),
        1,
        "after a press on the app, keys reach it"
    );
    click(&mut devtools, &mut app, ON_PANEL);
    devtools.dispatch(&mut app, key("a", Modifiers::default()));
    assert_eq!(
        heard.keys.get(),
        1,
        "after a press on the panel, keys reach the overlay"
    );
}

#[test]
fn a_hidden_panel_takes_nothing() {
    let (mut app, mut devtools, heard) = apps(false);
    click(&mut devtools, &mut app, ON_PANEL);
    assert_eq!(heard.presses.get(), 1);
}

#[test]
fn the_two_trees_never_wake_each_other() {
    TextSystem::bundled_only().install();
    let count = RwSignal::new(0u32);
    let read = count.clone();
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || Element::new(Panel).width(move || pct(read.get() as f32)),
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    let woke_app = Rc::new(Cell::new(0u32));
    let woke_overlay = Rc::new(Cell::new(0u32));
    let (app_wakes, overlay_wakes) = (Rc::clone(&woke_app), Rc::clone(&woke_overlay));
    app.set_wake_callback(move || app_wakes.set(app_wakes.get() + 1));
    devtools
        .app
        .set_wake_callback(move || overlay_wakes.set(overlay_wakes.get() + 1));

    count.set(50);
    assert!(woke_app.get() > 0, "a write in the app wakes it");
    assert_eq!(woke_overlay.get(), 0, "and leaves the overlay asleep");
    app.frame();

    let app_before = woke_app.get();
    click(&mut devtools, &mut app, Point { x: 300.0, y: 26.0 });
    devtools.app.frame();
    assert!(woke_overlay.get() > 0, "the press collapses the panel");
    assert_eq!(
        woke_app.get(),
        app_before,
        "a change in the overlay leaves the app asleep"
    );
}

#[test]
fn a_shortcut_reads_its_modifiers_and_one_character() {
    let parsed = Shortcut::parse("Ctrl+Shift+F").expect("a valid key");
    assert_eq!(parsed.character, "f");
    assert!(parsed.modifiers.control && parsed.modifiers.shift);
    assert!(!parsed.modifiers.alt && !parsed.modifiers.meta);
    assert!(
        Shortcut::parse("cmd+option+k").is_some_and(|key| key.modifiers.meta && key.modifiers.alt)
    );
    assert_eq!(Shortcut::parse("ctrl+shift"), None, "no character");
    assert_eq!(Shortcut::parse("ctrl+ab"), None, "two characters");
    assert_eq!(Shortcut::parse("hyper+d"), None, "an unknown modifier");
}

/// An app that fills the window and counts what reaches it: hovers, wheel
/// steps and drag moves, and records whether it counts the window as
/// focused.
struct Watched {
    hovers: Rc<Cell<u32>>,
    wheels: Rc<Cell<u32>>,
    drags: Rc<Cell<u32>>,
    focused: Rc<Cell<bool>>,
}

/// The watched app and the devtools over it, open, both laid out.
fn watched() -> (App, Devtools, Watched) {
    TextSystem::bundled_only().install();
    let watched = Watched {
        hovers: Rc::new(Cell::new(0)),
        wheels: Rc::new(Cell::new(0)),
        drags: Rc::new(Cell::new(0)),
        focused: Rc::new(Cell::new(true)),
    };
    let (hovers, wheels, drags, focused) = (
        Rc::clone(&watched.hovers),
        Rc::clone(&watched.wheels),
        Rc::clone(&watched.drags),
        Rc::clone(&watched.focused),
    );
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || {
            smudge_core::reactive::RenderEffect::new(move || {
                focused.set(smudge_core::window::focused());
            });
            Element::new(Panel)
                .width(pct(100.0))
                .height(pct(100.0))
                .on_pointer_down(|_| {})
                .on_hover(move |inside| {
                    if inside {
                        hovers.set(hovers.get() + 1);
                    }
                })
                .on_wheel(move |_| {
                    wheels.set(wheels.get() + 1);
                    Handled::Yes
                })
                .on_drag(move |_| {
                    drags.set(drags.get() + 1);
                    Handled::Yes
                })
        },
    );
    app.handle_event(InputEvent::WindowFocus(true));
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    (app, devtools, watched)
}

#[test]
fn moves_and_the_wheel_over_the_panel_do_not_reach_the_app() {
    let (mut app, mut devtools, watched) = watched();
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_BODY });
    devtools.dispatch(
        &mut app,
        InputEvent::Wheel {
            delta: Point { x: 0.0, y: 40.0 },
        },
    );
    assert_eq!(
        watched.hovers.get(),
        0,
        "the app is not hovered under the panel"
    );
    assert_eq!(watched.wheels.get(), 0);
    devtools.dispatch(
        &mut app,
        InputEvent::PointerMoved {
            position: OFF_PANEL,
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::Wheel {
            delta: Point { x: 0.0, y: 40.0 },
        },
    );
    assert_eq!(watched.hovers.get(), 1);
    assert_eq!(watched.wheels.get(), 1);
}

#[test]
fn a_press_that_began_on_the_app_keeps_the_pointer_over_the_panel() {
    let (mut app, mut devtools, watched) = watched();
    devtools.dispatch(
        &mut app,
        InputEvent::PointerMoved {
            position: OFF_PANEL,
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerDown {
            position: OFF_PANEL,
            button: PointerButton::Primary,
        },
    );
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_BODY });
    assert!(
        watched.drags.get() > 0,
        "the app's drag goes on over the panel"
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerUp {
            position: ON_BODY,
            button: PointerButton::Primary,
        },
    );
    let drags = watched.drags.get();
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_PANEL });
    assert_eq!(
        watched.drags.get(),
        drags,
        "after the release the panel has the pointer"
    );
}

#[test]
fn hiding_the_panel_during_a_press_on_it_ends_the_press() {
    let (mut app, mut devtools, _) = apps(true);
    let start = panel_origin(&devtools);
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_PANEL });
    devtools.dispatch(
        &mut app,
        InputEvent::PointerDown {
            position: ON_PANEL,
            button: PointerButton::Primary,
        },
    );
    let toggle = Shortcut::default().modifiers;
    devtools.dispatch(&mut app, key("d", toggle));
    devtools.dispatch(
        &mut app,
        InputEvent::PointerUp {
            position: ON_PANEL,
            button: PointerButton::Primary,
        },
    );
    devtools.dispatch(&mut app, key("d", toggle));
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_PANEL });
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_BODY });
    devtools.app.frame();
    assert_eq!(
        panel_origin(&devtools),
        start,
        "no button is held, so nothing drags the panel"
    );
}

#[test]
fn holding_the_toggle_key_does_not_flicker_the_panel() {
    let (mut app, mut devtools, _) = apps(false);
    let toggle = Shortcut::default().modifiers;
    assert!(devtools.dispatch(&mut app, key("d", toggle)));
    for _ in 0..5 {
        let held = InputEvent::Key(KeyEvent {
            key: Key::Character(String::from("d")),
            pressed: true,
            repeat: true,
            modifiers: toggle,
            text: None,
        });
        assert!(!devtools.dispatch(&mut app, held));
    }
    assert!(devtools.is_open());
}

#[test]
fn the_window_s_focus_follows_the_keys() {
    let (mut app, mut devtools, watched) = watched();
    click(&mut devtools, &mut app, ON_BODY);
    app.frame();
    assert!(
        !watched.focused.get(),
        "a press on the panel takes the keys from the app"
    );
    click(&mut devtools, &mut app, OFF_PANEL);
    app.frame();
    assert!(watched.focused.get(), "a press on the app gives them back");
}

/// Where the panel's top left corner is.
fn panel_origin(devtools: &Devtools) -> Point {
    let mut origin = None;
    devtools.app.visit_nodes(|depth, _, rect| {
        if depth == 1 && origin.is_none() {
            origin = Some(rect.origin());
        }
    });
    origin.expect("the overlay holds the panel")
}

#[test]
fn a_press_held_on_the_app_while_the_panel_opens_stays_with_the_app() {
    let (mut app, mut devtools, watched) = watched();
    devtools.show(&mut app, false);
    devtools.dispatch(
        &mut app,
        InputEvent::PointerMoved {
            position: OFF_PANEL,
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerDown {
            position: OFF_PANEL,
            button: PointerButton::Primary,
        },
    );
    devtools.dispatch(&mut app, key("d", Shortcut::default().modifiers));
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_BODY });
    assert!(
        watched.drags.get() > 0,
        "the app's drag goes on over the panel"
    );
}

#[test]
fn the_input_method_follows_the_keys() {
    TextSystem::bundled_only().install();
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || smudge_text::text_input(RwSignal::new(String::new())).autofocus(),
    );
    app.handle_event(InputEvent::WindowFocus(true));
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    assert!(
        devtools.ime_area(&app).is_some(),
        "the app's focused field anchors the input method"
    );
    click(&mut devtools, &mut app, ON_BODY);
    assert_eq!(
        devtools.ime_area(&app),
        None,
        "with the keys on the panel, which has no field, the input method rests"
    );
}

#[test]
fn a_press_that_moves_the_keys_keeps_an_unfocused_window_unfocused() {
    let (mut app, mut devtools, watched) = watched();
    devtools.dispatch(&mut app, InputEvent::WindowFocus(false));
    click(&mut devtools, &mut app, ON_BODY);
    click(&mut devtools, &mut app, OFF_PANEL);
    app.frame();
    assert!(
        !watched.focused.get(),
        "the press gave the app the keys, not the focus"
    );
}

#[test]
fn a_smaller_window_pulls_the_panel_inside() {
    let (mut app, mut devtools, _) = apps(true);
    devtools.dispatch(&mut app, InputEvent::PointerMoved { position: ON_PANEL });
    devtools.dispatch(
        &mut app,
        InputEvent::PointerDown {
            position: ON_PANEL,
            button: PointerButton::Primary,
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerMoved {
            position: Point { x: 700.0, y: 500.0 },
        },
    );
    devtools.dispatch(
        &mut app,
        InputEvent::PointerUp {
            position: Point { x: 700.0, y: 500.0 },
            button: PointerButton::Primary,
        },
    );
    devtools.app.frame();
    let small = Size {
        width: 400.0,
        height: 300.0,
    };
    devtools.set_viewport(small, 1.0);
    devtools.app.frame();
    let origin = panel_origin(&devtools);
    assert!(
        origin.x <= 400.0 - 320.0 && origin.y <= 300.0 - 32.0,
        "{origin:?}"
    );
}
