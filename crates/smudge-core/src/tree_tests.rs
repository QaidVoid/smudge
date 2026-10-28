use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::geometry::{Color, Point, Rect, Size};
use crate::paint::PaintOp;
use crate::reactive::{self, Owner, RenderEffect, prelude::*};
use crate::style::{Edges, Length, Position, Style};
use crate::tree::{Dirty, PaintedBounds, WidgetId, WidgetTree};
use crate::widget::{PaintCx, Widget};

/// A widget with no behavior, for filling tree nodes.
struct Empty;

impl Widget for Empty {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A widget with state, for checking that the tree hands back the widget it
/// stores.
struct Counter(u32);

impl Widget for Counter {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// Clears a node's dirty state, as a frame does once it has laid out and
/// painted the node.
fn clean(tree: &mut WidgetTree, id: WidgetId) {
    tree.nodes[id].dirty = Dirty::Clean;
    tree.paint_queue.clear();
    tree.layout_dirty = false;
}

#[test]
fn removing_an_absolute_child_keeps_its_parent_in_the_layout_tree() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let mut spacer = Style::default();
    spacer.layout.width = Length::px(15.0);
    let _spacer = tree.insert(Some(root), Box::new(Empty), spacer);
    let holder = tree.insert(Some(root), Box::new(Empty), Style::default());
    let mut absolute = Style::default();
    absolute.layout.position = Position::Absolute;
    absolute.layout.inset = Edges {
        top: Length::px(2.0),
        right: Length::Auto,
        bottom: Length::Auto,
        left: Length::px(3.0),
    };
    let child = tree.insert(Some(holder), Box::new(Empty), absolute);

    let viewport = Size {
        width: 100.0,
        height: 80.0,
    };
    assert!(tree.layout(viewport));
    assert_eq!(
        tree.rect(holder).map(|rect| rect.x0),
        Some(15.0),
        "the empty holder sits after the spacer in the row"
    );

    tree.remove(child);
    assert!(tree.layout(viewport));
    tree.debug_assert_consistent();
    assert_eq!(
        tree.rect(holder).map(|rect| rect.x0),
        Some(15.0),
        "the holder's layout node stays reachable, so the pass still \
         positions it"
    );
}

#[test]
fn removing_a_subtree_removes_every_descendant() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let middle = tree.insert(Some(root), Box::new(Empty), Style::default());
    let leaf = tree.insert(Some(middle), Box::new(Empty), Style::default());
    tree.debug_assert_consistent();

    tree.remove(middle);

    assert!(tree.style(middle).is_none(), "the removed node is gone");
    assert!(tree.style(leaf).is_none(), "its descendant is gone too");
    assert!(tree.style(root).is_some(), "the parent survives");
    assert!(
        tree.nodes[root].children.is_empty(),
        "the parent no longer lists the subtree"
    );
    tree.debug_assert_consistent();

    tree.remove(middle);
    assert!(
        tree.style(root).is_some(),
        "removing a stale id does nothing"
    );
}

#[test]
fn removing_a_node_runs_its_owner_cleanup() {
    let mut tree = WidgetTree::new();
    let id = tree.insert(None, Box::new(Empty), Style::default());

    let cleaned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cleaned);
    tree.owner(id).expect("the node is live").with(|| {
        Owner::on_cleanup(move || flag.store(true, Ordering::Relaxed));
    });
    assert!(!cleaned.load(Ordering::Relaxed));

    tree.remove(id);

    assert!(
        cleaned.load(Ordering::Relaxed),
        "removal cleans the node's owner up"
    );
}

#[test]
fn removing_a_node_stops_its_effects() {
    reactive::init();
    let mut tree = WidgetTree::new();
    let id = tree.insert(None, Box::new(Empty), Style::default());

    let (value, set_value) = signal_local(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&runs);
    tree.owner(id).expect("the node is live").with(|| {
        RenderEffect::new(move || {
            value.get();
            counter.set(counter.get() + 1);
        });
    });
    assert_eq!(runs.get(), 1, "a render effect runs when it is created");

    set_value.set(1);
    reactive::flush();
    assert_eq!(
        runs.get(),
        2,
        "a write reaches the effect at the next flush"
    );

    tree.remove(id);
    set_value.set(2);
    reactive::flush();
    assert_eq!(
        runs.get(),
        2,
        "removal cleans the owner up, which stops the effect"
    );
}

#[test]
fn removing_a_painted_node_damages_its_painted_bounds() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let child = tree.insert(Some(root), Box::new(Empty), Style::default());

    let bounds = Rect::new(4.0, 8.0, 24.0, 18.0);
    let clip = Rect::new(0.0, 0.0, 40.0, 30.0);
    let painted = PaintedBounds {
        bounds,
        clip,
        shift: Point::default(),
    };
    tree.nodes[child].painted = Some(painted);

    tree.remove(child);

    assert_eq!(
        tree.pending_damage,
        vec![painted],
        "the area the node painted last frame needs a repaint"
    );
}

#[test]
fn an_unchanged_paint_value_leaves_the_node_clean() {
    let mut tree = WidgetTree::new();
    let id = tree.insert(None, Box::new(Empty), Style::default());
    clean(&mut tree, id);

    tree.apply_paint(id, |_| {});

    assert_eq!(tree.nodes[id].dirty, Dirty::Clean);
    assert!(!tree.layout_dirty, "a paint change never needs layout");
    assert!(tree.paint_queue.is_empty(), "a clean node is not queued");

    tree.apply_paint(id, |paint| paint.background = Some(Color::BLACK.into()));

    assert_eq!(tree.nodes[id].dirty, Dirty::Paint);
    assert!(!tree.layout_dirty);
    assert_eq!(tree.paint_queue, vec![id]);
    assert_eq!(
        tree.style(id).expect("the node is live").paint.background,
        Some(Color::BLACK.into())
    );
}

#[test]
fn a_changed_layout_value_marks_the_node_for_layout() {
    let mut tree = WidgetTree::new();
    let id = tree.insert(None, Box::new(Empty), Style::default());
    clean(&mut tree, id);

    tree.apply_layout(id, |_| {});

    assert_eq!(tree.nodes[id].dirty, Dirty::Clean);
    assert!(!tree.layout_dirty);

    tree.apply_layout(id, |layout| layout.width = Length::px(40.0));

    assert_eq!(tree.nodes[id].dirty, Dirty::Layout);
    assert!(tree.layout_dirty, "the next frame must lay the tree out");
    assert_eq!(tree.paint_queue, vec![id]);
    tree.debug_assert_consistent();
}

#[test]
fn mark_never_lowers_a_dirty_flag() {
    let mut tree = WidgetTree::new();
    let id = tree.insert(None, Box::new(Empty), Style::default());

    tree.mark(id, Dirty::Layout);
    tree.mark(id, Dirty::Paint);

    assert_eq!(tree.nodes[id].dirty, Dirty::Layout);
    assert_eq!(
        tree.paint_queue,
        vec![id],
        "a node is queued once, when it leaves the clean state"
    );
}

#[test]
fn widget_mut_returns_the_widget_only_as_its_own_type() {
    let mut tree = WidgetTree::new();
    let id = tree.insert(None, Box::new(Counter(1)), Style::default());

    tree.widget_mut::<Counter>(id)
        .expect("the stored widget is a counter")
        .0 = 2;

    assert_eq!(
        tree.widget_mut::<Counter>(id).map(|counter| counter.0),
        Some(2),
        "the change reached the stored widget"
    );
    assert!(
        tree.widget_mut::<Empty>(id).is_none(),
        "asking for another widget type finds nothing"
    );
}

#[test]
fn set_children_reorders_the_children_in_both_trees() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let first = tree.insert(Some(root), Box::new(Empty), Style::default());
    let second = tree.insert(Some(root), Box::new(Empty), Style::default());
    clean(&mut tree, root);

    tree.set_children(root, vec![second, first]);

    assert_eq!(tree.nodes[root].children, vec![second, first]);
    assert_eq!(tree.nodes[root].dirty, Dirty::Layout);
    tree.debug_assert_consistent();
}

/// A widget that reports the role and value its fields name, to check what
/// the projection reads from a widget.
struct Described {
    role: crate::semantics::Role,
    value: &'static str,
}

impl Widget for Described {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<crate::semantics::Semantics> {
        Some(crate::semantics::Semantics {
            role: self.role,
            label: None,
            value: Some(self.value.to_owned()),
        })
    }
}

/// Mounts a described root with a described child and drains the changes the
/// mounts queued.
fn described_pair() -> (WidgetTree, WidgetId, WidgetId) {
    let mut tree = WidgetTree::new();
    let root = tree.insert(
        None,
        Box::new(Described {
            role: crate::semantics::Role::Group,
            value: "root",
        }),
        Style::default(),
    );
    let child = tree.insert(
        Some(root),
        Box::new(Described {
            role: crate::semantics::Role::Button,
            value: "child",
        }),
        Style::default(),
    );
    tree.take_a11y_changes();
    (tree, root, child)
}

#[test]
fn a_marked_node_is_taken_once_and_only_once() {
    let (mut tree, _root, child) = described_pair();
    tree.nodes[child].dirty = Dirty::Clean;

    tree.mark(child, Dirty::Paint);
    tree.mark(child, Dirty::Paint);
    assert!(tree.has_a11y_changes());

    let changes = tree.take_a11y_changes();
    assert_eq!(changes.changed, vec![child]);
    assert!(!changes.reset);
    assert!(!tree.has_a11y_changes(), "a take drains the changes");
}

#[test]
fn a_stale_id_drops_out_of_the_changes() {
    let (mut tree, root, child) = described_pair();
    tree.remove(child);
    tree.mark(root, Dirty::Paint);

    let changes = tree.take_a11y_changes();
    assert_eq!(changes.changed, vec![root], "the removed child is gone");
}

#[test]
fn a_hidden_node_leaves_the_changes_to_its_parent() {
    let (mut tree, root, child) = described_pair();
    tree.apply_layout(child, |layout| layout.hidden = true);

    let changes = tree.take_a11y_changes();
    assert!(
        changes.changed.contains(&root),
        "the parent is rebuilt so its child list loses the hidden node"
    );
    assert!(
        !changes.changed.contains(&child),
        "a hidden node is never rebuilt"
    );
}

#[test]
fn an_overflowing_change_queue_collapses_to_a_reset() {
    let (mut tree, _root, child) = described_pair();
    for _ in 0..5000 {
        tree.nodes[child].dirty = Dirty::Clean;
        tree.mark(child, Dirty::Paint);
    }

    let changes = tree.take_a11y_changes();
    assert!(
        changes.reset,
        "an overflowing queue asks for a full rebuild"
    );
    assert!(changes.changed.is_empty());
    assert!(!tree.has_a11y_changes());
}

#[test]
fn the_projection_walks_through_unprojected_nodes() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let group = tree.insert(
        Some(root),
        Box::new(Described {
            role: crate::semantics::Role::Group,
            value: "group",
        }),
        Style::default(),
    );
    let leaf = tree.insert(
        Some(group),
        Box::new(Described {
            role: crate::semantics::Role::Button,
            value: "leaf",
        }),
        Style::default(),
    );

    let nodes = tree.semantics_nodes();
    assert_eq!(
        nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
        vec![group, leaf],
        "the unprojected root is left out"
    );
    assert_eq!(nodes[0].parent, None);
    assert_eq!(nodes[0].children, vec![leaf]);
    assert_eq!(nodes[1].parent, Some(group));
    assert_eq!(nodes[1].value.as_deref(), Some("leaf"));

    let single = tree.semantics_node(leaf).expect("the leaf projects");
    assert_eq!(single.parent, Some(group));
    assert!(tree.semantics_node(root).is_none(), "the root has no role");
}

#[test]
fn a_hidden_subtree_drops_out_of_the_projection() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let group = tree.insert(
        Some(root),
        Box::new(Described {
            role: crate::semantics::Role::Group,
            value: "group",
        }),
        Style::default(),
    );
    let leaf = tree.insert(
        Some(group),
        Box::new(Described {
            role: crate::semantics::Role::Button,
            value: "leaf",
        }),
        Style::default(),
    );
    tree.apply_layout(group, |layout| layout.hidden = true);

    assert!(tree.semantics_nodes().is_empty());
    assert!(tree.semantics_node(group).is_none());
    assert!(tree.semantics_node(leaf).is_none(), "an ancestor is hidden");
}

#[test]
fn the_projection_reports_the_interaction_bits_and_a_press_handler() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(
        None,
        Box::new(Described {
            role: crate::semantics::Role::Button,
            value: "root",
        }),
        Style::default(),
    );
    tree.set_focusable(root, true);
    let handlers = super::interaction::Handlers {
        on_press: Some(Box::new(|| {})),
        ..Default::default()
    };
    tree.set_handlers(root, handlers);

    let node = tree.semantics_node(root).expect("the root projects");
    assert!(node.focusable);
    assert!(node.pressable);
    assert!(!node.disabled);
    assert!(!node.focused);

    tree.set_disabled(root, true);
    let node = tree.semantics_node(root).expect("the root projects");
    assert!(node.disabled);

    tree.set_disabled(root, false);
    tree.set_focus(Some(root));
    let node = tree.semantics_node(root).expect("the root projects");
    assert!(node.focused);
}

#[test]
fn an_element_role_overrides_the_widget_role_but_not_its_value() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(
        None,
        Box::new(Described {
            role: crate::semantics::Role::Button,
            value: "root",
        }),
        Style::default(),
    );
    tree.set_a11y_role(root, Some(crate::semantics::Role::List));
    tree.set_a11y_label(root, Some(String::from("named")));

    let node = tree.semantics_node(root).expect("the root projects");
    assert_eq!(node.role, crate::semantics::Role::List);
    assert_eq!(node.label.as_deref(), Some("named"));
    assert_eq!(node.value.as_deref(), Some("root"));
}

#[test]
fn focus_changes_and_disabling_queue_the_nodes_they_touch() {
    let (mut tree, root, child) = described_pair();
    tree.set_focusable(root, true);
    tree.take_a11y_changes();

    tree.set_focus(Some(root));
    let changes = tree.take_a11y_changes();
    assert_eq!(changes.changed, vec![root]);

    tree.set_focus(Some(child));
    let changes = tree.take_a11y_changes();
    assert_eq!(changes.changed, vec![root, child], "the old and the new");
}
