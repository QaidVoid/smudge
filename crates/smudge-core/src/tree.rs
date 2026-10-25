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
use std::collections::HashSet;
use std::rc::Rc;

use slotmap::{SlotMap, new_key_type};

use crate::geometry::{Point, Rect, Size};
use crate::paint::PaintOp;
use crate::reactive::Owner;
use crate::style::{LayoutProps, PaintProps, Style};
use crate::theme::Theme;
use crate::variants::{Variants, resolve};
use crate::widget::Widget;

#[path = "display_list_pass.rs"]
mod display_list_pass;
#[path = "focus.rs"]
pub(crate) mod focus;
#[path = "interaction.rs"]
pub(crate) mod interaction;
#[path = "layout.rs"]
mod layout;
#[path = "paint_pass.rs"]
mod paint_pass;

use self::interaction::{Handlers, Interaction};

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

/// The bounds a node painted in the last frame, and the clip it painted under.
///
/// The bounds are the union of the node's paint operation bounds before
/// clipping. The clip is kept beside them, because damage converts the two to
/// physical pixels separately: a pixel that a clip edge only partly covers is
/// still drawn, and clipping the bounds in logical space first would miss it.
#[derive(Copy, Clone, Debug, PartialEq)]
struct PaintedBounds {
    /// The union of the node's paint operation bounds, before clipping.
    bounds: Rect,
    /// The rectangle the node's subtree was clipped to.
    clip: Rect,
}

/// One widget and everything the frame needs to know about it.
struct Node {
    /// The widget this node owns.
    widget: Box<dyn Widget>,
    /// The node this one is a child of, or `None` for the root.
    parent: Option<WidgetId>,
    /// The node's children, in paint order.
    children: Vec<WidgetId>,
    /// The node's base style, written by the element's builders and their
    /// bindings.
    base: Style,
    /// The style layout and paint read, with the patches of the active
    /// interaction states applied over the base style.
    resolved: Style,
    /// The style patches per interaction state, when the element declared
    /// any. Without patches the resolved style tracks the base style.
    variants: Option<Rc<Variants>>,
    /// Whether the widget's last paint resolved a value from the theme, so a
    /// theme switch repaints the node.
    uses_tokens: bool,
    /// The node's id in the layout engine's tree.
    taffy: taffy::NodeId,
    /// The reactive owner that every binding of this node runs under.
    ///
    /// Cleaning the owner up stops the node's bindings, so removal cleans it
    /// up and nothing else has to track binding handles.
    owner: Owner,
    /// How much of the node is out of date.
    dirty: Dirty,
    /// The node's absolute rectangle, set by the layout pass.
    rect: Rect,
    /// The rectangle the node's subtree is clipped to, set by the layout pass.
    clip: Rect,
    /// The node's interaction state: hovered, pressed, disabled and focus.
    interaction: Interaction,
    /// The input handlers the node owns, set by its element's builders.
    handlers: Handlers,
    /// The node's cached paint output, filled by the paint pass.
    ops: Vec<PaintOp>,
    /// The bounds the node painted last frame and the clip it painted under.
    painted: Option<PaintedBounds>,
}

/// Returns whether two child lists that hold the same nodes in a different
/// relative order restack any pair of them.
///
/// Children one list gained or lost are skipped: a mount and a removal move
/// nothing that stayed, so their damage comes from the nodes themselves. Two
/// survivors that traded places can overlap, and no rectangle or dirty flag
/// announces the pixels they now share in the other order.
fn children_differ_in_order(before: &[WidgetId], after: &[WidgetId]) -> bool {
    let kept_after: HashSet<WidgetId> = after.iter().copied().collect();
    let kept_before: HashSet<WidgetId> = before.iter().copied().collect();
    let survivors_of_before: Vec<WidgetId> = before
        .iter()
        .filter(|id| kept_after.contains(id))
        .copied()
        .collect();
    let survivors_of_after: Vec<WidgetId> = after
        .iter()
        .filter(|id| kept_before.contains(id))
        .copied()
        .collect();
    survivors_of_before != survivors_of_after
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
    /// The painted bounds the next frame must repaint: the nodes removed
    /// since the last frame, and the subtrees a child reorder restacked.
    pending_damage: Vec<PaintedBounds>,
    /// The viewport the last layout pass ran for, or `None` before the first.
    viewport: Option<Size>,
    /// How many times the layout engine has run for this tree.
    layout_runs: u64,
    /// The deepest node the pointer is over, or `None` when it is over no
    /// node that can respond.
    hovered: Option<WidgetId>,
    /// The pointer position the last pointer event reported, in logical
    /// pixels, so the hovered chain can be recomputed when the tree moves
    /// under a pointer that stays still.
    pointer: Point,
    /// Whether the pointer is inside the window, so a layout pass only
    /// recomputes hover from [`WidgetTree::pointer`] while it is.
    pointer_inside: bool,
    /// The node a primary press captured, while the button is held.
    pressed: Option<WidgetId>,
    /// The node that holds the keyboard focus, when one does.
    focused: Option<WidgetId>,
    /// The node the focus fell off of because it became disabled or hidden.
    ///
    /// The next key event still bubbles from that node, so a shortcut above a
    /// disabled control keeps working, and any focus change or press consumes
    /// the anchor.
    stale_focus: Option<WidgetId>,
    /// Whether the focus last moved by keyboard navigation, so a focus ring
    /// can draw for keyboard users alone. A Tab or an arrow key turns this on
    /// and a pointer press turns it off.
    keyboard_nav: bool,
    /// The theme that paint-time token resolution reads.
    theme: Theme,
}

impl WidgetTree {
    /// Creates an empty tree.
    pub(crate) fn new() -> WidgetTree {
        WidgetTree {
            nodes: SlotMap::with_key(),
            taffy: taffy::TaffyTree::new(),
            root: None,
            paint_queue: Vec::new(),
            layout_dirty: false,
            pending_damage: Vec::new(),
            viewport: None,
            layout_runs: 0,
            hovered: None,
            pointer: Point::default(),
            pointer_inside: false,
            pressed: None,
            focused: None,
            stale_focus: None,
            keyboard_nav: false,
            theme: Theme::light(),
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
            resolved: style.clone(),
            base: style,
            variants: None,
            uses_tokens: false,
            taffy,
            owner,
            dirty: Dirty::Clean,
            rect: Rect::default(),
            clip: Rect::default(),
            interaction: Interaction::default(),
            handlers: Handlers::default(),
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
    /// Each removed node cleans its reactive owner up, which stops its
    /// bindings, and contributes its last painted bounds to the removed
    /// damage. The node is detached from its parent, which is marked for
    /// layout. A stale id does nothing.
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
        if self.focused.is_some_and(|focused| order.contains(&focused)) {
            self.focused = None;
        }
        if self.stale_focus.is_some_and(|stale| order.contains(&stale)) {
            self.stale_focus = None;
        }

        // Deepest node first, so every taffy node is still attached to a live
        // parent when it is removed, and so an owner's bindings stop before
        // the owners above it dispose the signals they read.
        let mut removed_taffy = Vec::new();
        for current in order.into_iter().rev() {
            let Some(node) = self.nodes.get_mut(current) else {
                continue;
            };
            let owner = node.owner.clone();
            let painted = node.painted.take();
            let taffy = node.taffy;
            owner.cleanup();
            if let Some(painted) = painted {
                self.pending_damage.push(painted);
            }
            self.taffy
                .remove(taffy)
                .expect("every widget node has a live layout node");
            self.nodes.remove(current);
            removed_taffy.push(taffy);
        }

        if let Some(parent) = parent {
            self.mark(parent, Dirty::Layout);
        }
        self.drop_stale_hoisted_children(parent, &removed_taffy);
    }

    /// Recreates the layout node of every ancestor, starting at `parent`,
    /// whose hoisted child list still names one of `removed_taffy`.
    ///
    /// The layout engine records an absolutely positioned node in the hoisted
    /// child list of its containing block, and clears that list only when the
    /// block's own layout runs again. A block that lost its last child, or one
    /// that is hidden, runs no layout of its own, so it keeps the stale id and
    /// the next layout pass reads it and panics. A containing block is always
    /// an ancestor, so only the removed node's ancestors need checking.
    fn drop_stale_hoisted_children(
        &mut self,
        parent: Option<WidgetId>,
        removed_taffy: &[taffy::NodeId],
    ) {
        let mut ancestor = parent;
        while let Some(id) = ancestor {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            ancestor = node.parent;
            let stale = self
                .taffy
                .hoisted_children(node.taffy)
                .expect("every widget node has a live layout node")
                .iter()
                .any(|hoisted| removed_taffy.contains(hoisted));
            if stale {
                self.recreate_layout_node(id);
            }
        }
    }

    /// Replaces the layout node of `id` with a fresh one, which carries an
    /// empty hoisted child list that the next layout pass rebuilds.
    ///
    /// The fresh node takes the old node's place among its parent's layout
    /// children, because the engine only lays out nodes it reaches from the
    /// root, and an unattached node would keep the default empty layout.
    fn recreate_layout_node(&mut self, id: WidgetId) {
        let node = self.nodes.get(id).expect("the node is live");
        let style = node.resolved.layout.to_taffy();
        let old = node.taffy;
        let parent = self.taffy.parent(old);
        let index = parent.and_then(|parent| {
            self.taffy
                .children(parent)
                .expect("the parent is a live layout node")
                .iter()
                .position(|child| *child == old)
        });
        let children = self
            .taffy
            .children(old)
            .expect("every widget node has a live layout node");
        self.taffy
            .remove(old)
            .expect("every widget node has a live layout node");
        let taffy = self
            .taffy
            .new_leaf(style)
            .expect("the layout engine accepts a new leaf node");
        self.taffy
            .set_node_context(taffy, Some(id))
            .expect("the new layout node is live");
        for child in children {
            self.taffy
                .add_child(taffy, child)
                .expect("the reattached children are live layout nodes");
        }
        if let Some(parent) = parent {
            match index {
                Some(index) => self
                    .taffy
                    .insert_child_at_index(parent, index, taffy)
                    .expect("the index fits the parent's child list"),
                None => self
                    .taffy
                    .add_child(parent, taffy)
                    .expect("both layout nodes are live"),
            }
        }
        self.nodes.get_mut(id).expect("the node is live").taffy = taffy;
        self.mark(id, Dirty::Layout);
    }

    /// Reorders the children of `id` to `children` and marks the node for
    /// layout.
    ///
    /// A reorder that changes the order also queues the painted bounds of the
    /// whole subtree for a repaint, because the reorder restacks the pixels
    /// the children share without changing any rectangle that would announce
    /// it. The next frame repaints those bounds from the display list in the
    /// new order, which puts the new stacking on the screen even when nothing
    /// below `id` repaints on its own.
    ///
    /// # Panics
    ///
    /// Panics unless `children` is a permutation of the node's current children.
    /// Callers reorder children the node already has.
    pub(crate) fn set_children(&mut self, id: WidgetId, children: Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let taffy = node.taffy;
        let restacked = children_differ_in_order(&node.children, &children);

        let mut reordered_ids = children.clone();
        reordered_ids.sort_unstable();
        let mut current = node.children.clone();
        current.sort_unstable();
        assert_eq!(
            reordered_ids, current,
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
        if restacked {
            self.queue_subtree_damage(id);
        }
        self.mark(id, Dirty::Layout);
    }

    /// Adds the painted bounds of every node below `id`, `id` included, to the
    /// pending damage.
    ///
    /// A child reorder restacks the pixels the reordered subtrees share
    /// without moving or repainting any node, so no node is marked and no
    /// rectangle changes. Repainting these bounds from the display list in the
    /// new child order draws the new stacking.
    fn queue_subtree_damage(&mut self, id: WidgetId) {
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let Some(node) = self.nodes.get(current) else {
                continue;
            };
            if let Some(painted) = node.painted {
                self.pending_damage.push(painted);
            }
            stack.extend_from_slice(&node.children);
        }
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

    /// Returns the node's resolved style, with the active variant patches
    /// applied, or `None` when the id is stale.
    pub(crate) fn style(&self, id: WidgetId) -> Option<&Style> {
        self.nodes.get(id).map(|node| &node.resolved)
    }

    /// Sets the style patches the node resolves its style from, and resolves
    /// the style again.
    ///
    /// Called once, while the node is being mounted, before the interaction
    /// bits are set. A stale id does nothing.
    pub(crate) fn set_variants(&mut self, id: WidgetId, variants: Option<Rc<Variants>>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.variants = variants;
        }
        self.re_resolve(id);
    }

    /// Applies `apply` to the node's base layout properties, then resolves
    /// the style again.
    ///
    /// The resolved style decides what the change invalidated: when its
    /// layout properties differ from the ones the last resolution produced,
    /// the layout engine's style is updated and the node marked for layout;
    /// otherwise nothing happens when the resolved style did not change, or
    /// the node is marked for paint when only its paint properties changed. A
    /// patch can hold a base change back, and then no node is marked at all.
    /// A stale id does nothing.
    pub(crate) fn apply_layout(&mut self, id: WidgetId, apply: impl FnOnce(&mut LayoutProps)) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let mut layout = node.base.layout.clone();
        apply(&mut layout);
        if layout == node.base.layout {
            return;
        }
        node.base.layout = layout;
        self.re_resolve(id);
    }

    /// Applies `apply` to the node's base paint properties, then resolves the
    /// style again, as [`WidgetTree::apply_layout`] does for layout
    /// properties. A stale id does nothing.
    pub(crate) fn apply_paint(&mut self, id: WidgetId, apply: impl FnOnce(&mut PaintProps)) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let mut paint = node.base.paint.clone();
        apply(&mut paint);
        if paint == node.base.paint {
            return;
        }
        node.base.paint = paint;
        self.re_resolve(id);
    }

    /// Recomputes the node's resolved style from its base style, its variant
    /// patches and its interaction state, and marks the node for what the
    /// change invalidated.
    ///
    /// A change to the resolved layout properties updates the layout engine's
    /// style and marks the node for layout; a change to the paint properties
    /// alone marks it for paint. An equal resolved style does nothing, so a
    /// patch that rewrites a value the resolved style already has repaints
    /// nothing. A node that the resolution hid loses the keyboard focus at
    /// once, before the style can draw a focus ring on a hidden node. A stale
    /// id does nothing.
    pub(crate) fn re_resolve(&mut self, id: WidgetId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let empty = Variants::default();
        let variants = node.variants.as_deref().unwrap_or(&empty);
        let resolved = resolve(&node.base, variants, node.interaction);
        if resolved == node.resolved {
            return;
        }
        let became_hidden = resolved.layout.hidden && !node.resolved.layout.hidden;
        let layout_changed = resolved.layout != node.resolved.layout;
        let taffy_style = resolved.layout.to_taffy();
        let node = self.nodes.get_mut(id).expect("the node is live");
        node.resolved = resolved;
        if layout_changed {
            let taffy = self.nodes[id].taffy;
            self.taffy
                .set_style(taffy, taffy_style)
                .expect("every widget node has a live layout node");
            self.mark(id, Dirty::Layout);
        } else {
            self.mark(id, Dirty::Paint);
        }
        if became_hidden {
            self.drop_invalid_focus();
        }
    }

    /// Replaces the theme and marks every node whose last paint resolved a
    /// value from the theme for repainting.
    ///
    /// Nodes whose paint consults no token paint the same pixels under any
    /// theme, so they are left alone. An equal theme marks nothing. Which
    /// nodes consulted the theme is what the paint pass recorded.
    pub(crate) fn set_theme(&mut self, theme: Theme) {
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        let marked: Vec<WidgetId> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.uses_tokens)
            .map(|(id, _)| id)
            .collect();
        for id in marked {
            self.mark(id, Dirty::Paint);
        }
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

    /// Visits every visible node in paint order, passing the node's depth, the
    /// name of its widget's concrete type and its absolute rectangle.
    ///
    /// The root has depth zero. A hidden node and its subtree are skipped,
    /// because they are laid out as empty and paint nothing. The tree itself
    /// stays private: this is how a caller outside the crate reads its
    /// structure.
    pub(crate) fn visit_nodes(&self, visit: &mut dyn FnMut(usize, &'static str, Rect)) {
        if let Some(root) = self.root {
            self.visit_from(root, 0, visit);
        }
    }

    /// Appends `id` and its subtree to the visit started by
    /// [`WidgetTree::visit_nodes`].
    fn visit_from(
        &self,
        id: WidgetId,
        depth: usize,
        visit: &mut dyn FnMut(usize, &'static str, Rect),
    ) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.resolved.layout.hidden {
            return;
        }
        visit(depth, node.widget.type_name(), node.rect);
        for &child in &node.children {
            self.visit_from(child, depth + 1, visit);
        }
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
                &node.resolved.layout.to_taffy(),
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
        self.pending_damage.clear();
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
