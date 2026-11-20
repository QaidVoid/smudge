//! The inspector tab: a pick button, the app's tree and the details of the
//! selected node.

use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Rect, Size};
use smudge_core::inspect::{BindingInfo, BoxModel, NodeDetails, SourceKind};
use smudge_core::reactive::{Memo, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Align, Direction, Justify, px};
use smudge_core::theme::ColorValue;
use smudge_core::widget::Panel;
use smudge_text::{FontWeight, LabelExt, label};
use smudge_widgets::{column, row, scroll, spacer, virtual_list};

use crate::look::{self, action, chip, fact, gap, heading, mono, note, number, rows, section};
use crate::marks::BOXES;
use crate::{Tools, styles};

/// The height of one row of the tree, in logical pixels.
const ROW: f32 = 22.0;

/// The height of the tree, in logical pixels.
const TREE: f32 = 154.0;

/// The details of the selected node, as the tab reads them.
type Details = RwSignal<Option<Rc<NodeDetails>>>;

/// Builds the inspector tab.
pub(crate) fn inspector(tools: &Tools) -> Element<Panel> {
    let count = tools.tree.clone();
    let rows_of = tools.clone();
    let tree = virtual_list(
        move || count.with(|tree| tree.len()),
        ROW,
        move |index| tree_row(&rows_of, index),
    )
    .role(Role::List)
    .accessible_label("Tree")
    .navigate_with_arrows(Direction::Column)
    .height(px(TREE))
    .shrink(1.0)
    .min_height(px(3.0 * ROW));
    column()
        .padding_xy(Size {
            width: crate::PAD,
            height: 4.0,
        })
        .gap(gap(8.0))
        .child(toolbar(tools))
        .child(status(tools))
        .child(
            column()
                .padding(px(4.0))
                .background(look::SUNKEN)
                .corner_radius(6.0)
                .shrink(1.0)
                .min_height(px(3.0 * ROW + 8.0))
                .child(tree),
        )
        .child(
            scroll(
                column()
                    .padding_xy(Size {
                        width: 0.0,
                        height: 4.0,
                    })
                    .child(details(&tools.details)),
            )
            .grow(1.0)
            .basis(px(0.0))
            .min_height(px(3.0 * ROW)),
        )
}

/// Builds the toolbar: the pick button, the copy button, and how many nodes
/// the tree has.
fn toolbar(tools: &Tools) -> Element<Panel> {
    let (back, text, toggle) = (
        tools.picking.clone(),
        tools.picking.clone(),
        tools.picking.clone(),
    );
    let caption = tools.picking.clone();
    let pick = row()
        .padding_xy(Size {
            width: 10.0,
            height: 3.0,
        })
        .gap(gap(6.0))
        .align_items(Align::Center)
        .corner_radius(6.0)
        .border(1.0)
        .border_color(Color::TRANSPARENT)
        .background(move || {
            ColorValue::from(if back.get() {
                look::ACCENT
            } else {
                look::RAISED
            })
        })
        .pressed(|p| p.background(look::PRESSED))
        .focused(|p| p.border_color(look::ACCENT))
        .role(Role::Button)
        .accessible_label("Pick")
        .focusable()
        .activate_on_keys()
        .on_press(move || toggle.update(|picking| *picking = !*picking))
        .child(
            label(move || String::from(if caption.get() { "Picking" } else { "Pick" })).text_color(
                move || {
                    ColorValue::from(if text.get() {
                        look::ON_ACCENT
                    } else {
                        look::TEXT
                    })
                },
            ),
        );
    let (copy, unselected) = (tools.copy.clone(), tools.details.clone());
    let copy = action("Copy as test")
        .disabled(move || unselected.with(Option::is_none))
        .disabled_style(|p| p.opacity(0.4))
        .on_press(move || copy.set(true));
    let count = tools.tree.clone();
    row()
        .gap(gap(8.0))
        .align_items(Align::Center)
        .child(pick)
        .child(copy)
        .child(spacer())
        .child(note(move || {
            look::counted(count.with(|tree| tree.len()), "node", "nodes")
        }))
}

/// Builds the line under the toolbar, which says what to do while picking
/// and what the last copy wrote, and takes no room otherwise.
fn status(tools: &Tools) -> Element<smudge_core::element::Slot> {
    let (picking, copied) = (tools.picking.clone(), tools.copied.clone());
    let (text, color) = (tools.picking.clone(), tools.picking.clone());
    let message = tools.copied.clone();
    look::when(
        move || picking.get() || copied.with(Option::is_some),
        move || {
            let (text, color, message) = (text.clone(), color.clone(), message.clone());
            label(move || {
                if text.get() {
                    String::from("Click a node in the app to select it.")
                } else {
                    message.get().unwrap_or_default()
                }
            })
            .text_size(look::SMALL)
            .text_color(move || {
                ColorValue::from(if color.get() {
                    look::ACCENT
                } else {
                    look::MUTED
                })
            })
        },
    )
}

/// Builds the row of the tree at `index`, which shows its node indented by
/// depth with its size, muted while hidden and raised while selected, and
/// selects it when pressed, or on Enter or Space while it holds the focus.
fn tree_row(tools: &Tools, index: usize) -> Element<Panel> {
    let entry = {
        let tree = tools.tree.clone();
        move || tree.with(|tree| tree.get(index).copied())
    };
    let (indent, chosen, name, hidden, size, pick) = (
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry.clone(),
    );
    let (spoken, marked) = (entry.clone(), entry);
    let (selected, select, picked) = (
        tools.selected.clone(),
        tools.selected.clone(),
        tools.selected.clone(),
    );
    row()
        .height(px(ROW))
        .gap(gap(8.0))
        .align_items(Align::Center)
        .corner_radius(4.0)
        .padding_xy(Size {
            width: 6.0,
            height: 0.0,
        })
        .background(move || {
            let on = chosen().is_some_and(|entry| selected.get() == Some(entry.id));
            ColorValue::from(if on {
                look::ACCENT_SOFT
            } else {
                Color::TRANSPARENT
            })
        })
        .border(1.0)
        .border_color(Color::TRANSPARENT)
        .hover(|p| p.background(look::HOVER))
        .focused(|p| p.border_color(look::ACCENT))
        .role(Role::ListItem)
        .accessible_label(move || {
            spoken().map_or_else(String::new, |entry| {
                String::from(short_name(entry.type_name))
            })
        })
        .accessible_selected(move || marked().is_some_and(|entry| picked.get() == Some(entry.id)))
        .focusable()
        .activate_on_keys()
        .on_press(move || {
            if let Some(entry) = pick() {
                select.set(Some(entry.id));
            }
        })
        .child(look::indent(move || {
            indent().map_or(0, |entry| entry.depth)
        }))
        .child(
            label(move || {
                name().map_or_else(String::new, |entry| {
                    String::from(short_name(entry.type_name))
                })
            })
            .text_color(move || {
                ColorValue::from(if hidden().is_some_and(|entry| entry.hidden) {
                    look::FAINT
                } else {
                    look::TEXT
                })
            }),
        )
        .child(
            mono(move || size().map_or_else(String::new, |entry| look::size_text(entry.rect)))
                .text_color(look::FAINT),
        )
}

/// Returns the name of a widget type without its path or generic arguments.
pub(crate) fn short_name(type_name: &str) -> &str {
    let path = type_name.split('<').next().unwrap_or(type_name);
    path.rsplit("::").next().unwrap_or(path)
}

/// Writes `at` as `file:line:column`.
pub(crate) fn site(at: &std::panic::Location<'_>) -> String {
    format!("{}:{}:{}", at.file(), at.line(), at.column())
}

/// Builds the details of the selected node, or what to do while none is.
fn details(details: &Details) -> Element<Panel> {
    let (none, some) = (details.clone(), details.clone());
    let shown = details.clone();
    column()
        .child(look::when(
            move || none.with(Option::is_none),
            || {
                look::card()
                    .child(heading("Nothing selected"))
                    .child(note(
                        "Press Pick and click a node in the app, or press a row of the tree. \
                         Its boxes show over the app.",
                    ))
                    .child(note(
                        "Copy as test then writes the node's layout and pixels to \
                         target/smudge-devtools, for a snapshot test.",
                    ))
            },
        ))
        .child(look::when(
            move || some.with(Option::is_some),
            move || {
                column()
                    .gap(gap(16.0))
                    .child(identity(&shown))
                    .child(
                        section(
                            "Box model",
                            "Margin, border, padding and content, in the colors the overlay \
                             shows them in.",
                        )
                        .child(box_model(&shown)),
                    )
                    .child(state(&shown))
                    .child(bindings(&shown))
                    .child(style(&shown))
            },
        ))
}

/// Builds what names the node: its widget, its role and accessible name,
/// and where it was built.
fn identity(details: &Details) -> Element<Panel> {
    let name = look::read(details, |details| {
        String::from(short_name(details.type_name))
    });
    let role = look::read(details, |details| {
        details
            .semantics
            .as_ref()
            .map(|semantics| format!("{:?}", semantics.role))
    });
    let has_role = role.clone();
    let hidden = look::read(details, |details| details.hidden);
    let spoken = look::read(details, |details| {
        details
            .semantics
            .as_ref()
            .and_then(|semantics| semantics.label.clone())
    });
    let has_spoken = spoken.clone();
    let source = look::read(details, |details| details.created_at.map(site));
    let source_color = source.clone();
    column()
        .gap(gap(3.0))
        .child(
            row()
                .gap(gap(8.0))
                .align_items(Align::Center)
                .child(label(name).text_size(15.0).weight(FontWeight::BOLD))
                .child(look::when(
                    move || has_role().is_some(),
                    move || {
                        let role = role.clone();
                        chip(move || role().unwrap_or_default(), look::ACCENT)
                    },
                ))
                .child(look::when(hidden, || chip("hidden", look::FAINT))),
        )
        .child(look::when(
            move || has_spoken().is_some(),
            move || {
                let spoken = spoken.clone();
                note(move || format!("Read as \u{201c}{}\u{201d}", spoken().unwrap_or_default()))
            },
        ))
        .child(
            mono(move || source().unwrap_or_else(|| String::from("Built outside an element")))
                .text_color(move || {
                    ColorValue::from(if source_color().is_some() {
                        look::ACCENT
                    } else {
                        look::FAINT
                    })
                }),
        )
}

/// Returns the widths of the band between `outer` and `inner` at the top,
/// right, bottom and left.
fn band(outer: Rect, inner: Rect) -> [f32; 4] {
    [
        inner.y0 - outer.y0,
        outer.x1 - inner.x1,
        outer.y1 - inner.y1,
        inner.x0 - outer.x0,
    ]
}

/// Returns `color` laid over the panel's background at `alpha`, out of 255,
/// as an opaque color.
fn over_background(color: Color, alpha: u8) -> Color {
    let mix = |top: u8, bottom: u8| -> u8 {
        let top = u32::from(top) * u32::from(alpha);
        let bottom = u32::from(bottom) * (255 - u32::from(alpha));
        u8::try_from((top + bottom) / 255).unwrap_or(u8::MAX)
    };
    let back = look::BACKGROUND;
    Color::rgb(
        mix(color.r, back.r),
        mix(color.g, back.g),
        mix(color.b, back.b),
    )
}

/// Builds the box model: the margin, border and padding as nested bands,
/// each with its widths at its four sides, around the content's size.
fn box_model(details: &Details) -> Element<Panel> {
    let content = look::read(details, |details| details.boxes.content);
    let layer = |name: &'static str,
                 color: Color,
                 sides: fn(&BoxModel) -> [f32; 4],
                 inner: Element<Panel>| {
        let side = |at: usize| {
            let sides = look::read(details, move |details| sides(&details.boxes)[at]);
            mono(move || number(sides()))
        };
        let edge = |at: usize| {
            row()
                .width(px(30.0))
                .shrink(0.0)
                .justify_content(Justify::Center)
                .child(side(at))
        };
        column()
            .padding_xy(Size {
                width: 2.0,
                height: 1.0,
            })
            .background(over_background(color, 56))
            .border(1.0)
            .border_color(Color::rgba(color.r, color.g, color.b, 150))
            .corner_radius(4.0)
            .child(
                row()
                    .align_items(Align::Center)
                    .child(
                        row()
                            .grow(1.0)
                            .basis(px(0.0))
                            .padding_xy(Size {
                                width: 4.0,
                                height: 0.0,
                            })
                            .child(note(name).text_color(color)),
                    )
                    .child(side(0))
                    .child(row().grow(1.0).basis(px(0.0))),
            )
            .child(
                row()
                    .align_items(Align::Center)
                    .child(edge(3))
                    .child(inner.grow(1.0))
                    .child(edge(1)),
            )
            .child(row().justify_content(Justify::Center).child(side(2)))
    };
    let core = row()
        .justify_content(Justify::Center)
        .padding_xy(Size {
            width: 0.0,
            height: 8.0,
        })
        .background(over_background(BOXES[3], 90))
        .border(1.0)
        .border_color(Color::rgba(BOXES[3].r, BOXES[3].g, BOXES[3].b, 180))
        .corner_radius(4.0)
        .child(mono(move || look::size_text(content())));
    let padding = layer(
        "padding",
        BOXES[2],
        |boxes| band(boxes.padding, boxes.content),
        core,
    );
    let border = layer(
        "border",
        BOXES[1],
        |boxes| band(boxes.border, boxes.padding),
        padding,
    );
    layer(
        "margin",
        BOXES[0],
        |boxes| band(boxes.margin, boxes.border),
        border,
    )
}

/// Builds the node's state: its interaction states, what is out of date,
/// where it is, and what it last painted.
fn state(details: &Details) -> Element<Panel> {
    let states = look::read(details, |details| {
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
        if active.is_empty() {
            String::from("none")
        } else {
            active.join(", ")
        }
    });
    let dirty = look::read(details, |details| format!("{:?}", details.dirty));
    let position = look::read(details, |details| {
        let border = details.boxes.border;
        format!("{}, {}", number(border.x0), number(border.y0))
    });
    let size = look::read(details, |details| look::size_text(details.boxes.border));
    let painted = look::read(details, |details| match details.painted {
        Some(painted) => format!(
            "{} at {}, {} in {} {}",
            look::size_text(painted),
            number(painted.x0),
            number(painted.y0),
            details.ops,
            if details.ops == 1 { "op" } else { "ops" }
        ),
        None => String::from("not yet"),
    });
    section(
        "State",
        "Where the node is, what it last painted, and what is out of date.",
    )
    .child(
        column()
            .gap(gap(2.0))
            .child(fact("Position", position))
            .child(fact("Size", size))
            .child(fact("Painted", painted))
            .child(fact("States", states))
            .child(fact("Dirty", dirty)),
    )
}

/// Builds the node's bindings: a card for each, with how often it ran, what
/// it read, and the write that woke it last.
fn bindings(details: &Details) -> Element<Panel> {
    let count = look::read(details, |details| details.bindings.len());
    let none = count.clone();
    let details = details.clone();
    section(
        "Bindings",
        "Each binding reruns when a signal it read changes. The write that woke it \
         last is the reason it ran.",
    )
    .child(look::when(
        move || none() == 0,
        || note("No bindings.").text_color(look::FAINT),
    ))
    .child(
        column()
            .gap(gap(6.0))
            .child(rows(count, move |index| binding(&details, index))),
    )
}

/// Builds the card of the binding at `index`.
fn binding(details: &Details, index: usize) -> Element<Panel> {
    let entry = {
        let memo = Memo::new(look::read(details, move |details| {
            details.bindings.get(index).cloned()
        }));
        move || memo.get()
    };
    let (runs, at, sources, woken, has_woken) = (
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry.clone(),
        entry,
    );
    let count = {
        let sources = sources.clone();
        move || sources().map_or(0, |binding| binding.sources.len())
    };
    look::card()
        .gap(gap(2.0))
        .child(
            row()
                .gap(gap(8.0))
                .child(label(move || {
                    runs().map_or_else(String::new, |binding| {
                        format!(
                            "Ran {} {}",
                            binding.runs,
                            if binding.runs == 1 { "time" } else { "times" }
                        )
                    })
                }))
                .child(spacer())
                .child(
                    mono(move || {
                        at().map_or_else(String::new, |binding: BindingInfo| {
                            binding
                                .defined_at
                                .map_or_else(|| String::from("element binding"), site)
                        })
                    })
                    .text_color(look::FAINT)
                    .shrink(1.0)
                    .min_width(px(0.0)),
                ),
        )
        .child(rows(count, move |source| {
            let (kinds, sources) = (sources.clone(), sources.clone());
            column()
                .child(note(move || {
                    let kind = kinds()
                        .and_then(|binding| binding.sources.get(source).copied())
                        .map(|source| source.kind);
                    String::from(match kind {
                        Some(SourceKind::Memo) => "Reads a memo created at",
                        Some(SourceKind::Signal) => "Reads a signal created at",
                        _ => "Reads a source created at",
                    })
                }))
                .child(mono(move || {
                    sources()
                        .and_then(|binding| binding.sources.get(source).copied())
                        .and_then(|source| source.defined_at)
                        .map_or_else(|| String::from("an unknown place"), site)
                }))
        }))
        .child(look::when(
            move || has_woken().is_some_and(|binding| binding.woken_by.is_some()),
            move || {
                let woken = woken.clone();
                column()
                    .child(note("Woken last by the write at").text_color(look::WARN))
                    .child(mono(move || {
                        woken()
                            .and_then(|binding| binding.woken_by)
                            .map_or_else(String::new, |write| site(write.written_at))
                    }))
            },
        ))
}

/// Builds the node's style: the properties it was built with that differ
/// from the default, and those its state or placement change.
fn style(details: &Details) -> Element<Panel> {
    let built = Memo::new(look::read(details, |details| {
        styles::style_lines(&details.base)
    }));
    let base = built.clone();
    let changed = Memo::new(look::read(details, move |details| {
        let resolved = styles::style_lines(&details.resolved);
        base.with(|built| {
            resolved
                .into_iter()
                .filter(|line| !built.contains(line))
                .collect::<Vec<_>>()
        })
    }));
    let any_changed = changed.clone();
    section(
        "Style",
        "What the node was built with, where it differs from the default.",
    )
    .child(style_list(built))
    .child(look::when(
        move || any_changed.with(|changed| !changed.is_empty()),
        move || {
            column()
                .gap(gap(4.0))
                .child(heading("Changed by its state or placement"))
                .child(style_list(changed.clone()))
        },
    ))
}

/// Builds the list of the style lines `lines` holds.
fn style_list(lines: Memo<Vec<(&'static str, String)>>) -> Element<Panel> {
    let (count, none) = (lines.clone(), lines.clone());
    column()
        .padding_xy(Size {
            width: 8.0,
            height: 6.0,
        })
        .background(look::SUNKEN)
        .corner_radius(6.0)
        .child(look::when(
            move || none.with(Vec::is_empty),
            || note("The default style.").text_color(look::FAINT),
        ))
        .child(rows(
            move || count.with(Vec::len),
            move |index| {
                let (name, value) = (lines.clone(), lines.clone());
                row()
                    .gap(gap(8.0))
                    .child(
                        mono(move || {
                            name.with(|lines| {
                                lines
                                    .get(index)
                                    .map_or_else(String::new, |(name, _)| String::from(*name))
                            })
                        })
                        .text_color(look::MUTED)
                        .width(px(92.0))
                        .shrink(0.0),
                    )
                    .child(
                        mono(move || {
                            value.with(|lines| {
                                lines
                                    .get(index)
                                    .map_or_else(String::new, |(_, value)| value.clone())
                            })
                        })
                        .shrink(1.0)
                        .min_width(px(0.0)),
                    )
            },
        ))
}
