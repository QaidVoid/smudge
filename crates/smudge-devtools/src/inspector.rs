//! The inspector tab: a pick button, the app's tree and the details of the
//! selected node.

use std::fmt::Write;

use smudge_core::element::Element;
use smudge_core::geometry::Rect;
use smudge_core::inspect::NodeDetails;
use smudge_core::reactive::prelude::*;
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::widget::Panel;
use smudge_text::label;
use smudge_widgets::{button, column, scroll, virtual_list};

use crate::Tools;

/// The height of one row of the tree, in logical pixels.
const ROW: f32 = 18.0;

/// The height of the tree, in logical pixels.
const TREE: f32 = 150.0;

/// The height of the details, in logical pixels.
const DETAILS: f32 = 180.0;

/// Builds the inspector tab.
pub(crate) fn inspector(tools: &Tools) -> Element<Panel> {
    let (picking, toggle) = (tools.picking.clone(), tools.picking.clone());
    let pick = button(move || {
        String::from(if picking.get() {
            "Pick: click a node"
        } else {
            "Pick"
        })
    })
    .on_press(move || toggle.update(|picking| *picking = !*picking));

    let count = tools.tree.clone();
    let rows = tools.clone();
    let tree = virtual_list(
        move || count.with(|tree| tree.len()),
        ROW,
        move |index| tree_row(&rows, index),
    )
    .height(px(TREE));

    let details = tools.details.clone();
    let text = label(move || {
        details.with(|details| {
            details.as_deref().map_or_else(
                || String::from("Pick a node, or press a row of the tree."),
                describe,
            )
        })
    });
    column()
        .gap(smudge_core::geometry::Size {
            width: 0.0,
            height: 6.0,
        })
        .child(pick)
        .child(tree)
        .child(scroll(text).height(px(DETAILS)))
}

/// Builds the row of the tree at `index`, which shows its node indented by
/// depth, muted while hidden and marked while selected, and selects it when
/// pressed.
fn tree_row(tools: &Tools, index: usize) -> Element<smudge_text::Label> {
    let (tree, selected) = (tools.tree.clone(), tools.selected.clone());
    let (hidden_tree, pick_tree, pick) = (
        tools.tree.clone(),
        tools.tree.clone(),
        tools.selected.clone(),
    );
    label(move || {
        let chosen = selected.get();
        tree.with(|tree| {
            tree.get(index).map_or_else(String::new, |entry| {
                let mark = if chosen == Some(entry.id) { "> " } else { "  " };
                format!(
                    "{mark}{:indent$}{}",
                    "",
                    short_name(entry.type_name),
                    indent = entry.depth * 2
                )
            })
        })
    })
    .text_color(move || {
        let hidden = hidden_tree.with(|tree| tree.get(index).is_some_and(|entry| entry.hidden));
        ColorValue::from(if hidden {
            ColorToken::TEXT_MUTED
        } else {
            ColorToken::TEXT
        })
    })
    .on_press(move || {
        if let Some(id) = pick_tree.with(|tree| tree.get(index).map(|entry| entry.id)) {
            pick.set(Some(id));
        }
    })
}

/// Returns the name of a widget type without its path or generic arguments.
pub(crate) fn short_name(type_name: &str) -> &str {
    let path = type_name.split('<').next().unwrap_or(type_name);
    path.rsplit("::").next().unwrap_or(path)
}

/// Describes `details` as the inspector shows them, one fact per line.
pub(crate) fn describe(details: &NodeDetails) -> String {
    let mut text = String::from(short_name(details.type_name));
    if let Some(semantics) = &details.semantics {
        let _ = write!(text, " ({:?}", semantics.role);
        if let Some(label) = &semantics.label {
            let _ = write!(text, " \"{label}\"");
        }
        text.push(')');
    }
    if details.hidden {
        text.push_str(", hidden");
    }
    match details.created_at {
        Some(at) => {
            let _ = write!(text, "\n{}:{}:{}", at.file(), at.line(), at.column());
        }
        None => text.push_str("\nbuilt outside an element"),
    }
    let states = details.states;
    let active: Vec<&str> = [
        (states.hovered, "hovered"),
        (states.pressed, "pressed"),
        (states.focused, "focused"),
        (states.disabled, "disabled"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    if !active.is_empty() {
        let _ = write!(text, "\nstates: {}", active.join(", "));
    }
    let _ = write!(text, "\ndirty: {:?}", details.dirty);
    let boxes = details.boxes;
    for (name, rect) in [
        ("margin", boxes.margin),
        ("border", boxes.border),
        ("padding", boxes.padding),
        ("content", boxes.content),
    ] {
        let _ = write!(text, "\n{name}: {}", rect_text(rect));
    }
    match details.painted {
        Some(painted) => {
            let _ = write!(
                text,
                "\npainted: {}, {} ops",
                rect_text(painted),
                details.ops
            );
        }
        None => text.push_str("\nnot painted yet"),
    }
    let _ = write!(text, "\nstyle: {:#?}", details.base);
    if details.resolved != details.base {
        let _ = write!(text, "\nresolved: {:#?}", details.resolved);
    }
    text
}

/// Writes `rect` as its top left corner and its size.
fn rect_text(rect: Rect) -> String {
    format!(
        "{}, {} {}x{}",
        rect.x0,
        rect.y0,
        rect.width(),
        rect.height()
    )
}
