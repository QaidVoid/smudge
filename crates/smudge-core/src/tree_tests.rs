use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::geometry::{Color, Rect, Size};
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
    let effect = tree.owner(id).expect("the node is live").with(|| {
        RenderEffect::new(move |_| {
            value.get();
            counter.set(counter.get() + 1);
        })
    });
    tree.nodes[id].effects.push(effect);
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
        "the node dropped the effect handle when it was removed"
    );
}

#[test]
fn removing_a_painted_node_damages_its_painted_bounds() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());
    let child = tree.insert(Some(root), Box::new(Empty), Style::default());

    let bounds = Rect::new(4.0, 8.0, 24.0, 18.0);
    let clip = Rect::new(0.0, 0.0, 40.0, 30.0);
    tree.nodes[child].painted = Some(PaintedBounds { bounds, clip });

    tree.remove(child);

    assert_eq!(
        tree.removed_damage,
        vec![PaintedBounds { bounds, clip }],
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

    tree.apply_paint(id, |paint| paint.background = Some(Color::BLACK));

    assert_eq!(tree.nodes[id].dirty, Dirty::Paint);
    assert!(!tree.layout_dirty);
    assert_eq!(tree.paint_queue, vec![id]);
    assert_eq!(
        tree.style(id).expect("the node is live").paint.background,
        Some(Color::BLACK)
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
