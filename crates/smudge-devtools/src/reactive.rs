//! The reactive tab: the effects that run most, the cycles a frame cut
//! short, and the owner tree.

use std::panic::Location;
use std::rc::Rc;

use smudge_core::element::{Element, Slot};
use smudge_core::inspect::{CycleInfo, OwnerEntry};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Align, Edges, pct, px};
use smudge_core::widget::{Panel, panel};
use smudge_text::label;
use smudge_widgets::{column, row};

use crate::Tools;
use crate::inspector::site;
use crate::look::{self, gap, mono, note, rows, section};

/// How many owners the tree lists before it stops.
const OWNERS: usize = 200;

/// What the reactive tab shows, as [`Tools::show_reactive`] takes it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Trace {
    /// Where each of the busiest effects was created, with how many times a
    /// second it ran, busiest first.
    busiest: Vec<(String, f64)>,
    /// Each cycle a frame cut short: where its effect was created, and where
    /// the write that last woke it is.
    cycles: Vec<(String, String)>,
    /// The first [`OWNERS`] owners of the app, parents first.
    owners: Vec<OwnerEntry>,
    /// How many owners the app has.
    owner_count: usize,
    /// How many signals, memos and effects the owners hold.
    held: (usize, usize, usize),
}

impl Trace {
    /// Builds what the tab shows from what [`Tools::show_reactive`] takes.
    pub(crate) fn new(
        busiest: &[(Option<&'static Location<'static>>, f64)],
        cycles: &[CycleInfo],
        owners: &[OwnerEntry],
    ) -> Trace {
        Trace {
            busiest: busiest
                .iter()
                .map(|&(at, rate)| {
                    (
                        at.map_or_else(|| String::from("Element bindings"), site),
                        rate,
                    )
                })
                .collect(),
            cycles: cycles
                .iter()
                .map(|cycle| {
                    (
                        cycle
                            .effect_defined_at
                            .map_or_else(|| String::from("the frame's updates"), site),
                        cycle
                            .written_at
                            .map_or_else(|| String::from("an unknown place"), site),
                    )
                })
                .collect(),
            owners: owners.iter().take(OWNERS).copied().collect(),
            owner_count: owners.len(),
            held: owners.iter().fold((0, 0, 0), |(s, m, e), owner| {
                (s + owner.signals, m + owner.memos, e + owner.effects)
            }),
        }
    }
}

/// Builds the reactive tab.
pub(crate) fn reactive_tab(tools: &Tools) -> Element<Panel> {
    let trace = &tools.reactive;
    let waiting = trace.clone();
    look::scrolling(
        look::tab_content()
            .child(look::when(
                move || waiting.with(Option::is_none),
                || {
                    note("Counting how often effects run. The first numbers come after a second.")
                        .text_color(look::FAINT)
                },
            ))
            .child(busiest(trace))
            .child(cycles(trace))
            .child(owners(trace)),
    )
}

/// Builds the section of the busiest effects: each one's rate, the place it
/// was created, and a bar as long as its share of the busiest one's rate.
fn busiest(trace: &RwSignal<Option<Rc<Trace>>>) -> Element<Panel> {
    let count = look::read(trace, |trace| trace.busiest.len());
    let none = count.clone();
    let (trace, top) = (
        trace.clone(),
        look::read(trace, |trace| trace.busiest.first().map(|(_, rate)| *rate)),
    );
    section(
        "Busiest effects",
        "Where effects were created, by how many times a second they ran.",
    )
    .child(look::when(
        move || none() == 0,
        || note("No effect ran in the last second.").text_color(look::FAINT),
    ))
    .child(column().gap(gap(6.0)).child(rows(count, move |index| {
        let entry = look::read(&trace, move |trace| trace.busiest.get(index).cloned());
        let (rate, place, share, top) = (entry.clone(), entry.clone(), entry, top.clone());
        row()
            .gap(gap(10.0))
            .align_items(Align::Start)
            .child(
                mono(move || rate().map_or_else(String::new, |(_, rate)| format!("{rate:.1}/s")))
                    .width(px(52.0))
                    .shrink(0.0),
            )
            .child(
                column()
                    .grow(1.0)
                    .shrink(1.0)
                    .min_width(px(0.0))
                    .gap(gap(3.0))
                    .child(mono(move || {
                        place().map_or_else(String::new, |(place, _)| place)
                    }))
                    .child(
                        panel()
                            .height(px(3.0))
                            .width(pct(100.0))
                            .background(look::SUNKEN)
                            .corner_radius(2.0)
                            .child(
                                panel()
                                    .height(pct(100.0))
                                    .width(move || {
                                        let top = top().unwrap_or(0.0);
                                        let rate = share().map_or(0.0, |(_, rate)| rate);
                                        pct(if top > 0.0 {
                                            (rate / top * 100.0) as f32
                                        } else {
                                            0.0
                                        })
                                    })
                                    .background(look::ACCENT)
                                    .corner_radius(2.0),
                            ),
                    ),
            )
    })))
}

/// Builds the section of the cycles a frame cut short, which shows only
/// when there are some.
fn cycles(trace: &RwSignal<Option<Rc<Trace>>>) -> Element<Slot> {
    let count = look::read(trace, |trace| trace.cycles.len());
    let any = count.clone();
    let trace = trace.clone();
    look::when(
        move || any() > 0,
        move || {
            let trace = trace.clone();
            section(
                "Cycles",
                "Effects that kept waking each other until the frame stopped them.",
            )
            .child(
                column()
                    .gap(gap(6.0))
                    .child(rows(count.clone(), move |index| {
                        let entry =
                            look::read(&trace, move |trace| trace.cycles.get(index).cloned());
                        let (effect, write) = (entry.clone(), entry);
                        look::card()
                            .border_edges(Edges {
                                top: 0.0,
                                right: 0.0,
                                bottom: 0.0,
                                left: 3.0,
                            })
                            .border_color(look::BAD)
                            .child(label("An effect kept rerunning").text_color(look::BAD))
                            .child(note("Created at"))
                            .child(mono(move || {
                                effect().map_or_else(String::new, |(effect, _)| effect)
                            }))
                            .child(note("Last woken by the write at"))
                            .child(mono(move || {
                                write().map_or_else(String::new, |(_, write)| write)
                            }))
                    })),
            )
        },
    )
}

/// Builds the section of the owner tree: a sum of what the owners hold, and
/// a row for each owner, indented by its depth.
fn owners(trace: &RwSignal<Option<Rc<Trace>>>) -> Element<Panel> {
    let summary = look::read(trace, |trace| {
        let (signals, memos, effects) = trace.held;
        format!(
            "{} {} {}, {} and {}.",
            look::counted(trace.owner_count, "owner", "owners"),
            if trace.owner_count == 1 {
                "holds"
            } else {
                "hold"
            },
            look::counted(signals, "signal", "signals"),
            look::counted(memos, "memo", "memos"),
            look::counted(effects, "effect", "effects"),
        )
    });
    let count = look::read(trace, |trace| trace.owners.len());
    let more = look::read(trace, |trace| trace.owner_count.saturating_sub(OWNERS));
    let shown_more = more.clone();
    let trace = trace.clone();
    section(
        "Owners",
        "Each owner holds what was created under it and disposes of it all at once.",
    )
    .child(label(summary))
    .child(
        column()
            .padding(px(6.0))
            .background(look::SUNKEN)
            .corner_radius(6.0)
            .child(rows(count, move |index| {
                let entry = look::read(&trace, move |trace| trace.owners.get(index).copied());
                let (indent, text) = (entry.clone(), entry);
                row()
                    .gap(gap(6.0))
                    .align_items(Align::Center)
                    .padding_xy(smudge_core::geometry::Size {
                        width: 0.0,
                        height: 1.0,
                    })
                    .child(look::indent(move || {
                        indent().map_or(0, |owner| owner.depth)
                    }))
                    .child(look::dot(look::FAINT))
                    .child(mono(move || {
                        text().map_or_else(String::new, |owner| {
                            format!(
                                "{} signals, {} memos, {} effects",
                                owner.signals, owner.memos, owner.effects
                            )
                        })
                    }))
            }))
            .child(look::when(
                move || more() > 0,
                move || {
                    let more = shown_more.clone();
                    note(move || format!("and {} more owners", more())).text_color(look::FAINT)
                },
            )),
    )
}
