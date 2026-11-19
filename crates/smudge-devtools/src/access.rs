//! The accessibility tab: the warnings about nodes assistive technology
//! cannot describe well, and the accessibility tree.

use std::collections::HashMap;
use std::fmt::Write;

use smudge_core::element::Element;
use smudge_core::inspect::{A11yWarning, A11yWarningKind};
use smudge_core::reactive::prelude::*;
use smudge_core::semantics::SemanticsNode;
use smudge_core::style::px;
use smudge_core::widget::Panel;
use smudge_text::label;
use smudge_widgets::{column, scroll};

use crate::Tools;

/// How many nodes of the tree the tab lists before it stops.
const NODES: usize = 300;

/// The height of the tab's text, in logical pixels.
const HEIGHT: f32 = 330.0;

/// Builds the accessibility tab.
pub(crate) fn access_tab(tools: &Tools) -> Element<Panel> {
    let text = tools.access.clone();
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

/// Describes `warnings` and then `nodes`, the tree indented by depth, one
/// node per line with its role, name and value.
pub(crate) fn describe(nodes: &[SemanticsNode], warnings: &[A11yWarning]) -> String {
    let mut text = format!("Warnings: {}", warnings.len());
    for warning in warnings {
        let what = match warning.kind {
            A11yWarningKind::FocusableWithoutName => "takes the focus but has no name",
            A11yWarningKind::PressableWithoutRole => "takes a press but reports no role",
            A11yWarningKind::TextInputWithoutName => "is a text input with no name",
            _ => "cannot be described well",
        };
        let rect = warning.rect;
        let _ = write!(
            text,
            "\n  the node at {}, {} {}x{} {what}",
            rect.x0,
            rect.y0,
            rect.width(),
            rect.height()
        );
    }
    let mut depths: HashMap<_, usize> = HashMap::new();
    let _ = write!(text, "\nTree: {} nodes", nodes.len());
    for node in nodes.iter().take(NODES) {
        let depth = node
            .parent
            .and_then(|parent| depths.get(&parent).copied())
            .map_or(0, |depth| depth + 1);
        depths.insert(node.id, depth);
        let _ = write!(text, "\n{:indent$}{:?}", "", node.role, indent = depth * 2);
        if let Some(label) = &node.label {
            let _ = write!(text, " \"{label}\"");
        }
        if let Some(value) = &node.value {
            let _ = write!(text, " = \"{value}\"");
        }
    }
    if nodes.len() > NODES {
        let _ = write!(text, "\n{} more nodes", nodes.len() - NODES);
    }
    text
}
