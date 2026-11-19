use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::app::{App, AppConfig, FrameOutput, RepaintMode};
use smudge_core::element::Element;
use smudge_core::geometry::{Color, Corners, PixelRect, Point, Rect, Size};
use smudge_core::input::{Handled, InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::paint::{DisplayItem, DisplayList, PaintOp};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{pct, px};
use smudge_core::widget::Panel;
use smudge_text::TextSystem;

use super::{Devtools, Shortcut};
use crate::present::CpuFrames;
use web_time::Instant;

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

#[test]
fn a_frame_recorded_while_the_window_paces_itself_requests_no_redraw() {
    let (_app, mut devtools, _) = apps(true);
    let wakes = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&wakes);
    devtools.set_wake(move || counted.set(counted.get() + 1));
    devtools.record(smudge_devtools::FrameRecord::default(), true);
    assert_eq!(
        wakes.get(),
        0,
        "the window's next paced frame shows the bar"
    );
    devtools.app.frame();
    devtools.record(smudge_devtools::FrameRecord::default(), false);
    assert_eq!(
        wakes.get(),
        1,
        "an idle window draws once more to show the bar"
    );
}

/// A white surface with one blue box at `at`, in logical pixels.
fn boxed(at: Rect) -> DisplayList {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: at,
        radius: Corners::all(0.0),
        color: Color::rgb(30, 90, 200),
    });
    list
}

#[test]
fn the_damage_tint_matches_the_fixed_damage_overlay_pixel_for_pixel() {
    TextSystem::bundled_only().install();
    for scale in [1.0, 2.0] {
        let (width, height) = (
            (VIEWPORT.width * scale) as u32,
            (VIEWPORT.height * scale) as u32,
        );
        let mut fixed = CpuFrames::new(width, height, true);
        let mut overlaid = CpuFrames::new(width, height, false);
        overlaid.set_overlay(true);
        let mut devtools = Devtools::with(VIEWPORT, scale, true, Shortcut::default());
        let mut switches = devtools.tools.switches();
        switches.damage_tint = true;
        devtools.tools.set_switches(switches);
        devtools.app.frame();
        let mut panel = None;
        devtools.app.visit_nodes(|depth, _, rect| {
            if depth == 1 {
                panel = Some(rect);
            }
        });
        let panel = panel.expect("the panel shows");

        let (mut tinted, mut overlay_tinted) = (
            vec![0; (width * height) as usize],
            vec![0; (width * height) as usize],
        );
        let (from, to) = (
            Rect::new(500.0, 300.0, 580.0, 360.0),
            Rect::new(530.5, 320.25, 610.5, 380.25),
        );
        let whole = [PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        }];
        let moved: Vec<PixelRect> = [from, to]
            .into_iter()
            .filter_map(|rect| PixelRect::from_logical(rect, scale, width, height))
            .collect();
        for (list, damage) in [(boxed(from), &whole[..]), (boxed(to), &moved[..])] {
            let frame = FrameOutput {
                list: &list,
                damage,
                full: false,
                clear_color: Color::WHITE,
                scale,
            };
            fixed.draw(&frame, None, 0, &mut tinted);
            fixed.presented(&frame);
            devtools.show_damage(damage, Instant::now());
            let over = devtools.app.frame();
            overlaid.draw(&frame, Some(&over), 0, &mut overlay_tinted);
            overlaid.presented(&frame);

            let mut compared = 0;
            for y in 0..height {
                for x in 0..width {
                    let at = Point {
                        x: (x as f32 + 0.5) / scale,
                        y: (y as f32 + 0.5) / scale,
                    };
                    if panel.contains(at) {
                        continue;
                    }
                    let index = (y * width + x) as usize;
                    assert_eq!(
                        overlay_tinted[index], tinted[index],
                        "scale {scale}: the pixel at {x}, {y}"
                    );
                    compared += 1;
                }
            }
            assert!(compared > 0);
        }
        let inside = |x: f32, y: f32| ((y * scale) as u32 * width + (x * scale) as u32) as usize;
        assert_ne!(
            tinted[inside(600.0, 370.0)],
            0x001e_5ac8,
            "scale {scale}: the moved box is tinted, so the comparison covers a tint"
        );
    }
}

#[test]
fn the_full_repaint_switch_makes_the_app_repaint_everything() {
    let (mut app, mut devtools, _) = apps(true);
    devtools.start_repaint(RepaintMode::Damage);
    let whole = PixelRect {
        x: 0,
        y: 0,
        width: VIEWPORT.width as u32,
        height: VIEWPORT.height as u32,
    };
    let mut switches = devtools.tools.switches();
    switches.full_repaint = true;
    devtools.tools.set_switches(switches);
    devtools.apply_switches(&mut app);
    app.frame();
    assert_eq!(
        app.frame().damage,
        [whole],
        "an unchanged frame repaints everything"
    );

    switches.full_repaint = false;
    devtools.tools.set_switches(switches);
    devtools.apply_switches(&mut app);
    app.frame();
    assert!(
        app.frame().damage.is_empty(),
        "an unchanged frame repaints nothing"
    );
}

/// Whether the overlay's last frame painted the damage tint.
fn tints(devtools: &mut Devtools) -> bool {
    let output = devtools.app.frame();
    output.list.items().iter().any(|item| {
        matches!(
            item,
            DisplayItem::Op(PaintOp::FillRect { color, .. }) if *color == Color::rgba(255, 64, 64, 96)
        )
    })
}

#[test]
fn the_tint_forgets_old_damage_when_it_comes_back() {
    let (mut app, mut devtools, _) = apps(true);
    let damage = [PixelRect {
        x: 600,
        y: 400,
        width: 40,
        height: 20,
    }];
    let mut switches = devtools.tools.switches();
    switches.damage_tint = true;
    devtools.tools.set_switches(switches);
    devtools.show_damage(&damage, Instant::now());
    assert!(tints(&mut devtools));

    switches.damage_tint = false;
    devtools.tools.set_switches(switches);
    switches.damage_tint = true;
    devtools.tools.set_switches(switches);
    assert!(
        !tints(&mut devtools),
        "turned on again, it waits for a new frame"
    );

    devtools.show_damage(&damage, Instant::now());
    devtools.show(&mut app, false);
    devtools.show(&mut app, true);
    assert!(!tints(&mut devtools), "reopened, it waits for a new frame");
}

/// The opacities of the flashes the overlay's next frame paints, oldest
/// first.
fn flash_opacities(devtools: &mut Devtools) -> Vec<u8> {
    let output = devtools.app.frame();
    output
        .list
        .items()
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Op(PaintOp::FillRect { color, .. })
                if (color.r, color.g, color.b) == (70, 210, 120) =>
            {
                Some(color.a)
            }
            _ => None,
        })
        .collect()
}

/// A damaged rectangle in physical pixels, away from the panel.
fn damaged(x: u32) -> [PixelRect; 1] {
    [PixelRect {
        x,
        y: 400,
        width: 30,
        height: 20,
    }]
}

/// Turns paint flashing on or off.
fn set_flashing(devtools: &Devtools, on: bool) {
    let mut switches = devtools.tools.switches();
    switches.paint_flashing = on;
    devtools.tools.set_switches(switches);
}

#[test]
fn paint_flashes_fade_out_and_then_stop() {
    let (_app, mut devtools, _) = apps(true);
    set_flashing(&devtools, true);
    let start = Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    devtools.show_damage(&damaged(500), at(0));
    devtools.fade(at(0));
    assert_eq!(flash_opacities(&mut devtools), [160]);

    devtools.show_damage(&damaged(600), at(100));
    devtools.fade(at(250));
    assert_eq!(
        flash_opacities(&mut devtools),
        [80, 112],
        "older fades more"
    );
    assert!(devtools.is_fading());

    devtools.fade(at(550));
    assert_eq!(flash_opacities(&mut devtools), [16], "the first is gone");

    devtools.fade(at(700));
    assert_eq!(flash_opacities(&mut devtools), Vec::<u8>::new());
    assert!(!devtools.is_fading(), "the window may go idle");
}

#[test]
fn turning_paint_flashing_off_ends_the_fade() {
    let (_app, mut devtools, _) = apps(true);
    set_flashing(&devtools, true);
    let now = Instant::now();
    devtools.show_damage(&damaged(500), now);
    devtools.fade(now);
    assert!(devtools.is_fading());

    set_flashing(&devtools, false);
    devtools.fade(now);
    assert!(!devtools.is_fading());
    assert_eq!(flash_opacities(&mut devtools), Vec::<u8>::new());

    set_flashing(&devtools, true);
    devtools.fade(now);
    assert_eq!(
        flash_opacities(&mut devtools),
        Vec::<u8>::new(),
        "turned on again, it waits for a new frame"
    );
}

#[test]
fn a_reopened_panel_shows_no_old_flashes() {
    let (mut app, mut devtools, _) = apps(true);
    set_flashing(&devtools, true);
    let now = Instant::now();
    devtools.show_damage(&damaged(500), now);
    devtools.fade(now);
    assert_eq!(flash_opacities(&mut devtools), [160]);

    devtools.show(&mut app, false);
    devtools.show(&mut app, true);
    devtools.fade(now);
    assert_eq!(flash_opacities(&mut devtools), Vec::<u8>::new());
}

/// The rectangles the overlay's next frame outlines in the colors of the
/// first two depths, which leaves out the panel's own border.
fn outlines(devtools: &mut Devtools) -> Vec<Rect> {
    let depths = [Color::rgb(255, 120, 80), Color::rgb(250, 200, 60)];
    let output = devtools.app.frame();
    output
        .list
        .items()
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Op(PaintOp::StrokeRect { rect, color, .. }) if depths.contains(color) => {
                Some(*rect)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn layout_bounds_outline_every_node_and_follow_a_layout_change() {
    TextSystem::bundled_only().install();
    let width = RwSignal::new(100.0_f32);
    let read = width.clone();
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
                .child(
                    Element::new(Panel)
                        .width(move || px(read.get()))
                        .height(px(40.0)),
                )
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    let wakes = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&wakes);
    devtools.set_wake(move || counted.set(counted.get() + 1));
    let mut switches = devtools.tools.switches();
    switches.layout_bounds = true;
    devtools.tools.set_switches(switches);
    devtools.app.frame();
    wakes.set(0);

    devtools.show_bounds(&app, false);
    assert_eq!(
        wakes.get(),
        1,
        "an idle window draws once more to show them"
    );
    assert_eq!(
        outlines(&mut devtools),
        [
            Rect::new(0.0, 0.0, VIEWPORT.width, VIEWPORT.height),
            Rect::new(0.0, 0.0, 100.0, 40.0),
        ]
    );

    devtools.show_bounds(&app, false);
    assert_eq!(wakes.get(), 1, "nothing moved, so nothing is asked for");

    width.set(180.0);
    app.frame();
    devtools.show_bounds(&app, true);
    assert_eq!(wakes.get(), 1, "a paced window draws the next frame itself");
    assert_eq!(outlines(&mut devtools)[1], Rect::new(0.0, 0.0, 180.0, 40.0));

    switches.layout_bounds = false;
    devtools.tools.set_switches(switches);
    devtools.show_bounds(&app, false);
    assert!(outlines(&mut devtools).is_empty());

    switches.layout_bounds = true;
    devtools.tools.set_switches(switches);
    devtools.show_bounds(&app, false);
    assert_eq!(outlines(&mut devtools).len(), 2, "on again, they come back");

    devtools.show(&mut app, false);
    devtools.show(&mut app, true);
    assert!(
        outlines(&mut devtools).is_empty(),
        "a reopened panel waits for a walk"
    );
    devtools.show_bounds(&app, false);
    assert_eq!(
        outlines(&mut devtools).len(),
        2,
        "and the next walk shows them"
    );
}

#[test]
fn a_node_with_an_unbounded_size_lets_layout_bounds_go_idle() {
    TextSystem::bundled_only().install();
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || {
            Element::new(Panel)
                .width(pct(100.0))
                .height(pct(100.0))
                .child(
                    Element::new(Panel)
                        .width(px(f32::INFINITY))
                        .height(px(10.0)),
                )
                .child(
                    Element::new(Panel)
                        .width(px(10.0))
                        .height(pct(f32::INFINITY)),
                )
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    let wakes = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&wakes);
    devtools.set_wake(move || counted.set(counted.get() + 1));
    let mut switches = devtools.tools.switches();
    switches.layout_bounds = true;
    devtools.tools.set_switches(switches);
    devtools.app.frame();
    wakes.set(0);

    devtools.show_bounds(&app, false);
    devtools.app.frame();
    devtools.show_bounds(&app, false);
    assert_eq!(
        wakes.get(),
        1,
        "an unchanged tree asks for no second redraw"
    );
}

/// The center of the overlay's text that reads `text`.
fn overlay_text(devtools: &Devtools, text: &str) -> Point {
    let bounds = devtools
        .app
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some(text))
        .unwrap_or_else(|| panic!("the overlay shows {text:?}"))
        .bounds;
    Point {
        x: (bounds.x0 + bounds.x1) / 2.0,
        y: (bounds.y0 + bounds.y1) / 2.0,
    }
}

#[test]
fn picking_a_button_selects_it_and_names_where_it_was_built() {
    TextSystem::bundled_only().install();
    let pressed = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&pressed);
    let line = line!() + 10;
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || {
            smudge_widgets::column().padding(px(300.0)).child(
                smudge_widgets::button("Save").on_press(move || counted.set(counted.get() + 1)),
            )
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    let at = overlay_text(&devtools, "Inspector");
    click(&mut devtools, &mut app, at);
    devtools.app.frame();
    let at = overlay_text(&devtools, "Pick");
    click(&mut devtools, &mut app, at);
    devtools.app.frame();

    let save = app
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == smudge_core::semantics::Role::Button)
        .expect("the app shows its button");
    let center = Point {
        x: (save.bounds.x0 + save.bounds.x1) / 2.0,
        y: (save.bounds.y0 + save.bounds.y1) / 2.0,
    };
    click(&mut devtools, &mut app, center);
    app.frame();
    assert_eq!(pressed.get(), 0, "a pick does not press the button");
    assert_eq!(devtools.tools.selected(), Some(save.id));
    assert!(!devtools.tools.is_picking(), "a pick ends pick mode");

    devtools.inspect(&app, false);
    devtools.app.frame();
    let details = app.node_details(save.id).expect("the button is live");
    let padding = details.boxes.padding;
    let shown = devtools
        .app
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .find(|text| text.starts_with("Panel (Button"))
        .expect("the details describe the button");
    assert!(shown.contains(&format!("{}:{line}:", file!())), "{shown}");
    assert!(
        shown.contains(&format!(
            "padding: {}, {} {}x{}",
            padding.x0,
            padding.y0,
            padding.width(),
            padding.height()
        )),
        "{shown}"
    );
    let highlighted = devtools.app.frame().list.items().iter().any(
        |item| matches!(item, DisplayItem::Op(PaintOp::FillRect { rect, .. }) if *rect == padding),
    );
    assert!(highlighted, "the selected button's boxes are highlighted");

    let root = app
        .node_at(Point { x: 10.0, y: 10.0 })
        .expect("the root is there");
    devtools.tools.select(Some(root));
    devtools.inspect(&app, false);
    let content = app
        .node_details(root)
        .expect("the root is live")
        .boxes
        .content;
    let followed = devtools.app.frame().list.items().iter().any(
        |item| matches!(item, DisplayItem::Op(PaintOp::FillRect { rect, .. }) if *rect == content),
    );
    assert!(followed, "the highlight follows the selection");

    let row = devtools
        .app
        .semantics_nodes()
        .into_iter()
        .find(|node| {
            node.value
                .as_deref()
                .is_some_and(|text| text.trim() == "Label")
        })
        .expect("the tree lists the button's label");
    let at = Point {
        x: (row.bounds.x0 + row.bounds.x1) / 2.0,
        y: (row.bounds.y0 + row.bounds.y1) / 2.0,
    };
    click(&mut devtools, &mut app, at);
    let label = app
        .node_details(devtools.tools.selected().expect("a row selects its node"))
        .expect("the node is live");
    assert!(label.type_name.ends_with("Label"), "{}", label.type_name);
}

#[test]
fn copy_as_test_writes_what_a_harness_of_the_node_draws() {
    TextSystem::bundled_only().install();
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || {
            smudge_widgets::column()
                .padding(px(300.0))
                .child(smudge_widgets::button("Save"))
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    devtools.copy_dir =
        std::env::temp_dir().join(format!("smudge-copy-{}-{nanos}", std::process::id()));
    devtools.app.frame();
    let at = overlay_text(&devtools, "Inspector");
    click(&mut devtools, &mut app, at);
    devtools.app.frame();
    let save = app
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == smudge_core::semantics::Role::Button)
        .expect("the app shows its button");
    devtools.tools.select(Some(save.id));
    devtools.inspect(&app, false);
    let at = overlay_text(&devtools, "Copy as test");
    click(&mut devtools, &mut app, at);
    devtools.inspect(&app, false);
    devtools.app.frame();

    let dump = std::fs::read_to_string(devtools.copy_dir.join("panel-1.txt"))
        .expect("the dump is written");
    let png = std::fs::read(devtools.copy_dir.join("panel-1.png")).expect("the image is written");
    let wrote = devtools
        .app
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .any(|text| text.starts_with("Wrote ") && text.contains("panel-1.png"));
    assert!(wrote, "the panel names the files");

    let size = save.bounds;
    let mut harness = smudge_test::Harness::new(size.width(), size.height(), 1.0, || {
        smudge_widgets::button("Save")
    });
    harness.frame();
    assert_eq!(harness.layout_dump(), dump);
    assert!(harness.png() == png, "the image matches the harness's");
    let _ = std::fs::remove_dir_all(&devtools.copy_dir);
}

#[test]
fn the_reactive_tab_counts_runs_a_second_where_each_effect_was_created() {
    TextSystem::bundled_only().install();
    let created = line!() + 11;
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || {
            let count = RwSignal::new(0_u32);
            let (seen, press) = (count.clone(), count);
            std::mem::forget(smudge_core::reactive::RenderEffect::new(move || seen.get()));
            smudge_widgets::column().padding(px(300.0)).child(
                smudge_widgets::button("More").on_press(move || press.update(|count| *count += 1)),
            )
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    let at = overlay_text(&devtools, "Reactive");
    click(&mut devtools, &mut app, at);
    devtools.app.frame();
    let start = Instant::now();
    devtools.trace(&app, start, false);

    let more = Point { x: 400.0, y: 317.0 };
    for _ in 0..4 {
        click(&mut devtools, &mut app, more);
        app.frame();
    }
    devtools.trace(&app, start + Duration::from_secs(2), false);
    devtools.app.frame();
    let text = devtools
        .app
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .find(|text| text.starts_with("Busiest effects"))
        .expect("the reactive tab shows its text");
    let place = format!("{}:{created}:", file!());
    let line = text
        .lines()
        .find(|line| line.contains(&place))
        .unwrap_or_else(|| panic!("the effect's place is listed: {text}"));
    assert!(
        line.trim_start().starts_with("2.0"),
        "four runs in two seconds: {line}"
    );
    assert!(text.contains("Owners: "), "{text}");
}

#[test]
fn the_focus_overlay_numbers_tab_stops_and_the_access_tab_warns() {
    TextSystem::bundled_only().install();
    let pressed = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&pressed);
    let mut app = App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || {
            smudge_widgets::column()
                .padding_xy(Size {
                    width: 360.0,
                    height: 100.0,
                })
                .child(
                    Element::new(Panel)
                        .width(px(40.0))
                        .height(px(40.0))
                        .role(smudge_core::semantics::Role::Button)
                        .focusable()
                        .on_press(move || counted.set(counted.get() + 1)),
                )
                .child(smudge_widgets::button("Save"))
        },
    );
    app.frame();
    let mut devtools = Devtools::with(VIEWPORT, 1.0, true, Shortcut::default());
    devtools.app.frame();
    let mut switches = devtools.tools.switches();
    switches.focus = true;
    devtools.tools.set_switches(switches);
    devtools.observe(&app, false);
    devtools.app.frame();
    let badge = |devtools: &Devtools, text: &str| {
        devtools
            .app
            .semantics_nodes()
            .into_iter()
            .find(|node| node.value.as_deref() == Some(text))
            .map(|node| node.bounds)
    };
    let first = badge(&devtools, "1").expect("the first tab stop is numbered");
    assert_eq!((first.x0, first.y0), (360.0, 100.0), "at the icon button");
    assert!(
        badge(&devtools, "2").is_some(),
        "the Save button is the second"
    );

    let on_badge = Point {
        x: first.x0 + 2.0,
        y: first.y0 + 2.0,
    };
    click(&mut devtools, &mut app, on_badge);
    assert_eq!(
        pressed.get(),
        1,
        "a badge lets the press through to the app"
    );

    let at = overlay_text(&devtools, "Access");
    click(&mut devtools, &mut app, at);
    devtools.app.frame();
    devtools.observe(&app, false);
    devtools.app.frame();
    let text = devtools
        .app
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .find(|text| text.starts_with("Warnings: "))
        .expect("the access tab shows its text");
    assert!(
        text.starts_with(
            "Warnings: 1\n  the node at 360, 100 40x40 takes the focus but has no name"
        ),
        "{text}"
    );
    assert!(text.contains("Button \"Save\""), "{text}");
}
