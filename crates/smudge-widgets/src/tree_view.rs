//! A tree of rows that folds open and shut: a scroll view over the nodes a
//! tree data model holds, which mounts only the rows near the viewport and
//! asks for a node's children only while the node is open.

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use smudge_core::element::{AnyElement, Element, IntoElement, Value};
use smudge_core::geometry::Color;
use smudge_core::hot;
use smudge_core::input::{Handled, Key};
use smudge_core::reactive::{Memo, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Direction, Length};
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::widget::{Panel, panel};

use crate::signal;
use crate::virtual_list_keyed::keyed;

/// One row of the flattened tree: a node and the place it sits at.
struct Visible<T, K> {
    /// The node the row shows.
    node: T,
    /// The row's stable id.
    key: K,
    /// How deep the node sits, from zero for a root.
    depth: usize,
    /// The key of the node this one is a child of, or none for a root.
    parent: Option<K>,
    /// The row's place among its siblings, from zero.
    sibling_index: usize,
    /// How many siblings the row has, itself included.
    siblings: usize,
}

/// The flattened rows, compared by identity, so the memo that holds them
/// hands a new value on only when a walk produced a new list.
struct Flat<T, K>(Rc<[Visible<T, K>]>);

impl<T, K> PartialEq for Flat<T, K> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// A node the walk has not reached yet, with the place it sits at.
struct Frame<T, K> {
    /// The siblings the node belongs to.
    items: Rc<[T]>,
    /// The node's place among them.
    index: usize,
    /// How deep the node sits.
    depth: usize,
    /// The key of the node's parent, or none for a root.
    parent: Option<K>,
}

/// Walks the tree depth first from the roots, taking a node's children only
/// while the node is open, so a closed branch costs one membership check.
fn flatten<T: Clone, K: Eq + Hash + Clone>(
    roots: &Rc<[T]>,
    children: &dyn Fn(&T) -> Rc<[T]>,
    key: &dyn Fn(&T) -> K,
    open: &HashSet<K>,
) -> Vec<Visible<T, K>> {
    let mut rows = Vec::new();
    let mut stack: Vec<Frame<T, K>> = (0..roots.len())
        .rev()
        .map(|index| Frame {
            items: Rc::clone(roots),
            index,
            depth: 0,
            parent: None,
        })
        .collect();
    while let Some(frame) = stack.pop() {
        let node = frame.items[frame.index].clone();
        let item_key = key(&node);
        let is_open = open.contains(&item_key);
        // The children are taken before the node moves into the row, and
        // kept for after, so a closed node never reaches the closure.
        let kids = is_open.then(|| children(&node));
        rows.push(Visible {
            node,
            key: item_key.clone(),
            depth: frame.depth,
            parent: frame.parent,
            sibling_index: frame.index,
            siblings: frame.items.len(),
        });
        if let Some(kids) = kids {
            stack.extend((0..kids.len()).rev().map(|index| Frame {
                items: Rc::clone(&kids),
                index,
                depth: frame.depth + 1,
                parent: Some(item_key.clone()),
            }));
        }
    }
    rows
}

/// A tree view, which [`tree_view`] creates.
pub struct TreeView<K: Eq + Hash + Clone> {
    /// The keys of the nodes the user opened.
    expanded: RwSignal<HashSet<K>>,
    /// The key of the node the user picked.
    selected: RwSignal<Option<K>>,
    /// The tree's element.
    content: Element<Panel>,
}

impl<K: Eq + Hash + Clone + 'static> TreeView<K> {
    /// Returns the signal holding the keys of the open nodes, so the
    /// application can open or close a branch itself.
    pub fn expanded(&self) -> RwSignal<HashSet<K>> {
        self.expanded.clone()
    }

    /// Returns the signal holding the key of the picked node, so the
    /// application can pick one or read the pick.
    pub fn selected(&self) -> RwSignal<Option<K>> {
        self.selected.clone()
    }

    /// Sets the height, which the scroll view inside needs to virtualize.
    pub fn height(mut self, height: impl Into<Value<Length>>) -> TreeView<K> {
        self.content = self.content.height(height);
        self
    }

    /// Sets the width.
    pub fn width(mut self, width: impl Into<Value<Length>>) -> TreeView<K> {
        self.content = self.content.width(width);
        self
    }
}

impl<K: Eq + Hash + Clone + 'static> IntoElement for TreeView<K> {
    fn into_any(self) -> AnyElement {
        self.content.into_any()
    }
}

/// Creates a tree view over `roots`, whose nodes open their `children`
/// while the user unfolds them, inside a scroll view that mounts only the
/// rows near the viewport.
///
/// Each node is keyed by `key`, which must return a unique, cheap and
/// stable id, exactly as the keyed virtual list requires. `row` builds a
/// row from its node and its depth, from zero for a root, which an
/// application uses to indent; the row is built when its key first enters
/// the window and stays mounted, with its reactive state, until the key
/// leaves the window. `estimate` is the height a row starts from, and must
/// be positive and finite, as the keyed virtual list's is.
///
/// The nodes a closed branch holds are never asked for: `children` runs
/// for a node only while the node is open, so a lazy data model loads a
/// branch when the user unfolds it. The rows come out in depth-first
/// order, a parent before its children, and the tree's flattening is a
/// memo, so a fold re-walks the tree once.
///
/// The view keeps its own state, which [`TreeView::expanded`] and
/// [`TreeView::selected`] hand back to the application: a press on a row
/// picks it, Right opens a closed row or moves to its first child, Left
/// closes an open row or moves to its parent, and Up, Down, Home and End
/// walk the mounted rows. The whole tree takes one Tab stop, which lands
/// on the row with the roving focus, or the picked one, or the first.
///
/// A screen reader hears a tree of tree items: each mounted row reports
/// its level from one, its place among its siblings and how many siblings
/// it has, and the tree reports how many rows the tree shows, the ones
/// scrolled out of view included. A row reports its expanded state as long
/// as it is mounted, and a leaf therefore reports closed; an application
/// that minds the difference can give its row content its own state.
///
/// A row's level, place and sibling count are read when the row mounts. A
/// row that stays mounted while an edit reshapes its siblings keeps the
/// place it was built with until its key leaves the window, like any keyed
/// row; its expanded and picked state follow their signals while it stays
/// mounted. The node type must be [`Clone`], because a walk copies every
/// node it visits out of the lists `roots` and `children` hand back.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_text::label;
/// use smudge_widgets::tree_view;
///
/// #[derive(Clone)]
/// struct Node {
///     id: u64,
///     name: String,
///     children: Vec<Node>,
/// }
///
/// let tree: RwSignal<Rc<[Node]>> = RwSignal::new(Rc::from(vec![
///     Node {
///         id: 1,
///         name: "src".into(),
///         children: vec![Node {
///             id: 2,
///             name: "main.rs".into(),
///             children: Vec::new(),
///         }],
///     },
/// ]));
/// let view = tree_view(
///     move || tree.get().clone(),
///     |node| Rc::from(node.children.as_slice()),
///     |node| node.id,
///     24.0,
///     |node, depth| label(format!("{}{}", "  ".repeat(depth), node.name)),
/// )
/// .height(px(320.0));
/// ```
#[track_caller]
pub fn tree_view<T, K, E>(
    roots: impl Fn() -> Rc<[T]> + 'static,
    children: impl Fn(&T) -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    estimate: f32,
    row: impl Fn(&T, usize) -> E + 'static,
) -> TreeView<K>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
    E: IntoElement,
{
    let roots = hot::wrap(roots);
    let children = Rc::new(hot::wrap1_ref(children));
    let key = Rc::new(hot::wrap1_ref(key));

    let expanded = signal(HashSet::new());
    let selected = signal(None::<K>);
    let focused = signal(None::<K>);

    let flat = {
        let expanded = expanded.clone();
        Memo::new(move || {
            let roots = roots();
            // The set is copied out of the signal before the walk, so the
            // application's closures never run inside the signal's borrow.
            let open = expanded.get();
            Flat(Rc::from(flatten(
                &roots,
                children.as_ref(),
                key.as_ref(),
                &open,
            )))
        })
    };

    let row = Rc::new(RefCell::new(hot::wrap2_ref_mut(row)));
    let rows_view = Rc::new(flat.clone());
    let rows_expanded = expanded.clone();
    let rows_selected = selected.clone();
    let rows_focused = focused.clone();
    let rows = move |visible: &Visible<T, K>| {
        let item_key = visible.key.clone();
        let parent = visible.parent.clone();
        let content = {
            let mut row = row.borrow_mut();
            row(&visible.node, visible.depth)
        };

        let picked = rows_selected.clone();
        let pick_key = item_key.clone();
        let fill_picked = rows_selected.clone();
        let fill_key = item_key.clone();
        let open = rows_expanded.clone();
        let open_key = item_key.clone();
        let held = rows_focused.clone();
        let held_key = item_key.clone();
        let focus_key = item_key.clone();
        let focus_held = rows_focused.clone();
        let press_key = item_key.clone();
        let press_selected = rows_selected.clone();
        let arrow_key = item_key.clone();
        let arrow_parent = parent;
        let arrow_expanded = rows_expanded.clone();
        let arrow_focused = rows_focused.clone();
        let arrow_flat = Rc::clone(&rows_view);
        let arrow_focused_2 = rows_focused.clone();

        panel()
            .role(Role::TreeItem)
            .accessible_level(visible.depth + 1)
            .accessible_position(visible.sibling_index)
            .accessible_set_size(visible.siblings)
            .accessible_expanded(move || open.with(|set| set.contains(&open_key)))
            .accessible_selected(move || picked.with(|pick| pick.as_ref() == Some(&pick_key)))
            .background(move || {
                if fill_picked.with(|pick| pick.as_ref() == Some(&fill_key)) {
                    ColorValue::Token(ColorToken::SURFACE_SELECTED)
                } else {
                    ColorValue::Literal(Color::TRANSPARENT)
                }
            })
            .focusable()
            .focus_when(move || held.with(|held| held.as_ref() == Some(&held_key)))
            .on_focus(move |gained| {
                if gained {
                    focus_held.set(Some(focus_key.clone()));
                }
            })
            .activate_on_keys()
            .on_press(move || press_selected.set(Some(press_key.clone())))
            .on_key(move |event| {
                if !event.pressed {
                    return Handled::No;
                }
                let item_key = &arrow_key;
                match event.key {
                    Key::ArrowRight => {
                        let was_open = arrow_expanded.with(|set| set.contains(item_key));
                        if !was_open {
                            arrow_expanded.update(|set| {
                                set.insert(item_key.clone());
                            });
                            return Handled::Yes;
                        }
                        // The memo recomputes on the read, so an open node
                        // reads the children it took on.
                        let child = arrow_flat.with(|flat| {
                            let mut after = false;
                            for row in flat.0.iter() {
                                if after {
                                    return if row.parent.as_ref() == Some(item_key) {
                                        Some(row.key.clone())
                                    } else {
                                        None
                                    };
                                }
                                after |= &row.key == item_key;
                            }
                            None
                        });
                        match child {
                            Some(child) => {
                                arrow_focused.set(Some(child));
                                Handled::Yes
                            }
                            None => Handled::No,
                        }
                    }
                    Key::ArrowLeft => {
                        let is_open = arrow_expanded.with(|set| set.contains(item_key));
                        if is_open {
                            arrow_expanded.update(|set| {
                                set.remove(item_key);
                            });
                            Handled::Yes
                        } else if let Some(parent) = arrow_parent.clone() {
                            arrow_focused_2.set(Some(parent));
                            Handled::Yes
                        } else {
                            Handled::No
                        }
                    }
                    _ => Handled::No,
                }
            })
            .child(content)
    };

    let list_view = Rc::new(flat.clone());
    let list = keyed(
        move || list_view.with(|flat| Rc::clone(&flat.0)),
        |visible: &Visible<T, K>| visible.key.clone(),
        estimate,
        rows,
        false,
        None,
    )
    .grow(1.0);

    TreeView {
        expanded: expanded.clone(),
        selected: selected.clone(),
        content: panel()
            .column()
            .role(Role::Tree)
            .accessible_set_size(move || flat.with(|flat| flat.0.len()))
            .navigate_with_arrows(Direction::Column)
            .child(list),
    }
}
