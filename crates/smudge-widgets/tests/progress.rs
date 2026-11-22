//! Progress bars and spinners: what a screen reader hears, the damage one
//! tick of either costs, how a control unmounts its clock, and how
//! reduced motion stills them.

use std::time::Duration;

use smudge_core::element::{IntoElement, show};
use smudge_core::geometry::Rect;
use smudge_core::geometry::Size;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode, ValueRange};
use smudge_core::style::{Align, Length, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_test::{FrameReport, Harness};
use smudge_text::{TextStyle, TextSystem, label, provide_text_style};
use smudge_widgets::{column, progress, row, spinner};

/// One tick of an animated control, one frame of a 60-hertz display.
const TICK: Duration = Duration::from_millis(16);

/// A step that crosses no phase of the reduced-motion pulse, which flips
/// every 600 milliseconds.
const STILL_STEP: Duration = Duration::from_millis(200);

/// Returns the window's only projected control, a progress bar or a
/// spinner.
fn control(harness: &mut Harness) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::ProgressBar)
        .unwrap_or_else(|| panic!("the window shows a progress bar"))
}

/// Returns the rectangle `node` paints in, in the physical pixels of a
/// window whose scale is one.
fn painted(node: &SemanticsNode) -> Rect {
    Rect::new(
        node.bounds.x0,
        node.bounds.y0,
        node.bounds.x1,
        node.bounds.y1,
    )
}

/// Returns `rect` grown `by` pixels each way, for the antialiasing of an
/// edge and the slack a stroke's bounds carry.
fn grown(rect: Rect, by: f32) -> Rect {
    Rect::new(rect.x0 - by, rect.y0 - by, rect.x1 + by, rect.y1 + by)
}

/// Returns whether every damaged rectangle of `report` lies within
/// `bounds`.
fn within(report: &FrameReport, bounds: Rect) -> bool {
    !report.damage.is_empty()
        && report.damage.iter().all(|rect| {
            bounds.x0 <= rect.x as f32
                && bounds.y0 <= rect.y as f32
                && bounds.x1 >= (rect.x + rect.width) as f32
                && bounds.y1 >= (rect.y + rect.height) as f32
        })
}

/// A window of 200 by 100 that shows `build`'s element at its top left.
fn window<E: IntoElement + 'static>(build: impl Fn() -> E + 'static) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().align_items(Align::Start).child(build())
    });
    harness.frame();
    harness
}

#[test]
fn a_progress_bar_hears_its_share_and_its_busy_state() {
    let done = RwSignal::new(Some(0.25_f32));
    let value = done.clone();
    let mut harness = window(move || {
        let read = value.clone();
        progress(move || read.get()).width(px(160.0))
    });

    let bar = control(&mut harness);
    assert_eq!(
        bar.range,
        Some(ValueRange {
            value: 0.25,
            min: 0.0,
            max: 1.0,
        }),
        "the share is the range a screen reader hears"
    );

    done.set(Some(0.75));
    harness.frame();
    let bar = control(&mut harness);
    assert_eq!(
        bar.range,
        Some(ValueRange {
            value: 0.75,
            min: 0.0,
            max: 1.0,
        }),
        "a write of the share moves the range"
    );

    done.set(None);
    harness.frame();
    let bar = control(&mut harness);
    assert_eq!(bar.range, None, "no share reads as busy");
}

#[test]
fn a_bar_of_a_known_share_keeps_no_frames_coming() {
    let mut harness = window(|| progress(0.5).width(px(160.0)));
    harness.settle();
    let before = harness.png();

    let report = harness.advance(TICK * 4);

    assert!(
        !harness.app().is_animating(),
        "a bar with a share shows no motion, so the window may go idle"
    );
    assert!(
        report.damage.is_empty() && report.nodes_painted == 0,
        "a bar with a share shows a still picture, so a frame costs no paint"
    );
    assert_eq!(before, harness.png());
}

#[test]
fn a_share_write_repaints_the_bar_alone_without_layout() {
    let done = RwSignal::new(0.5_f32);
    let value = done.clone();
    let mut harness = window(move || {
        let read = value.clone();
        progress(move || Some(read.get())).width(px(160.0))
    });
    let bar = grown(painted(&control(&mut harness)), 1.0);

    done.set(0.9);
    let report = harness.frame();

    assert!(
        !report.layout_ran,
        "a share write changes no size, so it never lays out"
    );
    assert_eq!(report.nodes_painted, 1, "only the bar repaints");
    assert!(
        within(&report, bar),
        "the damage stays inside the bar at {bar:?}: {:?}",
        report.damage
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn an_unknown_length_moves_a_step_a_frame_alone() {
    let mut harness = window(|| progress(None).width(px(160.0)));
    let bar = grown(painted(&control(&mut harness)), 1.0);
    let before = harness.png();

    let report = harness.advance(TICK);

    assert!(
        harness.app().is_animating(),
        "a stripe that rides the frame's time keeps the frames coming"
    );

    assert!(
        !report.layout_ran,
        "a stripe that moves changes no size, so it never lays out"
    );
    assert_eq!(report.nodes_painted, 1, "only the bar repaints");
    assert!(
        within(&report, bar),
        "the damage stays inside the bar at {bar:?}: {:?}",
        report.damage
    );
    assert_ne!(before, harness.png(), "the stripe moved");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_turning_spinner_costs_one_node_a_frame() {
    let mut harness = window(spinner);
    let arc = grown(painted(&control(&mut harness)), 2.0);
    let before = harness.png();

    let report = harness.advance(TICK);

    assert!(
        harness.app().is_animating(),
        "a turning arc keeps the frames coming"
    );

    assert_eq!(report.nodes_painted, 1, "only the spinner repaints");
    assert!(
        within(&report, arc),
        "the damage stays inside the spinner at {arc:?}: {:?}",
        report.damage
    );
    assert_ne!(before, harness.png(), "the arc turned");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_hidden_spinner_wakes_no_frame() {
    let mut harness = window(|| spinner().hidden(true));

    assert!(
        harness.app().next_deadline().is_none(),
        "a hidden spinner schedules no timer"
    );
    assert!(!harness.app().is_animating(), "and it asks for no frames");

    let report = harness.advance(TICK);
    assert!(
        report.damage.is_empty() && report.nodes_painted == 0,
        "so time passing wakes nothing"
    );
}

#[test]
fn an_unmounted_control_wakes_no_frame() {
    let visible = RwSignal::new(true);
    let mut harness = window({
        let shown = visible.clone();
        move || {
            let shown = shown.clone();
            show(
                move || shown.get(),
                || column().child(progress(None)).child(spinner()),
            )
        }
    });
    assert!(
        harness.app().next_deadline().is_none(),
        "with motion at full the window paces the controls, so they arm no timer"
    );

    harness.app().set_reduced_motion(true);
    harness.frame();
    assert!(
        harness.app().next_deadline().is_some(),
        "under reduced motion the pulse keeps its own clock"
    );

    visible.set(false);
    harness.frame();
    for _ in 0..20 {
        if harness.app().next_deadline().is_none() {
            break;
        }
        harness.advance(Duration::from_millis(100));
    }
    assert!(
        harness.app().next_deadline().is_none(),
        "the unmounted controls left no timer behind"
    );
    assert!(
        !harness.app().is_animating(),
        "and they ask for no more frames"
    );
}

#[test]
fn restoring_motion_moves_a_spinner_reduced_motion_stilled() {
    let mut harness = window(spinner);
    harness.app().set_reduced_motion(true);
    harness.frame();
    assert!(
        harness.app().next_deadline().is_some(),
        "the reduced pulse keeps its own clock"
    );

    harness.app().set_reduced_motion(false);
    let report = harness.advance(TICK);

    assert!(
        within(&report, grown(painted(&control(&mut harness)), 2.0)),
        "the arc turns again once motion is back"
    );
    assert!(
        harness.app().next_deadline().is_none(),
        "the pulse retired with the motion"
    );
}

#[test]
fn reduced_motion_stills_a_spinner() {
    let mut harness = window(spinner);
    harness.app().set_reduced_motion(true);
    harness.frame();
    let before = harness.png();

    let report = harness.advance(STILL_STEP);

    assert!(
        report.damage.is_empty() && report.nodes_painted == 0,
        "a still spinner wakes no paint between pulse phases"
    );
    assert_eq!(before, harness.png(), "the arc kept its angle");
}

#[test]
fn reduced_motion_stills_a_bar_of_unknown_length() {
    let mut harness = window(|| progress(None).width(px(160.0)));
    harness.app().set_reduced_motion(true);
    harness.frame();
    let before = harness.png();

    let report = harness.advance(STILL_STEP);

    assert!(
        report.damage.is_empty() && report.nodes_painted == 0,
        "a still stripe wakes no paint between pulse phases"
    );
    assert_eq!(before, harness.png(), "the stripe kept its place");
}

#[test]
fn a_spinner_measures_a_line_of_the_text_around_it() {
    TextSystem::bundled_only().install();
    let mut small = Harness::new(200.0, 100.0, 1.0, || {
        provide_text_style(TextStyle {
            size: 12.0,
            ..TextStyle::default()
        });
        panel().align_items(Align::Start).child(spinner())
    });
    let mut large = Harness::new(200.0, 100.0, 1.0, || {
        provide_text_style(TextStyle {
            size: 32.0,
            ..TextStyle::default()
        });
        panel().align_items(Align::Start).child(spinner())
    });
    small.frame();
    large.frame();

    let small = control(&mut small).bounds;
    let large = control(&mut large).bounds;
    assert!(
        (small.x1 - small.x0 - (small.y1 - small.y0)).abs() < 0.5,
        "a spinner is square: {small:?}"
    );
    assert!(
        large.x1 - large.x0 > small.x1 - small.x0,
        "a bigger text sizes a bigger spinner"
    );
}

/// A window that shows the states a progress bar and a spinner take, in
/// `theme`, with the clock run to a fixed moment so the moving states sit
/// where the picture says.
fn states_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(240.0, 220.0, 1.0, || {
        panel().child(
            column()
                .width(Length::pct(100.0))
                .padding(px(12.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
                .align_items(Align::Start)
                .child(progress(0.0).width(Length::pct(100.0)))
                .child(progress(0.5).width(Length::pct(100.0)))
                .child(progress(1.0).width(Length::pct(100.0)))
                .child(
                    progress(None)
                        .width(Length::pct(100.0))
                        .accessible_label("Working"),
                )
                .child(
                    progress(0.5)
                        .width(Length::pct(100.0))
                        .disabled(true)
                        .accessible_label("Stopped"),
                )
                .child(
                    row()
                        .gap(Size {
                            width: 8.0,
                            height: 0.0,
                        })
                        .child(spinner().accessible_label("Saving"))
                        .child(label("Saving")),
                ),
        )
    });
    harness.app().set_theme(theme);
    harness.frame();
    // Nine ticks of 125 milliseconds land the clock at 1.125 seconds:
    // the stripe crosses mid-track and the arc sits mid-turn, moments a
    // frozen control could not match, one tick at a time because one call
    // fires a timer once however far it spans.
    for _ in 0..9 {
        harness.advance(Duration::from_millis(125));
    }
    harness
}

#[test]
fn the_states_of_progress_bars_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::light()).png());
}

#[test]
fn the_states_of_progress_bars_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::dark()).png());
}

/// A window that shows what reduced motion leaves of a busy bar and a
/// spinner, in `theme`, past the first flip of the pulse so the picture
/// holds the dim phase.
fn still_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(240.0, 80.0, 1.0, || {
        panel().child(
            column()
                .width(Length::pct(100.0))
                .padding(px(12.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
                .align_items(Align::Start)
                .child(progress(None).width(Length::pct(100.0)))
                .child(spinner()),
        )
    });
    harness.app().set_theme(theme);
    harness.app().set_reduced_motion(true);
    harness.frame();
    for _ in 0..6 {
        harness.advance(Duration::from_millis(120));
    }
    harness
}

#[test]
fn reduced_motion_leaves_a_pulse_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", still_window(Theme::light()).png());
}

#[test]
fn reduced_motion_leaves_a_pulse_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", still_window(Theme::dark()).png());
}
