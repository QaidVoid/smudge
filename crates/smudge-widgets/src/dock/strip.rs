use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{AnyElement, Element, IntoElement, for_each, show};
use smudge_core::geometry::{Color, Rect, Size};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{Memo, RenderEffect, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Align, Direction, Edges, Justify, pct, px};
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_text::label;

use super::widget::{Payload, State, node_at};
use super::{Branch, DockLayout, DockNode, GroupPath, keyboard};
use crate::{ContextMenuExt, ScrollAxis, ScrollExt, scroll, signal};

/// The height of a tab strip, in logical pixels.
pub(super) const STRIP: f32 = 36.0;

struct Geometry {
    viewport: Cell<Rect>,
    active: Cell<Option<Rect>>,
    pending: Cell<bool>,
    measured: RwSignal<()>,
    offset: RwSignal<f32>,
}

impl Geometry {
    fn changed(&self) {
        self.pending.set(true);
        self.measured.set(());
    }

    fn reveal(&self) {
        let Some(active) = self.active.get() else {
            return;
        };
        if !self.pending.replace(false) {
            return;
        }
        let viewport = self.viewport.get();
        let width = viewport.width();
        let left = active.x0 - viewport.x0;
        let right = active.x1 - viewport.x0;
        let current = self.offset.read(|offset| *offset);
        let target = if left < current || active.width() > width {
            left
        } else if right > current + width {
            right - width
        } else {
            current
        }
        .max(0.0);
        if target != current {
            self.offset.set(target);
        }
    }
}

struct TabStrip {
    geometry: Rc<Geometry>,
}

impl Widget for TabStrip {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        if self.geometry.viewport.replace(cx.content) != cx.content {
            self.geometry.changed();
        }
        Panel.paint(cx, out);
    }
}

struct Tab {
    geometry: Rc<Geometry>,
    selected: bool,
    width: Rc<Cell<f32>>,
}

impl Widget for Tab {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        self.width.set(cx.content.width());
        if self.selected {
            if self.geometry.active.replace(Some(cx.rect)) != Some(cx.rect) {
                self.geometry.changed();
            } else if self.geometry.pending.get() {
                self.geometry.measured.set(());
            }
        }
        Panel.paint(cx, out);
    }
}

struct SelectedKey;

fn select(tab: &mut Tab, selected: bool) -> Dirty {
    if tab.selected == selected {
        return Dirty::Clean;
    }
    tab.selected = selected;
    if selected {
        tab.geometry.pending.set(true);
    }
    Dirty::Paint
}

pub(super) fn strip(path: GroupPath, state: Rc<State>, keys: Memo<Vec<String>>) -> AnyElement {
    let geometry = Rc::new(Geometry {
        viewport: Cell::new(Rect::default()),
        active: Cell::new(None),
        pending: Cell::new(true),
        measured: signal(()),
        offset: signal(0.0),
    });
    let reveal = Rc::clone(&geometry);
    RenderEffect::new(move || {
        reveal.measured.get();
        reveal.reveal();
    });
    let clear_state = Rc::clone(&state);
    let tab_geometry = Rc::clone(&geometry);
    let tabs = for_each(
        move || keys.get(),
        Clone::clone,
        move |key| tab(key, &path, Rc::clone(&state), Rc::clone(&tab_geometry)),
    )
    .row()
    .role(Role::TabList)
    .navigate_with_arrows(Direction::Row)
    .height(pct(100.0));
    let offset = geometry.offset.clone();
    Element::new(TabStrip { geometry })
        .column()
        .height(px(STRIP))
        .min_width(px(0.0))
        .shrink(0.0)
        .background(ColorToken::SURFACE)
        .on_drag_over(move |_at| clear_state.clear_zone())
        .border_edges(Edges {
            top: 0.0,
            right: 0.0,
            bottom: 1.0,
            left: 0.0,
        })
        .border_color(ColorToken::BORDER)
        .child(
            scroll(tabs)
                .axis(ScrollAxis::Horizontal)
                .horizontal_offset(offset)
                .width(pct(100.0))
                .height(pct(100.0)),
        )
        .into_any()
}

fn tab(key: String, path: &[Branch], state: Rc<State>, geometry: Rc<Geometry>) -> AnyElement {
    let selected_state = Rc::clone(&state);
    let selected_key = key.clone();
    let selected_path = path.to_vec();
    let selected = Memo::new(move || selected_state.selected(&selected_path, &selected_key));
    let selected_binding = selected.clone();
    let mark = selected.clone();
    let accessible = selected.clone();
    let color = selected.clone();
    let press = Rc::clone(&state);
    let press_key = key.clone();
    let focus = Rc::clone(&state);
    let focus_key = key.clone();
    let name = Rc::clone(&state);
    let name_key = key.clone();
    let text = Rc::clone(&state);
    let text_key = key.clone();
    let closable = Rc::clone(&state);
    let closable_key = key.clone();
    let source = Rc::clone(&state);
    let source_key = key.clone();
    let preview = Rc::clone(&state);
    let preview_key = key.clone();
    let accept = Rc::clone(&state);
    let accept_path = path.to_vec();
    let drop = Rc::clone(&state);
    let drop_path = path.to_vec();
    let drop_key = key.clone();
    let end = Rc::clone(&state);
    let over = Rc::clone(&state);
    let close_state = Rc::clone(&state);
    let close_panel = key.clone();
    let close = show(
        move || closable.closable(&closable_key),
        move || {
            let close = Rc::clone(&close_state);
            let close_key = close_panel.clone();
            let title = Rc::clone(&close_state);
            let title_key = close_panel.clone();
            panel()
                .width(px(20.0))
                .height(px(20.0))
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .hover(|patch| patch.background(ColorToken::SURFACE_HOVER))
                .role(Role::Button)
                .accessible_label(move || format!("Close {}", title.title(&title_key)))
                .keeps_focus()
                .on_press(move || {
                    close.close(&close_key, false);
                })
                .child(label("×").text_color(ColorToken::TEXT_MUTED))
        },
    );
    let width = Rc::new(Cell::new(0.0));
    Element::new(Tab {
        geometry,
        selected: false,
        width: Rc::clone(&width),
    })
    .row()
    .height(pct(100.0))
    .shrink(0.0)
    .align_items(Align::Center)
    .padding_xy(Size {
        width: 12.0,
        height: 6.0,
    })
    .gap(Size {
        width: 6.0,
        height: 0.0,
    })
    .border_edges(Edges {
        top: 0.0,
        right: 0.0,
        bottom: 2.0,
        left: 0.0,
    })
    .border_color(move || {
        if mark.get() {
            ColorValue::Token(ColorToken::ACCENT)
        } else {
            ColorValue::Literal(Color::TRANSPARENT)
        }
    })
    .hover(|patch| patch.background(ColorToken::SURFACE_HOVER))
    .focused(|patch| patch.background(ColorToken::SURFACE_HOVER))
    .role(Role::Tab)
    .accessible_label(move || name.title(&name_key))
    .accessible_selected(move || accessible.get())
    .focusable()
    .focus_target(&state.focus_for(&key).tab)
    .on_unhandled_key({
        let state = Rc::clone(&state);
        let key = key.clone();
        let path = path.to_vec();
        move |event| keyboard::tab_key(&state, &path, &key, event)
    })
    .context_menu(move || keyboard::tab_menu(&state, &key))
    .activate_on_keys()
    .on_press(move || press.activate(&press_key))
    .on_focus(move |inside| {
        if inside {
            focus.activate(&focus_key);
        }
    })
    .drag_source(move || (Rc::clone(&source), source_key.clone()))
    .drag_preview({
        let name = Rc::clone(&preview);
        let name_key = preview_key;
        move || {
            let name = Rc::clone(&name);
            let name_key = name_key.clone();
            panel()
                .row()
                .padding_xy(Size {
                    width: 12.0,
                    height: 6.0,
                })
                .background(ColorToken::SURFACE)
                .border_edges(Edges {
                    top: 0.0,
                    right: 0.0,
                    bottom: 1.0,
                    left: 0.0,
                })
                .border_color(ColorToken::BORDER)
                .child(label(move || name.title(&name_key)).text_color(ColorToken::TEXT))
        }
    })
    .drop_target::<Payload>(
        move |payload: &Payload| {
            Rc::ptr_eq(&payload.0, &accept) && accept.in_group(&accept_path, &payload.1)
        },
        {
            let drop_width = Rc::clone(&width);
            move |(_dock, dragged): Payload, at| {
                let after = at.x * 2.0 > drop_width.get();
                let positions = drop
                    .layout
                    .read(|layout| reorder(layout, &drop_path, &dragged, &drop_key, after));
                if let Some((from, to, count)) = positions {
                    drop.reorder(&dragged, from, to, count);
                }
            }
        },
    )
    .on_drag_end(move |_taken| end.clear_zone())
    .on_drag_over(move |_at| over.clear_zone())
    .bind(SelectedKey, move || selected_binding.get(), select)
    .child(label(move || text.title(&text_key)).text_color(move || {
        ColorValue::Token(if color.get() {
            ColorToken::TEXT
        } else {
            ColorToken::TEXT_MUTED
        })
    }))
    .child(close)
    .into_any()
}

fn reorder(
    layout: &DockLayout,
    path: &[Branch],
    dragged: &str,
    hovered: &str,
    after: bool,
) -> Option<(usize, usize, usize)> {
    let Some(DockNode::Group { panels, .. }) = node_at(layout, path) else {
        return None;
    };
    let from = panels.iter().position(|key| key == dragged)?;
    let mut to = panels.iter().position(|key| key == hovered)?;
    if after {
        to += 1;
    }
    if to > from {
        to -= 1;
    }
    Some((from, to, panels.len()))
}
