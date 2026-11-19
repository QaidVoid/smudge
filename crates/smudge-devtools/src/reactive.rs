//! The reactive tab: the effects that run most, the cycles a frame cut
//! short, and the owner tree.

use std::fmt::Write;
use std::panic::Location;

use smudge_core::element::Element;
use smudge_core::inspect::{CycleInfo, OwnerEntry};
use smudge_core::reactive::prelude::*;
use smudge_core::style::px;
use smudge_core::widget::Panel;
use smudge_text::label;
use smudge_widgets::{column, scroll};

use crate::Tools;
use crate::inspector::site;

/// How many owners the tree lists before it stops.
const OWNERS: usize = 200;

/// The height of the tab's text, in logical pixels.
const HEIGHT: f32 = 330.0;

/// Builds the reactive tab.
pub(crate) fn reactive_tab(tools: &Tools) -> Element<Panel> {
    let text = tools.reactive.clone();
    column().child(
        scroll(label(move || {
            let text = text.get();
            if text.is_empty() {
                String::from("Waiting for the app's next frame.")
            } else {
                text
            }
        }))
        .height(px(HEIGHT)),
    )
}

/// Describes what the reactive tab shows, as [`Tools::show_reactive`]
/// takes it.
pub(crate) fn describe(
    busiest: &[(Option<&'static Location<'static>>, f64)],
    cycles: &[CycleInfo],
    owners: &[OwnerEntry],
) -> String {
    let mut text = String::from("Busiest effects, runs a second:");
    for &(at, rate) in busiest {
        let place = at.map_or_else(|| String::from("element bindings"), site);
        let _ = write!(text, "\n  {rate:>6.1}  {place}");
    }
    if busiest.is_empty() {
        text.push_str("\n  none ran");
    }
    if !cycles.is_empty() {
        text.push_str("\nCycles cut short:");
        for cycle in cycles {
            let effect = cycle.effect_defined_at.map_or_else(
                || String::from("a frame's updates"),
                |at| format!("an effect created at {}", site(at)),
            );
            let write = cycle
                .written_at
                .map_or_else(|| String::from("an unknown place"), site);
            let _ = write!(text, "\n  {effect}, last woken by the write at {write}");
        }
    }
    let (signals, memos, effects) = owners.iter().fold((0, 0, 0), |(s, m, e), owner| {
        (s + owner.signals, m + owner.memos, e + owner.effects)
    });
    let _ = write!(
        text,
        "\nOwners: {}, holding {signals} signals, {memos} memos and {effects} effects",
        owners.len()
    );
    for owner in owners.iter().take(OWNERS) {
        let _ = write!(
            text,
            "\n{:indent$}{} signals, {} memos, {} effects",
            "",
            owner.signals,
            owner.memos,
            owner.effects,
            indent = owner.depth * 2
        );
    }
    if owners.len() > OWNERS {
        let _ = write!(text, "\n{} more owners", owners.len() - OWNERS);
    }
    text
}
