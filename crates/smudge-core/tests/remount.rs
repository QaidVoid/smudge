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
    let hook: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
        app.remount(build.clone());
    });
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

#[test]
fn a_restored_focus_whose_handler_moves_it_is_still_revealed() {
    let spacer = RwSignal::new(20.0_f32);
    let offset = RwSignal::new(0.0_f32);
    let build = {
        let spacer = spacer.clone();
        let offset = offset.clone();
        move || {
            let write = offset.clone();
            let read = offset.clone();
            let height = spacer.clone();
            let grow = spacer.clone();
            panel()
                .column()
                .height(px(60.0))
                .overflow(smudge_core::style::Overflow::Scroll)
                .on_reveal(move |revealed| write.set(revealed.y))
                .scroll_offset(move || Point {
                    x: 0.0,
                    y: read.get(),
                })
                .child(panel().height(move || px(height.get())).shrink(0.0))
                .child(square().shrink(0.0).focusable().on_focus(move |focused| {
                    if focused {
                        grow.set(100.0);
                    }
                }))
                .child(panel().height(px(200.0)).shrink(0.0))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    harness.press_key(smudge_core::input::Key::Tab);
    harness.frame();
    assert!(harness.app().focused().is_some());
    spacer.set(20.0);
    offset.set(0.0);
    harness.frame();

    harness.app().remount(build);
    harness.frame();

    assert_eq!(
        offset.get(),
        60.0,
        "the focused square, moved to 100..120 by its own handler, ends at the bottom edge"
    );
}

#[test]
fn an_autofocus_whose_handler_moves_it_after_a_remount_is_revealed() {
    let spacer = RwSignal::new(20.0_f32);
    let offset = RwSignal::new(0.0_f32);
    let mut harness = Harness::new(100.0, 60.0, 1.0, panel);
    harness.frame();
    assert!(
        harness.app().focused().is_none(),
        "nothing is focused before"
    );

    let build = {
        let spacer = spacer.clone();
        let offset = offset.clone();
        move || {
            let write = offset.clone();
            let read = offset.clone();
            let height = spacer.clone();
            let grow = spacer.clone();
            panel()
                .column()
                .height(px(60.0))
                .overflow(smudge_core::style::Overflow::Scroll)
                .on_reveal(move |revealed| write.set(revealed.y))
                .scroll_offset(move || Point {
                    x: 0.0,
                    y: read.get(),
                })
                .child(panel().height(move || px(height.get())).shrink(0.0))
                .child(
                    square()
                        .shrink(0.0)
                        .focusable()
                        .autofocus()
                        .on_focus(move |focused| {
                            if focused {
                                grow.set(100.0);
                            }
                        }),
                )
                .child(panel().height(px(200.0)).shrink(0.0))
        }
    };
    harness.app().remount(build);
    harness.frame();

    assert!(
        harness.app().focused().is_some(),
        "the autofocus took the focus"
    );
    assert_eq!(
        offset.get(),
        60.0,
        "the autofocused square, moved to 100..120 by its own handler, ends at the bottom edge"
    );
}

/// A slot a build stores the signal it created in, so a test can read it.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
type Slot<T> = Rc<RefCell<Option<RwSignal<T>>>>;

/// Records that a patch kept the layouts of the types whose probes are
/// `probes`, as the window does after a patch.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
fn keep(probes: &[usize]) {
    // SAFETY: the tests never change a type, so every layout they list is
    // the one the values hold.
    unsafe { smudge_core::hot::note_kept_types(probes.iter().copied().collect()) }
}

/// Reads the signal a build stored in `slot`.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
fn read<T: Clone + 'static>(slot: &Slot<T>) -> T {
    slot.borrow()
        .as_ref()
        .expect("the build stored its signal")
        .read(T::clone)
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_remount_after_a_patch_keeps_a_signal_created_inside_the_build() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let build = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build.clone());
    assert_eq!(read(&slot), 7, "the new build's signal took the old value");

    // A remount no patch came before starts the signal fresh, as a reset
    // of the interface expects.
    harness.app().remount(build);
    assert_eq!(read(&slot), 1, "a remount without a patch carried a value");
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_remount_resets_a_signal_whose_type_changed() {
    let before: Slot<u32> = Rc::new(RefCell::new(None));
    let after: Slot<i32> = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let before = Rc::clone(&before);
        move || {
            *before.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    });
    harness.frame();
    before.borrow().as_ref().expect("a signal").set(7);

    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<i32>(),
    ]);
    harness.app().remount({
        let after = Rc::clone(&after);
        move || {
            *after.borrow_mut() = Some(RwSignal::new(-1i32));
            square()
        }
    });
    assert_eq!(read(&after), -1, "a signal of another type took the value");
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_remount_resets_a_signal_whose_layout_the_patch_changed() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let build = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    // The patch listed no probe, so no type kept a layout it could vouch for.
    keep(&[]);
    harness.app().remount(build);
    assert_eq!(
        read(&slot),
        1,
        "a value of a type the patch did not keep was carried"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_remount_resets_a_signal_whose_call_site_moved_to_another_owner() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    });
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    // The owner lives as long as the test, as a node's owner lives as long
    // as its node.
    let inner: Rc<RefCell<Option<smudge_core::reactive::Owner>>> = Rc::new(RefCell::new(None));
    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount({
        let slot = Rc::clone(&slot);
        let inner = Rc::clone(&inner);
        move || {
            let owner = smudge_core::reactive::Owner::new();
            *slot.borrow_mut() = Some(owner.with(|| RwSignal::new(1u32)));
            *inner.borrow_mut() = Some(owner);
            square()
        }
    });
    assert_eq!(
        read(&slot),
        1,
        "a signal under another owner took the value"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_remount_finds_a_list_rows_value_after_the_rows_moved() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let items = RwSignal::new(vec!["a", "b"]);
    let build = {
        let rows = Rc::clone(&rows);
        let items = items.clone();
        move || {
            let rows = Rc::clone(&rows);
            let items = items.clone();
            panel().child(for_each(
                move || items.get(),
                |name| *name,
                move |name| {
                    rows.borrow_mut().insert(name, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(1);
    rows.borrow()["b"].set(2);
    // The rows keep their owners when they move, and the new build makes
    // them in the new order.
    items.set(vec!["b", "a"]);
    harness.frame();

    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<&'static str>(),
    ]);
    harness.app().remount(build);
    assert_eq!(read_row(&rows, "a"), 1, "row a lost its value");
    assert_eq!(read_row(&rows, "b"), 2, "row b lost its value");
}

/// Reads the value of the row `name` of a list a build made.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
fn read_row(
    rows: &Rc<RefCell<std::collections::HashMap<&'static str, RwSignal<u32>>>>,
    name: &str,
) -> u32 {
    rows.borrow()[name].read(|value| *value)
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_remount_keeps_a_value_of_a_branch_that_was_shown_again() {
    use smudge_core::element::show;

    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let shown = RwSignal::new(true);
    let build = {
        let slot = Rc::clone(&slot);
        let shown = shown.clone();
        move || {
            let slot = Rc::clone(&slot);
            let shown = shown.clone();
            panel().child(show(
                move || shown.get(),
                move || {
                    *slot.borrow_mut() = Some(RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    shown.set(false);
    harness.frame();
    shown.set(true);
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(5);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    assert_eq!(read(&slot), 5, "the branch shown again lost its value");
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_handle_kept_from_the_old_build_names_the_carried_signal() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let build = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    let kept = slot.borrow().clone().expect("a signal");
    kept.set(7);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    assert_eq!(
        kept.read(|value| *value),
        7,
        "the old handle lost the value"
    );
    kept.set(9);
    assert_eq!(
        read(&slot),
        9,
        "the old handle writes another signal than the new one"
    );
}

/// A row key whose every value hashes alike, as a lawful hash may.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[derive(Clone, PartialEq, Eq)]
struct Colliding(&'static str);

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl std::hash::Hash for Colliding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u8(0);
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn rows_whose_keys_hash_alike_carry_nothing() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let build = {
        let rows = Rc::clone(&rows);
        move || {
            let rows = Rc::clone(&rows);
            panel().child(for_each(
                || vec![Colliding("a"), Colliding("b")],
                Colliding::clone,
                move |key| {
                    rows.borrow_mut().insert(key.0, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(1);
    rows.borrow()["b"].set(2);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(
        read_row(&rows, "a"),
        0,
        "a row took a value it cannot be told to own"
    );
    assert_eq!(
        read_row(&rows, "b"),
        0,
        "a row took a value it cannot be told to own"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_signal_keeps_its_value_across_two_patches() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let build = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    for patch in 1..=2 {
        keep(&[smudge_core::hot::probe_of::<u32>()]);
        harness.app().remount(build.clone());
        harness.frame();
        assert_eq!(read(&slot), 7, "patch {patch} lost the value");
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_signal_a_handler_creates_after_a_patch_takes_no_old_value() {
    let made: Rc<RefCell<Vec<RwSignal<u32>>>> = Rc::new(RefCell::new(Vec::new()));
    let build = {
        let made = Rc::clone(&made);
        move || {
            let made = Rc::clone(&made);
            square().on_press(move || {
                let next = 100 + made.borrow().len() as u32;
                made.borrow_mut().push(RwSignal::new(next));
            })
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    harness.click(in_square(0));
    harness.click(in_square(0));

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    harness.click(in_square(0));
    let values: Vec<u32> = made
        .borrow()
        .iter()
        .map(|signal| signal.read(|value| *value))
        .collect();
    assert_eq!(
        values,
        [100, 101, 102],
        "a handler's signal took an old value"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn what_a_row_shows_follows_its_row_when_a_row_comes_before_it() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let items = RwSignal::new(vec!["a", "b"]);
    let build = {
        let rows = Rc::clone(&rows);
        let items = items.clone();
        move || {
            let rows = Rc::clone(&rows);
            let items = items.clone();
            panel().child(for_each(
                move || items.get(),
                |name| *name,
                move |name| {
                    let rows = Rc::clone(&rows);
                    panel().child(show(
                        || true,
                        move || {
                            rows.borrow_mut().insert(name, RwSignal::new(9u32));
                            square()
                        },
                    ))
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(1);
    rows.borrow()["b"].set(2);
    items.set(vec!["x", "a", "b"]);
    harness.frame();

    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<&'static str>(),
    ]);
    harness.app().remount(build);
    harness.frame();
    let values: Vec<u32> = ["x", "a", "b"]
        .iter()
        .map(|name| read_row(&rows, name))
        .collect();
    assert_eq!(
        values,
        [9, 1, 2],
        "a row's shown branch took another row's value"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_row_whose_key_hashes_like_a_new_one_takes_nothing() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let items = RwSignal::new(vec![Colliding("a")]);
    let build = {
        let rows = Rc::clone(&rows);
        let items = items.clone();
        move || {
            let rows = Rc::clone(&rows);
            let items = items.clone();
            panel().child(for_each(
                move || items.get(),
                Colliding::clone,
                move |key| {
                    rows.borrow_mut().insert(key.0, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(1);
    // The new key hashes like the old one and comes first.
    items.set(vec![Colliding("b"), Colliding("a")]);
    harness.frame();

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(read_row(&rows, "b"), 0, "a new row took an old row's value");
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_row_whose_key_came_to_hash_like_another_carries_nothing() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let items = RwSignal::new(vec![Colliding("a")]);
    let build = {
        let rows = Rc::clone(&rows);
        let items = items.clone();
        move || {
            let rows = Rc::clone(&rows);
            let items = items.clone();
            panel().child(for_each(
                move || items.get(),
                Colliding::clone,
                move |key| {
                    rows.borrow_mut().insert(key.0, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(7);
    // A key that hashes like the mounted one joins the list.
    items.set(vec![Colliding("b"), Colliding("a")]);
    harness.frame();
    // The new build shows only the newcomer, which no key shares a hash
    // with any more.
    items.set(vec![Colliding("b")]);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(
        read_row(&rows, "b"),
        0,
        "a row took the value of the row it collided with"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_signal_an_effect_creates_while_the_build_mounts_takes_no_value() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let effects: Rc<RefCell<Vec<RenderEffect>>> = Rc::new(RefCell::new(Vec::new()));
    let build = {
        let slot = Rc::clone(&slot);
        let effects = Rc::clone(&effects);
        move || {
            let slot = Rc::clone(&slot);
            effects.borrow_mut().push(RenderEffect::new(move || {
                *slot.borrow_mut() = Some(RwSignal::new(1u32));
            }));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(read(&slot), 1, "an effect's signal took a carried value");
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn an_app_dropped_on_the_thread_leaves_another_apps_offer_open() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    /// A list that shows its one row only while `ready` holds.
    fn app(
        rows: &Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>>,
        ready: &RwSignal<bool>,
    ) -> impl Fn() -> Element<Panel> + Clone + 'static {
        let rows = Rc::clone(rows);
        let ready = ready.clone();
        move || {
            let rows = Rc::clone(&rows);
            let ready = ready.clone();
            panel().child(for_each(
                move || if ready.get() { vec!["a"] } else { Vec::new() },
                |name| *name,
                move |name| {
                    rows.borrow_mut().insert(name, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    }

    let first_rows = Rc::new(RefCell::new(HashMap::new()));
    let second_rows = Rc::new(RefCell::new(HashMap::new()));
    let first_ready = RwSignal::new(true);
    let second_ready = RwSignal::new(true);
    let mut first = Harness::new(100.0, 60.0, 1.0, app(&first_rows, &first_ready));
    let mut second = Harness::new(100.0, 60.0, 1.0, app(&second_rows, &second_ready));
    first.frame();
    second.frame();
    second_rows.borrow()["a"].set(5);

    // The row of the second app mounts only frames after its remount.
    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<&'static str>(),
    ]);
    first.app().remount(app(&first_rows, &first_ready));
    second_ready.set(false);
    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<&'static str>(),
    ]);
    second.app().remount(app(&second_rows, &second_ready));
    drop(first);
    second_ready.set(true);
    second.frame();
    assert_eq!(
        read_row(&second_rows, "a"),
        5,
        "the other app's offer ended this one"
    );
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn an_effect_that_outlives_the_build_still_hears_a_carried_signal() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let build = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    let held = slot.borrow().clone().expect("a signal");
    // An effect outside the build, as a configure step installs, reads the
    // build's signal through a handle it keeps.
    let seen = Rc::new(Cell::new(0u32));
    let outside = smudge_core::reactive::Owner::new();
    let _effect = {
        let held = held.clone();
        let seen = Rc::clone(&seen);
        outside.with(|| RenderEffect::new(move || seen.set(held.get())))
    };
    smudge_core::reactive::flush();
    held.set(7);
    smudge_core::reactive::flush();

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(9);
    smudge_core::reactive::flush();
    assert_eq!(
        seen.get(),
        9,
        "the outliving effect no longer hears the signal"
    );
}

/// A value whose drop creates a signal of the type the build's next signal
/// has.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
struct MakesASignal;

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl Drop for MakesASignal {
    fn drop(&mut self) {
        let _ = RwSignal::new(0u32);
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_dropped_initial_value_takes_no_place_among_the_builds_signals() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let maker: Slot<MakesASignal> = Rc::new(RefCell::new(None));
    let build = {
        let slot = Rc::clone(&slot);
        let maker = Rc::clone(&maker);
        move || {
            *maker.borrow_mut() = Some(RwSignal::new(MakesASignal));
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<MakesASignal>(),
    ]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(
        read_u32(&slot),
        7,
        "the dropped value's signal took the build's place"
    );
}

/// Reads a `u32` signal a build stored in `slot`.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
fn read_u32(slot: &Slot<u32>) -> u32 {
    slot.borrow()
        .as_ref()
        .expect("the build stored its signal")
        .read(|value| *value)
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_row_of_an_unequal_key_that_hashes_alike_takes_nothing() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let items = RwSignal::new(vec![Colliding("a")]);
    let build = {
        let rows = Rc::clone(&rows);
        let items = items.clone();
        move || {
            let rows = Rc::clone(&rows);
            let items = items.clone();
            panel().child(for_each(
                move || items.get(),
                Colliding::clone,
                move |key| {
                    rows.borrow_mut().insert(key.0, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(7);
    // The new build shows another key that hashes alike, alone.
    items.set(vec![Colliding("b")]);

    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<Colliding>(),
    ]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(
        read_row(&rows, "b"),
        0,
        "an unequal key's row took the value"
    );
}

/// A row key whose comparison creates a signal, as a key's own code may.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[derive(Clone, Eq)]
struct ComparesWithASignal(&'static str);

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl std::hash::Hash for ComparesWithASignal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl PartialEq for ComparesWithASignal {
    fn eq(&self, other: &Self) -> bool {
        let _ = RwSignal::new(0u8);
        self.0 == other.0
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_row_key_whose_comparison_creates_a_signal_still_carries() {
    use smudge_core::element::for_each;
    use std::collections::HashMap;

    let rows: Rc<RefCell<HashMap<&'static str, RwSignal<u32>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let build = {
        let rows = Rc::clone(&rows);
        move || {
            let rows = Rc::clone(&rows);
            panel().child(for_each(
                || vec![ComparesWithASignal("a")],
                ComparesWithASignal::clone,
                move |key| {
                    rows.borrow_mut().insert(key.0, RwSignal::new(0u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    rows.borrow()["a"].set(7);

    for patch in 1..=2 {
        keep(&[
            smudge_core::hot::probe_of::<u32>(),
            smudge_core::hot::probe_of::<u8>(),
            smudge_core::hot::probe_of::<ComparesWithASignal>(),
        ]);
        harness.app().remount(build.clone());
        harness.frame();
        assert_eq!(
            read_row(&rows, "a"),
            7,
            "patch {patch} lost the row's value"
        );
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_handler_another_apps_build_runs_takes_no_value() {
    let made: Rc<RefCell<Vec<RwSignal<u32>>>> = Rc::new(RefCell::new(Vec::new()));
    let build = {
        let made = Rc::clone(&made);
        move || {
            let made = Rc::clone(&made);
            square().on_press(move || {
                let next = 100 + made.borrow().len() as u32;
                made.borrow_mut().push(RwSignal::new(next));
            })
        }
    };
    let pressed = Rc::new(RefCell::new(Harness::new(100.0, 60.0, 1.0, build.clone())));
    pressed.borrow_mut().frame();
    pressed.borrow_mut().click(in_square(0));
    keep(&[smudge_core::hot::probe_of::<u32>()]);
    pressed.borrow_mut().app().remount(build);

    // Another app's build presses the first app's square while the first
    // app's offer is open.
    let _other = Harness::new(100.0, 60.0, 1.0, {
        let pressed = Rc::clone(&pressed);
        move || {
            pressed.borrow_mut().click(in_square(0));
            square()
        }
    });
    let values: Vec<u32> = made
        .borrow()
        .iter()
        .map(|signal| signal.read(|value| *value))
        .collect();
    assert_eq!(values, [100, 101], "a handler's signal took an old value");
}

/// Where a drop keeps the app it made and the signal that app's build made.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
type MadeApp = Rc<RefCell<Option<(Harness, Slot<u32>)>>>;

/// A value whose drop makes an app, once, and keeps it with the signal its
/// build made.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
struct MakesAnApp(MadeApp);

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl Drop for MakesAnApp {
    fn drop(&mut self) {
        if self.0.borrow().is_some() {
            return;
        }
        let slot: Slot<u32> = Rc::new(RefCell::new(None));
        let harness = Harness::new(100.0, 60.0, 1.0, {
            let slot = Rc::clone(&slot);
            move || {
                *slot.borrow_mut() = Some(RwSignal::new(1u32));
                square()
            }
        });
        *self.0.borrow_mut() = Some((harness, slot));
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn an_app_made_while_a_value_is_dropped_still_carries_its_values() {
    let made: MadeApp = Rc::new(RefCell::new(None));
    let maker: Slot<MakesAnApp> = Rc::new(RefCell::new(None));
    let build = {
        let made = Rc::clone(&made);
        let maker = Rc::clone(&maker);
        move || {
            *maker.borrow_mut() = Some(RwSignal::new(MakesAnApp(Rc::clone(&made))));
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    // The remount drops the new build's initial value, whose drop makes the
    // other app.
    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<MakesAnApp>(),
    ]);
    harness.app().remount(build);
    let (mut other, slot) = made.borrow_mut().take().expect("the drop made an app");
    other.frame();
    slot.borrow().as_ref().expect("a signal").set(42);
    let rebuild = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(1u32));
            square()
        }
    };
    keep(&[smudge_core::hot::probe_of::<u32>()]);
    other.app().remount(rebuild);
    assert_eq!(read_u32(&slot), 42, "the app made in a drop lost its value");
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_signal_beside_a_passing_owner_keeps_its_value() {
    let slot: Slot<u32> = Rc::new(RefCell::new(None));
    let kept_owners: Rc<RefCell<Vec<smudge_core::reactive::Owner>>> =
        Rc::new(RefCell::new(Vec::new()));
    let build = {
        let slot = Rc::clone(&slot);
        let kept_owners = Rc::clone(&kept_owners);
        move || {
            let holder = smudge_core::reactive::Owner::new();
            let lasting = holder.with(|| {
                // An owner that lives only while the next one is made.
                let passing = smudge_core::reactive::Owner::new();
                let lasting = smudge_core::reactive::Owner::new();
                *slot.borrow_mut() = Some(lasting.with(|| RwSignal::new(1u32)));
                drop(passing);
                lasting
            });
            // The owners live as long as the build, as a node's owner lives
            // as long as its node.
            *kept_owners.borrow_mut() = vec![holder, lasting];
            square()
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    slot.borrow().as_ref().expect("a signal").set(7);

    keep(&[smudge_core::hot::probe_of::<u32>()]);
    harness.app().remount(build);
    assert_eq!(
        read_u32(&slot),
        7,
        "the passing owner moved the signal's place"
    );
}

/// A row key whose comparison creates a signal of the type its row's own
/// signals have.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[derive(Clone, Eq)]
struct ComparesWithAU32(&'static str);

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl PartialEq for ComparesWithAU32 {
    fn eq(&self, other: &Self) -> bool {
        let _ = RwSignal::new(0u32);
        self.0 == other.0
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl std::hash::Hash for ComparesWithAU32 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[test]
fn a_row_key_comparison_takes_no_value_of_its_rows_signals() {
    use smudge_core::element::for_each;

    let first: Slot<u32> = Rc::new(RefCell::new(None));
    let second: Slot<u32> = Rc::new(RefCell::new(None));
    let build = {
        let first = Rc::clone(&first);
        let second = Rc::clone(&second);
        move || {
            let first = Rc::clone(&first);
            let second = Rc::clone(&second);
            panel().child(for_each(
                || vec![ComparesWithAU32("a")],
                ComparesWithAU32::clone,
                move |_| {
                    *first.borrow_mut() = Some(RwSignal::new(1u32));
                    *second.borrow_mut() = Some(RwSignal::new(2u32));
                    square()
                },
            ))
        }
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, build.clone());
    harness.frame();
    first.borrow().as_ref().expect("a signal").set(7);
    second.borrow().as_ref().expect("a signal").set(8);

    keep(&[
        smudge_core::hot::probe_of::<u32>(),
        smudge_core::hot::probe_of::<ComparesWithAU32>(),
    ]);
    harness.app().remount(build);
    harness.frame();
    assert_eq!(
        [read_u32(&first), read_u32(&second)],
        [7, 8],
        "the comparison's signal took a row's value"
    );
}

#[test]
fn a_remount_reports_whether_the_focus_came_back() {
    let log = FocusLog::default();
    let first = log.clone();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || focus_row(&first, "old"));
    harness.frame();
    harness.click(in_square(1));
    harness.frame();

    let again = log.clone();
    let same = harness.app().remount(move || focus_row(&again, "new"));
    assert!(
        same.focus_restored,
        "the new build has the focused square in the same place"
    );

    let gone = harness.app().remount(|| panel().child(square()));
    assert!(!gone.focus_restored, "the new build has no node there");
    assert_eq!(
        (gone.signals_kept, gone.signals_left),
        (0, 0),
        "no patch, so nothing carries"
    );
}
