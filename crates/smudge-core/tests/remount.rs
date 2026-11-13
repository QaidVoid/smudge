//! Behavior tests for remounting the root of an app.
//!
//! Every test drives the same frame pipeline a window does, through the
//! harness, and remounts the way the windowing layer does: through the app,
//! between frames.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use smudge_core::app::App;
use smudge_core::channel::{self, UiSender};
use smudge_core::element::{Element, IntoElement, show};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::reactive::{RenderEffect, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::timer;
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// A counter shared between a test and the closures it hands out.
type Count = Rc<Cell<u32>>;

/// A log of focus changes, shared between a test and its handlers.
type FocusLog = Rc<RefCell<Vec<(&'static str, bool)>>>;

/// Returns a closure that adds one to `count`.
fn bump(count: &Count) -> impl FnMut() + 'static {
    let count = Rc::clone(count);
    move || count.set(count.get() + 1)
}

/// Returns a focus handler that logs `name` with each change.
fn log_focus(log: &FocusLog, name: &'static str) -> impl FnMut(bool) + 'static {
    let log = Rc::clone(log);
    move |focused| log.borrow_mut().push((name, focused))
}

/// A twenty pixel square.
fn square() -> Element<Panel> {
    panel().width(px(20.0)).height(px(20.0))
}

/// Returns the point inside the `index`th square of a row of squares.
fn in_square(index: u32) -> Point {
    Point {
        x: 20.0 * index as f32 + 10.0,
        y: 10.0,
    }
}

/// A row of two squares whose second one is focusable and logs its focus as
/// `name`.
fn focus_row(log: &FocusLog, name: &'static str) -> impl IntoElement + use<> {
    panel()
        .child(square())
        .child(square().focusable().on_focus(log_focus(log, name)))
}

#[test]
fn remount_disposes_the_effects_timers_and_channels_of_the_build() {
    let source = RwSignal::new(0u32);
    let old_effect = Count::default();
    let old_timer = Count::default();
    let old_channel = Count::default();
    let sender: Rc<RefCell<Option<UiSender<u32>>>> = Rc::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let old_effect = Rc::clone(&old_effect);
        let timer_fired = bump(&old_timer);
        let mut received = bump(&old_channel);
        let sender = Rc::clone(&sender);
        let source = source.clone();
        move || {
            RenderEffect::new(move || {
                source.get();
                old_effect.set(old_effect.get() + 1);
            });
            timer::after(Duration::from_secs(1), timer_fired);
            *sender.borrow_mut() = Some(channel::channel(move |_: u32| received()));
            panel()
        }
    });
    harness.frame();
    assert_eq!(old_effect.get(), 1, "the effect ran once at mount");

    let new_timer = Count::default();
    harness.app().remount({
        let timer_fired = bump(&new_timer);
        move || {
            timer::after(Duration::from_secs(1), timer_fired);
            panel()
        }
    });
    source.set(1);
    harness.advance(Duration::from_secs(2));

    assert_eq!(old_effect.get(), 1, "the old build's effect stopped");
    assert_eq!(old_timer.get(), 0, "the old build's timer stopped");
    assert_eq!(new_timer.get(), 1, "the new build's timer runs");
    let sender = sender
        .borrow()
        .clone()
        .expect("the build stored its sender");
    assert!(
        sender.send(1).is_err(),
        "a sender from the old build answers with an error"
    );
    harness.frame();
    assert_eq!(old_channel.get(), 0, "nothing reached the old handler");
}

#[test]
fn remount_keeps_signals_created_outside_the_build() {
    let color = RwSignal::new(Color::BLACK);
    let build = {
        let color = color.clone();
        move || {
            let write = color.clone();
            panel().child(
                square()
                    .background(move || color.get().into())
                    .on_press(move || write.set(Color::rgb(0, 0, 255))),
            )
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    harness.click(in_square(0));
    harness.frame();

    harness.app().remount(build);
    harness.frame();
    assert_eq!(
        harness.pixel(10, 10),
        Some([0, 0, 255, 255]),
        "the new build reads the value the old one wrote"
    );

    color.set(Color::rgb(255, 0, 0));
    harness.frame();
    assert_eq!(
        harness.pixel(10, 10),
        Some([255, 0, 0, 255]),
        "the new build's bindings follow the signal"
    );
}

#[test]
fn remount_keeps_the_theme_binding() {
    let dark = RwSignal::new(false);
    let build = || panel().child(square().background(ColorToken::SURFACE));
    let mut harness = Harness::new(100.0, 60.0, 1.0, build);
    harness.app().bind_theme({
        let dark = dark.clone();
        move || {
            if dark.get() {
                Theme::dark()
            } else {
                Theme::light()
            }
        }
    });
    harness.frame();

    harness.app().remount(build);
    harness.frame();
    dark.set(true);
    harness.frame();

    let mut expected = Harness::new(100.0, 60.0, 1.0, move || {
        panel().child(square().background(Theme::dark().color(ColorToken::SURFACE)))
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn a_theme_bound_just_before_a_remount_still_lands() {
    let build = || panel().child(square().background(ColorToken::SURFACE));
    let mut harness = Harness::new(100.0, 60.0, 1.0, build);
    harness.app().bind_theme(Theme::dark);
    harness.app().remount(build);
    harness.frame();

    let mut expected = Harness::new(100.0, 60.0, 1.0, move || {
        panel().child(square().background(Theme::dark().color(ColorToken::SURFACE)))
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn remount_restores_the_focus_by_path() {
    let log = FocusLog::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || show(|| true, move || focus_row(&log, "old"))
    });
    harness.frame();
    harness.click(in_square(1));
    assert_eq!(*log.borrow(), [("old", true)]);

    harness.app().remount({
        let log = Rc::clone(&log);
        move || show(|| true, move || focus_row(&log, "new"))
    });
    harness.frame();

    assert_eq!(
        *log.borrow(),
        [("old", true), ("new", true)],
        "the node at the same path took the focus, inside a show"
    );
    assert!(harness.app().focused().is_some());
}

#[test]
fn remount_leaves_the_focus_empty_when_the_path_leads_to_no_focusable_node() {
    let log = FocusLog::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || focus_row(&log, "old")
    });
    harness.frame();
    harness.click(in_square(1));

    harness.app().remount({
        let log = Rc::clone(&log);
        move || {
            panel()
                .child(square().focusable().on_focus(log_focus(&log, "first")))
                .child(square())
        }
    });
    harness.frame();

    assert_eq!(*log.borrow(), [("old", true)], "no new node took the focus");
    assert_eq!(harness.app().focused(), None);
}

#[test]
fn a_restored_focus_outlasts_the_autofocus_of_the_new_build() {
    let log = FocusLog::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || focus_row(&log, "old")
    });
    harness.frame();
    harness.click(in_square(1));

    harness.app().remount({
        let log = Rc::clone(&log);
        move || {
            panel()
                .child(square().autofocus().on_focus(log_focus(&log, "auto")))
                .child(square().focusable().on_focus(log_focus(&log, "kept")))
        }
    });
    harness.frame();

    assert_eq!(*log.borrow(), [("old", true), ("kept", true)]);
}

#[test]
fn remount_restores_no_focus_outside_a_trap_the_new_build_mounts() {
    let log = FocusLog::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || focus_row(&log, "old")
    });
    harness.frame();
    harness.click(in_square(1));

    harness.app().remount({
        let log = Rc::clone(&log);
        move || {
            panel()
                .child(square())
                .child(square().focusable().on_focus(log_focus(&log, "behind")))
                .child(
                    square()
                        .trap_focus()
                        .child(square().autofocus().on_focus(log_focus(&log, "dialog"))),
                )
        }
    });
    harness.frame();

    assert_eq!(
        *log.borrow(),
        [("old", true), ("dialog", true)],
        "the focus went into the trap, not behind it"
    );
}

/// The state a scene draws, kept outside every build.
#[derive(Clone)]
struct SceneState {
    color: RwSignal<Color>,
    wide: RwSignal<bool>,
    extra: RwSignal<bool>,
}

/// A first version of a scene: two tall squares side by side.
fn scene_before(state: SceneState) -> impl IntoElement {
    panel()
        .child(square().height(px(50.0)).background(Color::BLACK))
        .child(
            square()
                .height(px(50.0))
                .background(move || state.color.get().into()),
        )
}

/// A second version of the same scene, laid out differently, that reads
/// the state the first one changed.
fn scene_after(state: SceneState) -> impl IntoElement {
    panel()
        .gap(Size {
            width: 4.0,
            height: 0.0,
        })
        .child(
            square()
                .width(move || px(if state.wide.get() { 60.0 } else { 20.0 }))
                .background(move || state.color.get().into()),
        )
        .child(show(
            move || state.extra.get(),
            || square().background(Color::rgb(0, 128, 0)),
        ))
}

#[test]
fn the_pixels_after_a_remount_match_a_fresh_app_with_the_same_state() {
    let state = SceneState {
        color: RwSignal::new(Color::rgb(255, 0, 0)),
        wide: RwSignal::new(false),
        extra: RwSignal::new(false),
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let state = state.clone();
        move || scene_before(state)
    });
    harness.frame();
    state.color.set(Color::rgb(0, 0, 255));
    state.wide.set(true);
    state.extra.set(true);
    harness.frame();

    harness.app().remount({
        let state = state.clone();
        move || scene_after(state)
    });
    let frame = harness.frame();
    assert!(frame.full, "a remount repaints the whole surface");

    let mut fresh = Harness::new(100.0, 60.0, 1.0, move || scene_after(state));
    fresh.frame();
    harness.assert_matches(&fresh);
    harness.assert_matches_full_repaint();
}

#[test]
fn remount_resets_the_accessibility_tree() {
    let build = || panel().child(square().role(Role::Button));
    let mut harness = Harness::new(100.0, 60.0, 1.0, build);
    harness.frame();
    harness.app().take_a11y_changes();

    harness.app().remount(build);
    harness.frame();

    let app = harness.app();
    assert!(
        app.take_a11y_changes().reset,
        "the backend rebuilds its tree"
    );
    let roots = app.semantics_roots();
    assert_eq!(roots.len(), 1);
    assert_eq!(
        app.semantics_node(roots[0]).map(|node| node.role),
        Some(Role::Button),
        "the projection shows the new node"
    );
}

#[test]
fn remount_hovers_the_new_node_under_the_pointer() {
    let hovered = Rc::new(Cell::new(false));
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        panel().child(square().on_hover(|_| {}))
    });
    harness.frame();
    harness.move_to(in_square(0));

    harness.app().remount({
        let hovered = Rc::clone(&hovered);
        move || panel().child(square().on_hover(move |on| hovered.set(on)))
    });
    harness.frame();

    assert!(hovered.get(), "the new node under the pointer is hovered");
}

#[test]
fn a_press_held_across_a_remount_ends_without_a_press_on_the_new_tree() {
    let old_up = Count::default();
    let new_presses = Count::default();
    let new_ups = Count::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let mut up = bump(&old_up);
        move || panel().child(square().on_press(|| {}).on_pointer_up(move |_| up()))
    });
    harness.frame();
    harness.button_down(in_square(0));

    harness.app().remount({
        let pressed = bump(&new_presses);
        let mut up = bump(&new_ups);
        move || panel().child(square().on_press(pressed).on_pointer_up(move |_| up()))
    });
    assert_eq!(old_up.get(), 1, "the pressed node heard the button come up");
    harness.frame();
    harness.button_up(in_square(0));

    assert_eq!(new_presses.get(), 0, "the release clicked nothing new");
    assert_eq!(new_ups.get(), 0, "the release reached no new handler");
}

#[test]
fn a_remount_inside_a_running_frame_hook_does_not_panic() {
    let dialog = RwSignal::new(true);
    let ticks = Count::default();
    let build = move || {
        timer::after(Duration::ZERO, || {});
        let sender = channel::channel(|_: u32| {});
        sender
            .send(1)
            .expect("the channel is open while its build lives");
        panel().child(square().on_press(|| {})).child(show(
            move || dialog.get(),
            || {
                square()
                    .trap_focus()
                    .child(square().autofocus())
                    .child(square().hidden(true).child(square().autofocus()))
            },
        ))
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.app().set_tick({
        let mut tick = bump(&ticks);
        move |_| tick()
    });
    harness.frame();
    assert!(
        harness.app().focused().is_some(),
        "the trap holds the focus"
    );
    harness.button_down(in_square(0));
    let ticks_before = ticks.get();

    // The windowing layer hands its input hook the app right before it runs
    // a frame, and this hook remounts there, with a press held, a trap
    // active, an autofocus request pending, a due timer and a queued value.
    let hook: Rc<dyn Fn(&mut App)> = Rc::new(move |app| app.remount(build.clone()));
    hook(harness.app());
    harness.frame();
    harness.button_up(in_square(0));
    harness.settle();

    assert!(ticks.get() > ticks_before, "the tick survived the remount");
    assert!(
        harness.app().focused().is_some(),
        "the focus is back in the trap"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_hidden_autofocus_request_waits_after_the_focus_is_restored() {
    let log = FocusLog::default();
    let shown = RwSignal::new(false);
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || focus_row(&log, "old")
    });
    harness.frame();
    harness.click(in_square(1));

    harness.app().remount({
        let log = Rc::clone(&log);
        let shown = shown.clone();
        move || {
            panel()
                .child(square())
                .child(square().focusable().on_focus(log_focus(&log, "kept")))
                .child(
                    square()
                        .hidden(move || !shown.get())
                        .child(square().autofocus().on_focus(log_focus(&log, "late"))),
                )
        }
    });
    harness.frame();
    shown.set(true);
    harness.frame();

    assert_eq!(
        *log.borrow(),
        [
            ("old", true),
            ("kept", true),
            ("kept", false),
            ("late", true)
        ]
    );
}

#[test]
fn nothing_the_old_build_scheduled_runs_after_a_remount() {
    let source = RwSignal::new(0_u32);
    let runs = Count::default();
    let received = Count::default();
    let sender: Rc<RefCell<Option<UiSender<u32>>>> = Rc::default();
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let runs = Rc::clone(&runs);
        let mut on_value = bump(&received);
        let sender = Rc::clone(&sender);
        let source = source.clone();
        move || {
            let read = source.clone();
            RenderEffect::new(move || {
                read.get();
                runs.set(runs.get() + 1);
            });
            *sender.borrow_mut() = Some(channel::channel(move |_: u32| on_value()));
            panel().child(square().width(move || px(source.get() as f32 + 1.0)))
        }
    });
    harness.frame();

    source.set(5);
    sender
        .borrow()
        .as_ref()
        .expect("the build stored its sender")
        .send(3)
        .expect("the channel is open before the remount");
    harness.app().remount(panel);
    harness.frame();

    assert_eq!(runs.get(), 1, "the old effect ran again");
    assert_eq!(received.get(), 0, "the old channel handler ran");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_remount_asks_the_window_to_restart_the_input_method_once() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, panel);
    harness.frame();
    assert!(!harness.app().take_ime_reset(), "no remount happened yet");

    harness.app().remount(panel);

    assert!(harness.app().take_ime_reset());
    assert!(!harness.app().take_ime_reset(), "the request is taken once");
}

#[test]
fn a_click_right_after_a_remount_reaches_the_new_tree() {
    let presses = Count::default();
    let log = FocusLog::default();
    let build = {
        let presses = Rc::clone(&presses);
        let log = Rc::clone(&log);
        move || {
            panel().child(square()).child(
                square()
                    .focusable()
                    .on_focus(log_focus(&log, "row"))
                    .on_press(bump(&presses)),
            )
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    harness.click(in_square(1));
    assert_eq!(presses.get(), 1);

    harness.app().remount(build);
    harness.click(in_square(1));

    assert_eq!(
        presses.get(),
        2,
        "the click before the next frame pressed the remounted node"
    );
    assert!(
        harness.app().focused().is_some(),
        "the click did not clear the restored focus"
    );
}
