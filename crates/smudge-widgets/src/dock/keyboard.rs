use std::rc::Rc;

use smudge_core::geometry::Point;
use smudge_core::input::{Handled, Key, KeyEvent, Modifiers};
use smudge_core::reactive::prelude::*;

use super::widget::{State, after_removal, node_at};
use super::{Branch, DockLayout, DockNode, GroupPath, Zone, floating};
use crate::{MenuItem, menu_item};

struct Group<'a> {
    path: Option<GroupPath>,
    panels: &'a [String],
    active: &'a str,
}

fn groups(layout: &DockLayout) -> Vec<Group<'_>> {
    fn visit<'a>(node: &'a DockNode, path: &mut GroupPath, out: &mut Vec<Group<'a>>) {
        match node {
            DockNode::Group { panels, active } => out.push(Group {
                path: Some(path.clone()),
                panels,
                active,
            }),
            DockNode::Split { first, second, .. } => {
                path.push(Branch::First);
                visit(first, path, out);
                path.pop();
                path.push(Branch::Second);
                visit(second, path, out);
                path.pop();
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = layout.root() {
        visit(root, &mut Vec::new(), &mut out);
    }
    out.extend(layout.floating().iter().map(|group| Group {
        path: None,
        panels: &[],
        active: group.panel(),
    }));
    out
}

pub(super) fn after_close(layout: &DockLayout, key: &str) -> Option<String> {
    let groups = groups(layout);
    let at = groups
        .iter()
        .position(|group| group.active == key || group.panels.iter().any(|panel| panel == key))?;
    let group = &groups[at];
    let next = if group.active != key {
        Some(group.active)
    } else {
        after_removal(group.panels, key)
    };
    next.or_else(|| {
        groups
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|at| groups.get(at)))
            .map(|group| group.active)
    })
    .map(str::to_owned)
}

fn is_close(event: &KeyEvent) -> bool {
    event.pressed
        && !event.repeat
        && event.modifiers
            == Modifiers {
                control: true,
                ..Modifiers::default()
            }
        && matches!(&event.key, Key::Character(key) if key.eq_ignore_ascii_case("w"))
}

pub(super) fn group_key(
    state: &State,
    path: Option<&[Branch]>,
    floating: Option<&str>,
    event: &KeyEvent,
) -> Handled {
    if event.key == Key::Function(6)
        && !event.repeat
        && !event.modifiers.control
        && !event.modifiers.alt
        && !event.modifiers.meta
    {
        let next = state.layout.read(|layout| {
            let groups = groups(layout);
            let here = groups.iter().position(|group| match group.path.as_deref() {
                Some(at) => path == Some(at),
                None => floating == Some(group.active),
            });
            let group = match (here, event.modifiers.shift) {
                (Some(at), false) => groups.get(at + 1).or_else(|| groups.first()),
                (Some(at), true) => at
                    .checked_sub(1)
                    .and_then(|at| groups.get(at))
                    .or_else(|| groups.last()),
                (None, false) => groups.first(),
                (None, true) => groups.last(),
            }?;
            Some(group.active.to_owned())
        });
        if let Some(next) = next {
            let target = state.focus_for(&next);
            target.content.request_focus(Some(&target.tab));
            return Handled::Yes;
        }
    }
    Handled::No
}

pub(super) fn close_key(state: &State, key: &str, event: &KeyEvent) -> Handled {
    if is_close(event) && state.close(key, true) {
        Handled::Yes
    } else {
        Handled::No
    }
}

pub(super) fn tab_key(
    state: &State,
    path: Option<&[Branch]>,
    key: &str,
    event: &KeyEvent,
) -> Handled {
    if is_close(event) {
        return close_key(state, key, event);
    }
    if event.modifiers
        != (Modifiers {
            alt: true,
            ..Modifiers::default()
        })
    {
        return Handled::No;
    }
    let forward = match event.key {
        Key::ArrowRight => true,
        Key::ArrowLeft => false,
        _ => return Handled::No,
    };
    let position = state.layout.read(|layout| {
        let Some(DockNode::Group { panels, .. }) = node_at(layout, path?) else {
            return None;
        };
        let from = panels.iter().position(|panel| panel == key)?;
        let to = if forward {
            (from + 1).min(panels.len() - 1)
        } else {
            from.saturating_sub(1)
        };
        Some((from, to, panels.len()))
    });
    if let Some((from, to, count)) = position {
        state.reorder(key, from, to, count);
    }
    Handled::Yes
}

pub(super) fn tab_menu(
    state: &Rc<State>,
    key: &str,
    window: Option<&Rc<floating::Window>>,
) -> Vec<MenuItem> {
    let mut items = state.layout.read(|layout| {
        let mut items = Vec::new();
        for group in groups(layout)
            .into_iter()
            .filter(|group| group.path.is_some())
        {
            if group.panels.iter().any(|panel| panel == key) {
                if group.panels.len() > 1 {
                    for (name, zone) in [
                        ("Move left", Zone::Left),
                        ("Move right", Zone::Right),
                        ("Move above", Zone::Top),
                        ("Move below", Zone::Bottom),
                    ] {
                        items.push(move_item(state, key, key, zone, name.to_owned()));
                    }
                }
            } else {
                let title = state.title(group.active);
                items.push(move_item(
                    state,
                    key,
                    group.active,
                    Zone::Center,
                    format!("Move into group with {title}"),
                ));
            }
        }
        items
    });
    if let Some(window) = window {
        if state.layout.read(|layout| layout.root().is_none()) {
            let state = Rc::clone(state);
            let key = key.to_owned();
            items.push(menu_item("Dock", move || {
                state.move_panel(&key, &[], Zone::Center)
            }));
        }
        for (name, resize) in [
            ("Move floating panel", false),
            ("Resize floating panel", true),
        ] {
            let window = Rc::clone(window);
            items.push(menu_item(name, move || window.begin(resize)));
        }
    } else {
        let state = Rc::clone(state);
        let key = key.to_owned();
        items.push(menu_item("Float", move || {
            state.float(&key, Point { x: 58.0, y: 64.0 })
        }));
    }
    if state.closable(key) {
        let state = Rc::clone(state);
        let key = key.to_owned();
        items.push(
            menu_item("Close", move || {
                state.close(&key, true);
            })
            .shortcut("Ctrl+W"),
        );
    }
    items
}

fn move_item(state: &Rc<State>, key: &str, anchor: &str, zone: Zone, name: String) -> MenuItem {
    let state = Rc::clone(state);
    let key = key.to_owned();
    let anchor = anchor.to_owned();
    menu_item(name, move || {
        let path = state.layout.read(|layout| {
            groups(layout)
                .into_iter()
                .find(|group| group.panels.contains(&anchor))
                .and_then(|group| group.path)
        });
        if let Some(path) = path {
            state.move_panel(&key, &path, zone);
        }
    })
}
