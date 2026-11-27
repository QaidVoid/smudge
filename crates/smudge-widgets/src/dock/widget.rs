use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use smudge_core::element::{AnyElement, Element, IntoElement, KeepAlive, Value, for_each};
use smudge_core::geometry::Point;
use smudge_core::hot;
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{Memo, Owner, RenderEffect, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Direction, Length, pct, px};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};

use super::{Branch, DockLayout, DockNode, GroupPath, Zone, zone};
use crate::{SplitExt, signal, split};

type CloseHandler = Box<dyn FnMut(&str) -> bool>;
type EmptyBuilder = Box<dyn Fn() -> AnyElement>;

/// What one dock's tabs drag around: the dock that started the drag, which
/// only its own groups accept, and the panel key the drag carries.
pub(super) type Payload = (Rc<State>, String);

/// A panel's title, close policy and content builder for a [`dock`].
///
/// The dock calls its panel factory and this content builder once while the
/// key remains in the layout, including while inactive or moved. Closing a
/// panel disposes its content. Opening the key again builds a fresh panel.
///
/// ```
/// use smudge_text::label;
/// use smudge_widgets::dock::DockPanel;
///
/// let files = DockPanel::new("Files", || label("Project files")).closable(false);
/// ```
pub struct DockPanel {
    title: String,
    closable: bool,
    content: Box<dyn FnOnce() -> AnyElement>,
}

impl DockPanel {
    /// Creates a closable panel named `title` with content built on mounting.
    pub fn new<E: IntoElement>(
        title: impl Into<String>,
        content: impl FnOnce() -> E + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            closable: true,
            content: Box::new(move || content().into_any()),
        }
    }

    /// Sets whether the panel offers a close button. The application can
    /// still remove a nonclosable panel by editing its layout directly.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

struct Options {
    close: RefCell<Option<CloseHandler>>,
    empty: RefCell<EmptyBuilder>,
}

struct PanelInfo {
    title: String,
    closable: bool,
}

pub(super) struct State {
    pub(super) layout: RwSignal<DockLayout>,
    zone: RwSignal<Option<(Rc<zone::Frame>, Zone)>>,
    hover: RefCell<Option<Point>>,
    factory: Rc<dyn Fn(&str) -> DockPanel>,
    info: RwSignal<HashMap<String, PanelInfo>>,
    options: Rc<Options>,
}

impl State {
    pub(super) fn title(&self, key: &str) -> String {
        self.info.with(|info| {
            info.get(key)
                .map_or_else(|| key.to_owned(), |panel| panel.title.clone())
        })
    }

    pub(super) fn closable(&self, key: &str) -> bool {
        self.info
            .with(|info| info.get(key).is_some_and(|panel| panel.closable))
    }

    pub(super) fn selected(&self, path: &[Branch], key: &str) -> bool {
        self.layout.with(|layout| {
            matches!(node_at(layout, path), Some(DockNode::Group { active, .. }) if active == key)
        })
    }

    pub(super) fn in_group(&self, path: &[Branch], key: &str) -> bool {
        self.layout.read(|layout| {
            matches!(
                node_at(layout, path),
                Some(DockNode::Group { panels, .. }) if panels.iter().any(|panel| panel == key)
            )
        })
    }

    pub(super) fn clear_zone(&self) {
        *self.hover.borrow_mut() = None;
        if self.zone.read(Option::is_some) {
            self.zone.set(None);
        }
    }

    /// Hides the zone rectangle `frame` shows, leaving the hover position
    /// the group still tracks, so the next delivery can show one again.
    pub(super) fn hide_zone(&self, frame: &Rc<zone::Frame>) {
        let shown = self
            .zone
            .read(|current| matches!(current, Some((shown, _)) if Rc::ptr_eq(shown, frame)));
        if shown {
            self.zone.set(None);
        }
    }

    pub(super) fn show_zone(&self, frame: &Rc<zone::Frame>, zone: Zone) {
        let unchanged = self.zone.read(|current| {
            matches!(current, Some((shown, shown_zone))
                if Rc::ptr_eq(shown, frame) && *shown_zone == zone)
        });
        if !unchanged {
            self.zone.set(Some((Rc::clone(frame), zone)));
        }
    }

    pub(super) fn activate(&self, key: &str) {
        let selected = self.layout.read(|layout| is_active(layout.root(), key));
        if !selected {
            self.layout.update(|layout| {
                let _ = layout.activate(key);
            });
        }
    }

    pub(super) fn close(&self, key: &str) {
        if !self
            .info
            .read(|info| info.get(key).is_some_and(|panel| panel.closable))
        {
            return;
        }
        let allowed = self
            .options
            .close
            .borrow_mut()
            .as_mut()
            .is_none_or(|close| close(key));
        if allowed {
            self.layout.update(|layout| {
                let _ = layout.close(key);
            });
        }
    }

    fn build_panel(&self, key: &str) -> AnyElement {
        let definition = (self.factory)(key);
        let title = definition.title.clone();
        self.info.update(|info| {
            info.insert(
                key.to_owned(),
                PanelInfo {
                    title: definition.title,
                    closable: definition.closable,
                },
            );
        });
        let info = self.info.clone();
        let key = key.to_owned();
        Owner::on_cleanup(move || {
            info.update(|info| {
                info.remove(&key);
            });
        });
        panel()
            .column()
            .grow(1.0)
            .min_height(px(0.0))
            .min_width(px(0.0))
            .role(Role::TabPanel)
            .accessible_label(title)
            .style_children(|style| {
                style.layout.grow = 1.0;
                if style.layout.min_height == Length::Auto {
                    style.layout.min_height = Length::Px(0.0);
                }
            })
            .child((definition.content)())
            .into_any()
    }
}

/// A docking workspace, created by [`dock`] and configured with [`DockExt`].
///
/// Give the element a size or put it in a parent that sizes it. Panels fill
/// their group's content area, and split dividers resize the groups.
pub struct Dock {
    options: Rc<Options>,
}

impl Widget for Dock {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        Panel.paint(cx, out);
    }
}

/// Creates a workspace from an application-owned layout and panel factory.
///
/// Each key is built once while present, even when another tab is active,
/// the panel moves to another group, or the application replaces the layout.
/// The factory and its content builder run under the panel's retained owner.
/// Closing a key disposes its state, and reopening it builds it again.
///
/// A tab press activates it. Left and Right, Home and End navigate the strip
/// as in [`tabs`](crate::tabs), and each strip takes one Tab stop. Close
/// buttons accept pointer and accessibility activation without adding arrow
/// destinations. The strips scroll sideways and reveal newly active tabs
/// after their geometry has been measured, without moving keyboard focus.
/// Divider drags and keys write the layout's split ratios. Each tab drags:
/// a drag along its own strip reorders the tabs, and a release over a group
/// moves the panel there, joining the group or splitting it toward the edge
/// the release lands on, with a translucent rectangle showing where the
/// panel would go. Escape, a release over nothing, and a release on the
/// panel's own group's center leave the layout as it was. Layout edits made
/// directly by the application do not call [`DockExt::on_close`]. An empty
/// layout draws no content unless [`DockExt::empty`] supplies it.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_core::style::px;
/// use smudge_text::label;
/// use smudge_widgets::dock::{DockExt, DockLayout, DockPanel, dock};
///
/// let mut initial = DockLayout::default();
/// initial.add("editor", &[]).expect("the empty root accepts a panel");
/// let workspace = dock(RwSignal::new(initial), |key| {
///     let key = key.to_owned();
///     DockPanel::new("Editor", move || label(key))
/// })
/// .width(px(640.0))
/// .height(px(480.0))
/// .empty(|| label("Open a file to begin"));
/// ```
#[track_caller]
pub fn dock(
    layout: RwSignal<DockLayout>,
    factory: impl Fn(&str) -> DockPanel + 'static,
) -> Element<Dock> {
    let factory: Rc<dyn Fn(&str) -> DockPanel> = Rc::new(hot::wrap1_ref(factory));
    let options = Rc::new(Options {
        close: RefCell::new(None),
        empty: RefCell::new(Box::new(|| panel().into_any())),
    });
    let mounted_options = Rc::clone(&options);
    let content = for_each(
        || vec![()],
        |()| (),
        move |()| {
            let state = Rc::new(State {
                layout: layout.clone(),
                zone: signal(None),
                hover: RefCell::new(None),
                factory: Rc::clone(&factory),
                info: signal(HashMap::new()),
                options: Rc::clone(&mounted_options),
            });
            let watch = Rc::clone(&state);
            RenderEffect::new(move || {
                watch.layout.with(|_| ());
                watch.clear_zone();
            });
            let build = Rc::clone(&state);
            let store = KeepAlive::new(move |key| build.build_panel(key));
            position(Vec::new(), state, store)
        },
    );
    Element::new(Dock { options })
        .column()
        .min_width(px(0.0))
        .min_height(px(0.0))
        .child(fill(content))
}

/// Configures panel closing and empty content for a [`dock`].
pub trait DockExt: Sized {
    /// Checks a close-button request before removing the panel. Returning
    /// `false` leaves it open, for example while unsaved work is confirmed.
    /// Without a handler, a close button removes the panel immediately.
    fn on_close(self, handler: impl FnMut(&str) -> bool + 'static) -> Self;

    /// Builds content while the layout is empty. It is unmounted when a
    /// panel opens and built anew the next time the layout becomes empty.
    fn empty<E: IntoElement>(self, build: impl Fn() -> E + 'static) -> Self;
}

struct CloseKey;
struct EmptyKey;

impl DockExt for Element<Dock> {
    fn on_close(self, handler: impl FnMut(&str) -> bool + 'static) -> Self {
        let handler: CloseHandler = Box::new(hot::wrap1_ref_mut(handler));
        self.bind(CloseKey, Value::Static(handler), |dock, handler| {
            *dock.options.close.borrow_mut() = Some(handler);
            Dirty::Clean
        })
    }

    fn empty<E: IntoElement>(self, build: impl Fn() -> E + 'static) -> Self {
        let build = hot::wrap(build);
        let build: EmptyBuilder = Box::new(move || build().into_any());
        self.bind(EmptyKey, Value::Static(build), |dock, build| {
            *dock.options.empty.borrow_mut() = build;
            Dirty::Clean
        })
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Kind {
    Empty,
    Group,
    Row,
    Column,
}

fn position(path: GroupPath, state: Rc<State>, store: KeepAlive) -> AnyElement {
    let shape_state = Rc::clone(&state);
    let shape_path = path.clone();
    let kind = Memo::new(move || {
        shape_state
            .layout
            .with(|layout| match node_at(layout, &shape_path) {
                Some(DockNode::Group { .. }) => Some(Kind::Group),
                Some(DockNode::Split {
                    direction: Direction::Row,
                    ..
                }) => Some(Kind::Row),
                Some(DockNode::Split {
                    direction: Direction::Column,
                    ..
                }) => Some(Kind::Column),
                None if shape_path.is_empty() => Some(Kind::Empty),
                None => None,
            })
    });
    fill(for_each(
        move || kind.get().into_iter().collect(),
        |kind| *kind,
        move |kind| match kind {
            Kind::Empty => (state.options.empty.borrow())(),
            Kind::Group => group(path.clone(), Rc::clone(&state), store.clone()),
            Kind::Row | Kind::Column => split_group(
                path.clone(),
                Rc::clone(&state),
                store.clone(),
                if kind == Kind::Row {
                    Direction::Row
                } else {
                    Direction::Column
                },
            ),
        },
    ))
    .into_any()
}

fn group(path: GroupPath, state: Rc<State>, store: KeepAlive) -> AnyElement {
    let items_state = Rc::clone(&state);
    let items_path = path.clone();
    let keys = Memo::new(move || {
        items_state
            .layout
            .with(|layout| match node_at(layout, &items_path) {
                Some(DockNode::Group { panels, .. }) => panels.clone(),
                _ => Vec::new(),
            })
    });
    let title_state = Rc::clone(&state);
    let title_path = path.clone();
    let strip = super::strip::strip(path.clone(), Rc::clone(&state), keys.clone());
    let content_state = Rc::clone(&state);
    let content_path = path.clone();
    let content = for_each(
        move || keys.get(),
        Clone::clone,
        move |key| {
            let state = Rc::clone(&content_state);
            let path = content_path.clone();
            fill(store.slot(key.clone())).hidden(move || !state.selected(&path, &key))
        },
    );
    let frame = Rc::new(zone::Frame::new());
    let over_state = Rc::clone(&state);
    let accepts_state = Rc::clone(&state);
    let accepts_frame = Rc::clone(&frame);
    let accepts_path = path.clone();
    let leave_state = Rc::clone(&state);
    let drop_state = Rc::clone(&state);
    let drop_frame = Rc::clone(&frame);
    let drop_path = path;
    fill(Element::new(zone::GroupFrame {
        frame: Rc::clone(&frame),
    }))
    .role(Role::Group)
    .accessible_label(move || {
        let active = title_state
            .layout
            .with(|layout| match node_at(layout, &title_path) {
                Some(DockNode::Group { active, .. }) => active.clone(),
                _ => String::new(),
            });
        title_state.title(&active)
    })
    .on_drag_over(move |at| {
        *over_state.hover.borrow_mut() = Some(at);
    })
    .on_drag_leave(move || leave_state.clear_zone())
    .drop_target::<Payload>(
        move |payload: &Payload| {
            if !Rc::ptr_eq(&payload.0, &accepts_state) {
                return false;
            }
            let (present, sole) = accepts_state.layout.read(|layout| {
                (
                    layout.contains(&payload.1),
                    matches!(
                        node_at(layout, &accepts_path),
                        Some(DockNode::Group { panels, .. })
                            if panels.len() == 1 && panels[0] == payload.1
                    ),
                )
            });
            if !present {
                return false;
            }
            if let Some(at) = *accepts_state.hover.borrow() {
                let zone = zone::zone_of(&accepts_frame, at);
                if zone == Zone::Center || !sole {
                    accepts_state.show_zone(&accepts_frame, zone);
                } else {
                    accepts_state.hide_zone(&accepts_frame);
                }
            }
            true
        },
        move |(_dock, key): Payload, at| {
            let zone = zone::zone_of(&drop_frame, at);
            drop_state.layout.update(|layout| {
                let _ = layout.move_panel(&key, &drop_path, zone);
            });
        },
    )
    .child(strip)
    .child(fill(content))
    .child(zone::highlight(state.zone.clone(), frame))
    .into_any()
}

fn split_group(
    path: GroupPath,
    state: Rc<State>,
    store: KeepAlive,
    direction: Direction,
) -> AnyElement {
    let canonical_state = Rc::clone(&state);
    let canonical_path = path.clone();
    let canonical = Memo::new(move || {
        canonical_state
            .layout
            .with(|layout| match node_at(layout, &canonical_path) {
                Some(DockNode::Split {
                    direction: axis,
                    ratio,
                    ..
                }) if *axis == direction => Some(*ratio),
                _ => None,
            })
    });
    let ratio = signal(canonical.get().unwrap_or(0.5));
    let proxy = ratio.clone();
    let mut synchronized = ratio.read(|value| *value);
    let edits = Rc::clone(&state);
    let edits_path = path.clone();
    RenderEffect::new(move || {
        let Some(current) = canonical.get() else {
            return;
        };
        let edited = proxy.get();
        if current != synchronized {
            synchronized = current;
            if edited != current {
                proxy.set(current);
            }
        } else if edited != current {
            synchronized = edited;
            edits.layout.update(|layout| {
                let _ = layout.set_ratio(&edits_path, edited);
            });
        }
    });
    let first_path = child_path(&path, Branch::First);
    let second_path = child_path(&path, Branch::Second);
    let first = position(first_path, Rc::clone(&state), store.clone());
    let second = position(second_path, state, store);
    split(direction, first, second)
        .ratio(ratio)
        .min_width(px(0.0))
        .min_height(px(0.0))
        .grow(1.0)
        .into_any()
}

fn child_path(path: &[Branch], branch: Branch) -> GroupPath {
    let mut path = path.to_vec();
    path.push(branch);
    path
}

pub(super) fn node_at<'a>(layout: &'a DockLayout, path: &[Branch]) -> Option<&'a DockNode> {
    let mut node = layout.root()?;
    for branch in path {
        let DockNode::Split { first, second, .. } = node else {
            return None;
        };
        node = match branch {
            Branch::First => first,
            Branch::Second => second,
        };
    }
    Some(node)
}

fn is_active(node: Option<&DockNode>, key: &str) -> bool {
    match node {
        Some(DockNode::Group { active, .. }) => active == key,
        Some(DockNode::Split { first, second, .. }) => {
            is_active(Some(first), key) || is_active(Some(second), key)
        }
        None => false,
    }
}

fn fill<W: Widget>(element: Element<W>) -> Element<W> {
    element
        .column()
        .grow(1.0)
        .width(pct(100.0))
        .min_width(px(0.0))
        .min_height(px(0.0))
}
