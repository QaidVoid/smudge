//! The accessibility tab: the warnings about nodes assistive technology
//! cannot describe well, and the accessibility tree.

use std::collections::HashMap;
use std::rc::Rc;

use smudge_core::element::{Element, Slot};
use smudge_core::geometry::{Rect, Size};
use smudge_core::inspect::{A11yWarning, A11yWarningKind};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, Edges, px};
use smudge_core::tree::WidgetId;
use smudge_core::widget::Panel;
use smudge_text::label;
use smudge_widgets::{column, row};

use crate::look::{self, chip, gap, mono, note, number, rows, section, size_text};
use crate::{Tab, Tools};

/// How many nodes of the tree the tab lists before it stops.
const NODES: usize = 300;

/// One node of the accessibility tree as the tab lists it.
#[derive(Clone, Debug)]
struct Node {
    /// The node.
    id: WidgetId,
    /// How deep it sits in the accessibility tree.
    depth: usize,
    /// Its role.
    role: Role,
    /// Its name.
    label: Option<String>,
    /// Its value.
    value: Option<String>,
}

/// What the accessibility tab shows, as [`Tools::show_access`] takes it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Audit {
    /// The nodes assistive technology cannot describe well.
    warnings: Vec<A11yWarning>,
    /// The first [`NODES`] nodes of the tree, parents first.
    nodes: Vec<Node>,
    /// How many nodes the tree has.
    node_count: usize,
}

impl Audit {
    /// Builds what the tab shows from `nodes`, the app's accessibility tree
    /// in paint order, and `warnings`.
    pub(crate) fn new(nodes: &[SemanticsNode], warnings: &[A11yWarning]) -> Audit {
        let mut depths: HashMap<WidgetId, usize> = HashMap::new();
        let mut listed = Vec::with_capacity(nodes.len().min(NODES));
        for node in nodes.iter().take(NODES) {
            let depth = node
                .parent
                .and_then(|parent| depths.get(&parent).copied())
                .map_or(0, |depth| depth + 1);
            depths.insert(node.id, depth);
            listed.push(Node {
                id: node.id,
                depth,
                role: node.role,
                label: node.label.clone(),
                value: node.value.clone(),
            });
        }
        Audit {
            warnings: warnings.to_vec(),
            nodes: listed,
            node_count: nodes.len(),
        }
    }
}

/// Names a warning's kind, and says why it matters and how to fix it.
fn explain(kind: A11yWarningKind) -> (&'static str, &'static str) {
    match kind {
        A11yWarningKind::FocusableWithoutName => (
            "Focusable without a name",
            "Tab reaches it, but a screen reader has nothing to call it. Give it an \
             accessible label.",
        ),
        A11yWarningKind::PressableWithoutRole => (
            "Pressable without a role",
            "It takes a press but reports no role, so assistive technology does not \
             offer it as a control. Give it a role, such as a button.",
        ),
        A11yWarningKind::TextInputWithoutName => (
            "Text input without a name",
            "A screen reader cannot say what to type here. Give it an accessible label.",
        ),
        _ => (
            "Hard to describe",
            "Assistive technology cannot describe this node well.",
        ),
    }
}

/// Writes where `rect` is and how large, as a warning names its node.
fn place(rect: Rect) -> String {
    format!(
        "At {}, {}, {}",
        number(rect.x0),
        number(rect.y0),
        size_text(rect)
    )
}

/// Returns what selects the node `id` returns and shows it in the
/// inspector.
fn inspect(tools: &Tools, id: impl Fn() -> Option<WidgetId> + 'static) -> impl FnMut() + 'static {
    let (selected, tab) = (tools.selected.clone(), tools.tab.clone());
    move || {
        if let Some(id) = id() {
            selected.set(Some(id));
            tab.set(Tab::Inspector);
        }
    }
}

/// Builds the accessibility tab.
pub(crate) fn access_tab(tools: &Tools) -> Element<Panel> {
    let audit = &tools.access;
    let waiting = audit.clone();
    look::scrolling(
        look::tab_content()
            .child(look::when(
                move || waiting.with(Option::is_none),
                || note("Waiting for the app's next frame.").text_color(look::FAINT),
            ))
            .child(summary(audit))
            .child(warnings(tools))
            .child(tree(tools)),
    )
}

/// Builds the line of chips that counts the warnings and the nodes.
fn summary(audit: &RwSignal<Option<Rc<Audit>>>) -> Element<Panel> {
    let warned = look::read(audit, |audit| audit.warnings.len());
    let (good, bad) = (warned.clone(), warned);
    let nodes = look::read(audit, |audit| audit.node_count);
    row()
        .gap(gap(6.0))
        .child(look::when(
            move || good() == 0,
            || chip("No warnings", look::GOOD),
        ))
        .child(look::when(move || bad() > 0, {
            let audit = audit.clone();
            move || {
                let count = look::read(&audit, |audit| audit.warnings.len());
                chip(
                    move || {
                        let count = count();
                        format!(
                            "{count} {}",
                            if count == 1 { "warning" } else { "warnings" }
                        )
                    },
                    look::WARN,
                )
            }
        }))
        .child(chip(move || format!("{} nodes", nodes()), look::MUTED))
}

/// Builds the warnings, a card each, which show only when there are some.
/// Pressing a card shows its node in the inspector.
fn warnings(tools: &Tools) -> Element<Slot> {
    let audit = tools.access.clone();
    let any = look::read(&audit, |audit| audit.warnings.len());
    let tools = tools.clone();
    look::when(
        move || any() > 0,
        move || {
            let (audit, tools) = (audit.clone(), tools.clone());
            section("Warnings", "Press one to see its node in the inspector.").child(
                column().gap(gap(6.0)).child(rows(
                    look::read(&audit, |audit| audit.warnings.len()),
                    move |index| warning(&tools, &audit, index),
                )),
            )
        },
    )
}

/// Builds the card of the warning at `index`.
fn warning(tools: &Tools, audit: &RwSignal<Option<Rc<Audit>>>, index: usize) -> Element<Panel> {
    let entry = look::read(audit, move |audit| audit.warnings.get(index).copied());
    let (title, about, at, id) = (entry.clone(), entry.clone(), entry.clone(), entry);
    look::card()
        .gap(gap(3.0))
        .border_edges(Edges {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 3.0,
        })
        .border_color(look::WARN)
        .hover(|p| p.background(look::HOVER))
        .pressed(|p| p.background(look::PRESSED))
        .role(Role::Button)
        .focusable()
        .activate_on_keys()
        .on_press(inspect(tools, move || id().map(|warning| warning.id)))
        .child(
            label(move || {
                title().map_or_else(String::new, |warning| String::from(explain(warning.kind).0))
            })
            .text_color(look::WARN),
        )
        .child(note(move || {
            about().map_or_else(String::new, |warning| String::from(explain(warning.kind).1))
        }))
        .child(
            mono(move || at().map_or_else(String::new, |warning| place(warning.rect)))
                .text_color(look::FAINT),
        )
}

/// Builds the tree: a row for each node, indented by depth, with its role,
/// name and value. Pressing a row shows its node in the inspector.
fn tree(tools: &Tools) -> Element<Panel> {
    let audit = tools.access.clone();
    let count = look::read(&audit, |audit| audit.nodes.len());
    let more = look::read(&audit, |audit| audit.node_count.saturating_sub(NODES));
    let shown_more = more.clone();
    let tools = tools.clone();
    section(
        "Tree",
        "What assistive technology reads, in the order it reads it.",
    )
    .child(
        column()
            .padding(px(4.0))
            .background(look::SUNKEN)
            .corner_radius(6.0)
            .child(rows(count, move |index| node_row(&tools, &audit, index)))
            .child(look::when(
                move || more() > 0,
                move || {
                    let more = shown_more.clone();
                    note(move || format!("and {} more nodes", more())).text_color(look::FAINT)
                },
            )),
    )
}

/// Builds the row of the tree's node at `index`.
fn node_row(tools: &Tools, audit: &RwSignal<Option<Rc<Audit>>>, index: usize) -> Element<Panel> {
    let entry = look::read(audit, move |audit| audit.nodes.get(index).cloned());
    let (indent, role, name, value, id) = (
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry,
    );
    row()
        .gap(gap(8.0))
        .align_items(Align::Center)
        .corner_radius(4.0)
        .padding_xy(Size {
            width: 6.0,
            height: 2.0,
        })
        .hover(|p| p.background(look::HOVER))
        .pressed(|p| p.background(look::PRESSED))
        .on_press(inspect(tools, move || id().map(|node| node.id)))
        .child(look::indent(move || indent().map_or(0, |node| node.depth)))
        .child(
            mono(move || role().map_or_else(String::new, |node| format!("{:?}", node.role)))
                .text_color(look::ACCENT),
        )
        .child(
            label(move || {
                name()
                    .and_then(|node| node.label)
                    .map_or_else(String::new, |label| format!("\u{201c}{label}\u{201d}"))
            })
            .shrink(1.0)
            .min_width(px(0.0)),
        )
        .child(
            note(move || {
                value()
                    .and_then(|node| node.value)
                    .map_or_else(String::new, |value| format!("= {value}"))
            })
            .shrink(1.0)
            .min_width(px(0.0)),
        )
}
