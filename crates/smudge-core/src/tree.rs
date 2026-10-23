//! The retained widget tree.
//!
//! The tree stores one node per widget, keyed by a generational
//! [`WidgetId`] that stays valid for the node's lifetime. It keeps its own
//! parent and child links in step with the layout engine's tree, gives every
//! node a reactive owner, and records how much of each node is out of date.
//!
//! The tree is the only owner of widget state. Effects never hold a reference
//! to it: a binding pushes an update onto a queue and the frame drains that
//! queue, so every mutation of the tree happens in one place and in one order.

use std::any::Any;

use slotmap::{SlotMap, new_key_type};

use crate::geometry::{Rect, Size};
use crate::paint::PaintOp;
use crate::reactive::{Owner, RenderEffect};
use crate::style::{LayoutProps, PaintProps, Style};
use crate::widget::Widget;

#[path = "layout.rs"]
mod layout;

new_key_type! {
    /// Identifies one node in the widget tree.
    ///
    /// The id stays valid for as long as the node lives and is never reused for
    /// a different node, so an id whose node was removed refers to nothing.
    /// Every operation that takes one reports that rather than acting on
    /// another node.
    pub struct WidgetId;
}

/// How much of a node is out of date.
///
/// The variants are ordered, and [`Dirty::Layout`] implies [`Dirty::Paint`]. A
/// node's flag only ever rises within a frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dirty {
    /// Nothing about the node changed since the last frame.
    Clean,
    /// The node's paint output must be regenerated. Its layout is current.
    Paint,
    /// The node must be laid out again, which repaints it as well.
    Layout,
}

/// One widget and everything the frame needs to know about it.
struct Node {
    /// The widget this node owns.
    widget: Box<dyn Widget>,
    /// The node this one is a child of, or `None` for the root.
    parent: Option<WidgetId>,
    /// The node's children, in paint order.
    children: Vec<WidgetId>,
    /// The node's complete style, split into layout and paint properties.
    style: Style,
    /// The node's id in the layout engine's tree.
    taffy: taffy::NodeId,
    /// The reactive owner that every binding of this node runs under.
    owner: Owner,
    /// The only strong handles to the node's bindings.
    ///
    /// Dropping a handle stops its binding, and owner cleanup does not, so
    /// removal drops these before it cleans the owner up.
    effects: Vec<RenderEffect<()>>,
    /// How much of the node is out of date.
    dirty: Dirty,
    /// The node's absolute rectangle, set by the layout pass.
    rect: Rect,
    /// The rectangle the node's subtree is clipped to, set by the layout pass.
    clip: Rect,
    /// The node's cached paint output, filled by the paint pass.
    #[expect(dead_code, reason = "the display list reads it")]
    ops: Vec<PaintOp>,
    /// The bounds the node painted last frame, already clipped.
    painted: Option<Rect>,
}

/// The retained tree of widgets.
///
/// A tree has at most one root. Removing the root leaves the tree empty.
pub(crate) struct WidgetTree {
    /// The nodes, keyed by the id the tree handed out.
    nodes: SlotMap<WidgetId, Node>,
    /// The layout engine's tree, holding one node per widget node.
    taffy: taffy::TaffyTree<WidgetId>,
    /// The root node, when the tree has one.
    root: Option<WidgetId>,
    /// Nodes that need painting, in the order they were first marked.
    paint_queue: Vec<WidgetId>,
    /// Whether any node needs a new layout pass.
    layout_dirty: bool,
    /// The painted bounds of nodes removed since the last frame.
    removed_damage: Vec<Rect>,
    /// The viewport the last layout pass ran for, or `None` before the first.
    viewport: Option<Size>,
    /// How many times the layout engine has run for this tree.
    layout_runs: u64,
}

impl WidgetTree {
    /// Creates an empty tree.
    #[cfg_attr(not(test), expect(dead_code, reason = "the app creates it"))]
    pub(crate) fn new() -> WidgetTree {
        WidgetTree {
            nodes: SlotMap::with_key(),
            taffy: taffy::TaffyTree::new(),
            root: None,
            paint_queue: Vec::new(),
            layout_dirty: false,
            removed_damage: Vec::new(),
            viewport: None,
            layout_runs: 0,
        }
    }

    /// Adds a node for `widget` under `parent`, or as the root when `parent` is
    /// `None`, and returns the new node's id.
    ///
    /// The node starts [`Dirty::Layout`], because it has never been laid out.
    ///
    /// # Panics
    ///
    /// Panics when `parent` is not a live id, or when `parent` is `None` and
    /// the tree already has a root. Both are internal invariants: callers pass
    /// ids that this tree handed out.
    pub(crate) fn insert(
        &mut self,
        parent: Option<WidgetId>,
        widget: Box<dyn Widget>,
        style: Style,
    ) -> WidgetId {
        match parent {
            Some(parent) => assert!(
                self.nodes.contains_key(parent),
                "the parent id is a live node"
            ),
            None => assert!(
                self.root.is_none(),
                "a tree has one root, and this one already has a root node"
            ),
        }

        let taffy = self
            .taffy
            .new_leaf(style.layout.to_taffy())
            .expect("the layout engine accepts a new leaf node");
        let owner = match parent {
            Some(parent) => self
                .nodes
                .get(parent)
                .expect("the parent id is a live node")
                .owner
                .child(),
            None => Owner::new(),
        };

        let node = Node {
            widget,
            parent,
            children: Vec::new(),
            style,
            taffy,
            owner,
            effects: Vec::new(),
            dirty: Dirty::Clean,
            rect: Rect::default(),
            clip: Rect::default(),
            ops: Vec::new(),
            painted: None,
        };
        let id = self.nodes.insert(node);
        self.taffy
            .set_node_context(taffy, Some(id))
            .expect("the new layout node is live");

        match parent {
            Some(parent) => {
                let parent_node = self.nodes.get_mut(parent).expect("the parent id is live");
                parent_node.children.push(id);
                let parent_taffy = parent_node.taffy;
                self.taffy
                    .add_child(parent_taffy, taffy)
                    .expect("both layout nodes are live");
            }
            None => self.root = Some(id),
        }

        self.mark(id, Dirty::Layout);
        id
    }

    /// Removes the node `id` and every node below it.
    ///
    /// Each removed node drops its bindings, cleans its reactive owner up and
    /// contributes its last painted bounds to the removed damage. The node is
    /// detached from its parent, which is marked for layout. A stale id does
    /// nothing.
    pub(crate) fn remove(&mut self, id: WidgetId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let parent = node.parent;
        if self.root == Some(id) {
            self.root = None;
        }
        if let Some(parent) = parent
            && let Some(parent_node) = self.nodes.get_mut(parent)
        {
            parent_node.children.retain(|child| *child != id);
        }

        let mut order = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            order.push(current);
            if let Some(node) = self.nodes.get(current) {
                stack.extend(node.children.iter().copied());
            }
        }

        // Deepest node first, so every taffy node is still attached to a live
        // parent when it is removed.
        for current in order.into_iter().rev() {
            let Some(node) = self.nodes.get_mut(current) else {
                continue;
            };
            let effects = std::mem::take(&mut node.effects);
            let owner = node.owner.clone();
            let painted = node.painted.take();
            let taffy = node.taffy;
            // Bindings must stop before the owner disposes the signals they
            // read.
            drop(effects);
            owner.cleanup();
            if let Some(painted) = painted {
                self.removed_damage.push(painted);
            }
            self.taffy
                .remove(taffy)
                .expect("every widget node has a live layout node");
            self.nodes.remove(current);
        }

        if let Some(parent) = parent {
            self.mark(parent, Dirty::Layout);
        }
    }

    /// Reorders the children of `id` to `children` and marks the node for
    /// layout.
    ///
    /// # Panics
    ///
    /// Panics unless `children` is a permutation of the node's current children.
    /// Callers reorder children the node already has.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "keyed child lists reorder children")
    )]
    pub(crate) fn set_children(&mut self, id: WidgetId, children: Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let taffy = node.taffy;

        let mut reordered = children.clone();
        reordered.sort_unstable();
        let mut current = node.children.clone();
        current.sort_unstable();
        assert_eq!(
            reordered, current,
            "set_children reorders the children the node already has"
        );
        for child in &children {
            assert_eq!(
                self.nodes.get(*child).and_then(|node| node.parent),
                Some(id),
                "every new child is a child of the node"
            );
        }

        let taffy_children: Vec<taffy::NodeId> = children
            .iter()
            .map(|child| self.nodes[*child].taffy)
            .collect();
        self.taffy
            .set_children(taffy, &taffy_children)
            .expect("the children are live layout nodes of this parent");
        self.nodes[id].children = children;
        self.mark(id, Dirty::Layout);
    }

    /// Raises the node's dirty flag to `dirty` and queues it for painting the
    /// first time it leaves [`Dirty::Clean`].
    ///
    /// Marking the node for layout also invalidates the size the layout engine
    /// cached for it and its ancestors, because a widget can change what it
    /// measures without any change to its style.
    ///
    /// A stale id does nothing.
    pub(crate) fn mark(&mut self, id: WidgetId, dirty: Dirty) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.dirty >= dirty {
            return;
        }
        let was_clean = node.dirty == Dirty::Clean;
        let taffy = node.taffy;
        node.dirty = dirty;
        if was_clean {
            self.paint_queue.push(id);
        }
        if dirty == Dirty::Layout {
            self.layout_dirty = true;
            self.taffy
                .mark_dirty(taffy)
                .expect("every widget node has a live layout node");
        }
    }

    /// Stores the handle of one of the node's bindings.
    ///
    /// The handle is the only strong reference to the binding's effect, so
    /// dropping it stops the effect. Removal drops every handle before it
    /// cleans the owner up. A stale id drops the handle, which stops a binding
    /// whose node is already gone.
    pub(crate) fn add_effect(&mut self, id: WidgetId, effect: RenderEffect<()>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.effects.push(effect);
        }
    }

    /// Returns the node's style, or `None` when the id is stale.
    #[cfg_attr(not(test), expect(dead_code, reason = "the paint pass reads it"))]
    pub(crate) fn style(&self, id: WidgetId) -> Option<&Style> {
        self.nodes.get(id).map(|node| &node.style)
    }

    /// Applies `apply` to the node's layout properties.
    ///
    /// The layout engine's style is updated and the node marked for layout only
    /// when the properties change, so applying an equal value does nothing. A
    /// stale id does nothing.
    pub(crate) fn apply_layout(&mut self, id: WidgetId, apply: impl FnOnce(&mut LayoutProps)) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let mut layout = node.style.layout.clone();
        apply(&mut layout);
        if layout == node.style.layout {
            return;
        }
        let taffy = node.taffy;
        let taffy_style = layout.to_taffy();
        node.style.layout = layout;
        self.taffy
            .set_style(taffy, taffy_style)
            .expect("every widget node has a live layout node");
        self.mark(id, Dirty::Layout);
    }

    /// Applies `apply` to the node's paint properties.
    ///
    /// The node is marked for paint only when the properties change, so
    /// applying an equal value does nothing. A stale id does nothing.
    pub(crate) fn apply_paint(&mut self, id: WidgetId, apply: impl FnOnce(&mut PaintProps)) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let mut paint = node.style.paint.clone();
        apply(&mut paint);
        if paint == node.style.paint {
            return;
        }
        node.style.paint = paint;
        self.mark(id, Dirty::Paint);
    }

    /// Returns the node's widget as `W`, or `None` when the id is stale or the
    /// widget has another type.
    pub(crate) fn widget_mut<W: Widget>(&mut self, id: WidgetId) -> Option<&mut W> {
        let node = self.nodes.get_mut(id)?;
        (node.widget.as_mut() as &mut dyn Any).downcast_mut::<W>()
    }

    /// Returns the node's reactive owner, or `None` when the id is stale.
    pub(crate) fn owner(&self, id: WidgetId) -> Option<&Owner> {
        self.nodes.get(id).map(|node| &node.owner)
    }

    /// Panics unless the layout engine's tree and the node store describe the
    /// same parents, children, contexts and layout styles.
    #[cfg(test)]
    pub(crate) fn debug_assert_consistent(&self) {
        assert_eq!(
            self.taffy.total_node_count(),
            self.nodes.len(),
            "every widget node has exactly one layout node"
        );
        for (id, node) in &self.nodes {
            let taffy_children = self
                .taffy
                .children(node.taffy)
                .expect("every widget node has a live layout node");
            assert_eq!(
                taffy_children.len(),
                node.children.len(),
                "the child count agrees"
            );
            for (child, taffy_child) in node.children.iter().zip(taffy_children) {
                let child_node = &self.nodes[*child];
                assert_eq!(child_node.taffy, taffy_child, "the child order agrees");
                assert_eq!(
                    child_node.parent,
                    Some(id),
                    "every child points back at its parent"
                );
            }
            assert_eq!(
                self.taffy.parent(node.taffy),
                node.parent.map(|parent| self.nodes[parent].taffy),
                "the parent agrees"
            );
            assert_eq!(
                self.taffy.get_node_context(node.taffy),
                Some(&id),
                "the layout node's context is the widget id"
            );
            assert_eq!(
                self.taffy
                    .style(node.taffy)
                    .expect("every widget node has a live layout node"),
                &node.style.layout.to_taffy(),
                "the layout style agrees"
            );
        }
    }
}

/// The state that tests outside this module observe without reading the store.
#[cfg(test)]
impl WidgetTree {
    /// Returns the node's dirty flag, or `None` when the id is stale.
    pub(crate) fn dirty(&self, id: WidgetId) -> Option<Dirty> {
        self.nodes.get(id).map(|node| node.dirty)
    }

    /// Returns whether any node needs a new layout pass.
    pub(crate) fn layout_is_dirty(&self) -> bool {
        self.layout_dirty
    }

    /// Returns the node's absolute rectangle, or `None` when the id is stale.
    pub(crate) fn rect(&self, id: WidgetId) -> Option<Rect> {
        self.nodes.get(id).map(|node| node.rect)
    }

    /// Returns the rectangle the node is clipped to, or `None` when the id is
    /// stale.
    pub(crate) fn clip(&self, id: WidgetId) -> Option<Rect> {
        self.nodes.get(id).map(|node| node.clip)
    }

    /// Returns how many times the layout engine has run for this tree.
    pub(crate) fn layout_runs(&self) -> u64 {
        self.layout_runs
    }

    /// Returns the node's children in paint order, or `None` when the id is
    /// stale.
    pub(crate) fn children(&self, id: WidgetId) -> Option<&[WidgetId]> {
        self.nodes.get(id).map(|node| node.children.as_slice())
    }

    /// Returns the number of nodes in the tree.
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Clears the dirty state of the whole tree, as a frame does once it has
    /// laid the tree out and painted every queued node.
    pub(crate) fn clear_dirty(&mut self) {
        for node in self.nodes.values_mut() {
            node.dirty = Dirty::Clean;
        }
        self.paint_queue.clear();
        self.layout_dirty = false;
        self.removed_damage.clear();
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
