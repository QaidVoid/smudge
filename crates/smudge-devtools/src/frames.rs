//! The frames tab: what the app's frames cost, a timeline of the last ones,
//! and what each stage of a frame took.

use std::time::Duration;

use smudge_core::element::Element;
use smudge_core::geometry::Size;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Align, Justify, px};
use smudge_core::theme::ColorValue;
use smudge_core::tree::Dirty;
use smudge_core::widget::Panel;
use smudge_text::{LabelExt, label};
use smudge_widgets::{column, row, spacer};

use crate::Tools;
use crate::look::{self, card, gap, heading, mono, note};
use crate::timeline::{
    BAR, BUDGET, FRAMES, FrameRecord, HEIGHT, STAGE_COLORS, STAGE_NAMES, Summary, Timeline,
    TimelineView,
};

/// The width of the timeline, in logical pixels: a bar for each frame it
/// keeps.
const TIMELINE_WIDTH: f32 = FRAMES as f32 * BAR;

/// Writes `cost` in milliseconds.
fn millis(cost: Duration) -> String {
    format!("{:.2} ms", cost.as_secs_f64() * 1000.0)
}

/// The color a frame's cost shows in: plain within the budget of a frame
/// at 60 frames a second, red past it.
fn cost_color(cost: Duration) -> ColorValue {
    ColorValue::from(if cost > BUDGET { look::BAD } else { look::TEXT })
}

/// The marker of the timeline's frames, which [`Element::bind`] keys on.
struct Frames;

/// The marker of where the pointer is over the timeline, which
/// [`Element::bind`] keys on.
struct Hovered;

/// Builds the frames tab.
pub(crate) fn frames_tab(tools: &Tools) -> Element<Panel> {
    let hovered: RwSignal<Option<f32>> = RwSignal::new(None);
    let idle = tools.timeline.clone();
    look::scrolling(
        look::tab_content()
            .child(look::when(
                move || idle.with(|timeline| timeline.summary().frames == 0),
                || {
                    card()
                        .border_edges(smudge_core::style::Edges {
                            top: 0.0,
                            right: 0.0,
                            bottom: 0.0,
                            left: 3.0,
                        })
                        .border_color(look::ACCENT)
                        .child(heading("The app has not drawn yet"))
                        .child(note(
                            "The panel records the frames the app draws while the panel is \
                             open, and an app that is still draws none. Hover, type or scroll \
                             in it to see what its frames cost.",
                        ))
                },
            ))
            .child(stats(&tools.timeline))
            .child(timeline(&tools.timeline, &hovered))
            .child(breakdown(&tools.timeline, &hovered)),
    )
}

/// Builds the row of tiles that sum up the app's frames: the newest, the
/// average, the slowest, and how many missed the budget.
fn stats(timeline: &RwSignal<std::rc::Rc<Timeline>>) -> Element<Panel> {
    let tile =
        |name: &'static str, value: fn(&Summary) -> String, color: fn(&Summary) -> ColorValue| {
            let (text, paint) = (timeline.clone(), timeline.clone());
            column()
                .grow(1.0)
                .basis(px(0.0))
                .padding_xy(Size {
                    width: 8.0,
                    height: 6.0,
                })
                .background(look::RAISED)
                .corner_radius(8.0)
                .child(note(name))
                .child(
                    mono(move || {
                        let summary = text.with(|timeline| timeline.summary());
                        if summary.frames == 0 {
                            String::from("idle")
                        } else {
                            value(&summary)
                        }
                    })
                    .text_size(look::BODY + 1.0)
                    .text_color(move || {
                        let summary = paint.with(|timeline| timeline.summary());
                        if summary.frames == 0 {
                            ColorValue::from(look::FAINT)
                        } else {
                            color(&summary)
                        }
                    }),
                )
        };
    row()
        .gap(gap(6.0))
        .child(tile(
            "Last",
            |summary| millis(summary.last),
            |summary| cost_color(summary.last),
        ))
        .child(tile(
            "Average",
            |summary| millis(summary.average),
            |summary| cost_color(summary.average),
        ))
        .child(tile(
            "Slowest",
            |summary| millis(summary.slowest),
            |summary| cost_color(summary.slowest),
        ))
        .child(tile(
            "Over budget",
            |summary| format!("{} of {}", summary.over, summary.frames),
            |summary| {
                ColorValue::from(if summary.over == 0 {
                    look::GOOD
                } else {
                    look::WARN
                })
            },
        ))
}

/// Builds the timeline's card: the bars, with the times their heights stand
/// for at their left, and a line that says how to read them.
fn timeline(
    timeline: &RwSignal<std::rc::Rc<Timeline>>,
    hovered: &RwSignal<Option<f32>>,
) -> Element<Panel> {
    let frames = timeline.clone();
    let (over, left, marked) = (hovered.clone(), hovered.clone(), hovered.clone());
    let view = Element::new(TimelineView::default())
        .width(px(TIMELINE_WIDTH))
        .height(px(HEIGHT))
        .shrink(0.0)
        .background(look::SUNKEN)
        .corner_radius(4.0)
        .bind(
            Frames,
            move || frames.get(),
            |view, timeline| {
                view.timeline.copy_from(&timeline);
                Dirty::Paint
            },
        )
        .bind(
            Hovered,
            move || marked.get(),
            |view, hovered| {
                view.hovered = hovered;
                Dirty::Paint
            },
        )
        .on_pointer_move(move |point| over.set(Some(point.x)))
        .on_hover(move |inside| {
            if !inside {
                left.set(None);
            }
        });
    let budget = BUDGET.as_secs_f64() * 1000.0;
    let axis = column()
        .height(px(HEIGHT))
        .grow(1.0)
        .align_items(Align::End)
        .justify_content(Justify::SpaceBetween)
        .child(mono(format!("{:.1} ms", 2.0 * budget)).text_color(look::FAINT))
        .child(mono(format!("{budget:.1} ms")).text_color(look::MUTED))
        .child(mono("0 ms").text_color(look::FAINT));
    card()
        .child(
            row()
                .align_items(Align::Center)
                .child(heading("Timeline"))
                .child(spacer())
                .child(note(format!("last {FRAMES} frames"))),
        )
        .child(
            row()
                .gap(gap(8.0))
                .align_items(Align::Center)
                .child(axis)
                .child(view),
        )
        .child(note(
            "Each bar is a frame, newest at the right, stacked by stage. A bar past the \
             dashed line missed 60 frames a second, and a gap is a frame that presented \
             nothing.",
        ))
}

/// Builds the breakdown's card: what each stage of the hovered frame took,
/// or of the average frame while no bar is hovered, in the colors the bars
/// stack them in.
fn breakdown(
    timeline: &RwSignal<std::rc::Rc<Timeline>>,
    hovered: &RwSignal<Option<f32>>,
) -> Element<Panel> {
    let frame = {
        let (timeline, hovered) = (timeline.clone(), hovered.clone());
        move || -> Option<FrameRecord> {
            let x = hovered.get()?;
            timeline.with(|timeline| timeline.at(x, TIMELINE_WIDTH).copied())
        }
    };
    let stages = {
        let (timeline, frame) = (timeline.clone(), frame.clone());
        move || -> [Duration; 8] {
            frame().map_or_else(
                || timeline.with(|timeline| timeline.summary().stages),
                |frame| frame.stages(),
            )
        }
    };
    let title = {
        let frame = frame.clone();
        move || {
            String::from(match frame() {
                Some(frame) if frame.presented => "Hovered frame",
                Some(_) => "Hovered frame, which presented nothing",
                None => "Average frame",
            })
        }
    };
    let total = {
        let stages = stages.clone();
        move || stages().iter().sum::<Duration>()
    };
    let total_color = total.clone();
    let stage = |index: usize| {
        let stages = stages.clone();
        row()
            .grow(1.0)
            .basis(px(0.0))
            .gap(gap(6.0))
            .align_items(Align::Center)
            .child(
                smudge_core::widget::panel()
                    .width(px(8.0))
                    .height(px(8.0))
                    .corner_radius(2.0)
                    .background(STAGE_COLORS[index]),
            )
            .child(note(STAGE_NAMES[index]))
            .child(spacer())
            .child(mono(move || millis(stages()[index])))
    };
    let mut grid = column().gap(gap(2.0));
    for pair in 0..STAGE_NAMES.len() / 2 {
        grid = grid.child(
            row()
                .gap(gap(16.0))
                .child(stage(2 * pair))
                .child(stage(2 * pair + 1)),
        );
    }
    card()
        .child(
            row()
                .align_items(Align::Center)
                .child(heading(title))
                .child(spacer())
                .child(
                    mono(move || millis(total()))
                        .text_size(look::BODY)
                        .text_color(move || cost_color(total_color())),
                ),
        )
        .child(grid)
        .child(
            label(move || {
                frame().map_or_else(
                    || String::from("Hover a bar of the timeline to break down its frame."),
                    |frame| frame.describe(),
                )
            })
            .text_size(look::SMALL)
            .text_color(look::MUTED),
        )
}
