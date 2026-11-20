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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use slotmap::{Key, SlotMap, new_key_type};
use web_time::Instant;

use crate::animation::{Presence, Spring, Transition};
use crate::damage::DamageAccumulator;
use crate::geometry::{Point, Rect, Size};
use crate::paint::PaintOp;
use crate::reactive::{Owner, RwSignal};
use crate::semantics::{A11yChanges, Checked, Role, Semantics, SemanticsNode, ValueRange};
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
#[path = "overlay.rs"]
pub(crate) mod overlay;
#[path = "paint_pass.rs"]
mod paint_pass;
#[cfg(feature = "devtools")]
#[path = "tree_inspect.rs"]
mod tree_inspect;

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

impl WidgetId {
    /// Returns the raw representation of the id.
    ///
    /// An accessibility backend identifies its nodes by `u64`, so the
    /// projection maps ids through this conversion. The raw value of two
    /// live ids is always different, and a value this tree never handed out
    /// restores to an id that names no node.
    pub fn as_u64(self) -> u64 {
        self.data().as_ffi()
    }

    /// Restores the id that [`WidgetId::as_u64`] encoded.
    ///
    /// An id whose node was removed already, or one that names no node at
    /// all, is a no-op everywhere the tree takes it.
    pub fn from_u64(raw: u64) -> WidgetId {
        WidgetId(slotmap::KeyData::from_ffi(raw))
    }
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
/// clipping, in the paint coordinates the node painted at, which are its
/// window coordinates translated by `shift`. The clip is kept beside them,
/// because damage converts the two to physical pixels separately: a pixel
/// that a clip edge only partly covers is still drawn, and clipping the
/// bounds in logical space first would miss it. The operations a node draws
/// inside its border keep a union of their own, with the rectangle inside the
/// border they are clipped to, both in the same paint coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
struct PaintedBounds {
    /// The union of the node's paint operation bounds, before clipping.
    bounds: Rect,
    /// The union of the bounds of the operations the node draws inside its
    /// border, before clipping.
    inner: Rect,
    /// The node's rectangle inside its border, which `inner` is clipped to
    /// within `clip`, in paint coordinates like `bounds` and `inner`, while
    /// `clip` is in window coordinates.
    inside: Rect,
    /// The rectangle the node's subtree was clipped to, in window coordinates.
    clip: Rect,
    /// The paint shift the bounds are relative to, so damage can turn them
    /// back into window coordinates.
    shift: Point,
}

impl PaintedBounds {
    /// Adds every pixel these bounds may have drawn to `damage`.
    fn damage(&self, damage: &mut DamageAccumulator) {
        let back = Point {
            x: -self.shift.x,
            y: -self.shift.y,
        };
        damage.add_clipped(self.bounds.translated(back), self.clip);
        damage.add_clipped_inside(
            self.inner.translated(back),
            self.clip,
            self.inside.translated(back),
        );
    }
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
    /// The node's absolute rectangle, set by the walk that computes absolute
    /// rectangles, which the layout pass and a scroll offset change run.
    rect: Rect,
    /// The node's content box: its rectangle inside the border and the
    /// padding, set by the same walk.
    content: Rect,
    /// The translation from the node's window coordinates to its paint
    /// coordinates, which is the sum of the scroll offsets that move it.
    ///
    /// The walk that computes absolute rectangles maintains it: a flowing
    /// child adds the scroll offset of its parent to the parent's shift, and
    /// an absolutely positioned child inherits the parent's shift. The paint
    /// pass stores operations and painted bounds in paint coordinates, which
    /// lets a scroll move content by changing offsets alone: the display list
    /// wraps the moved children in an offset scope instead of repainting
    /// them, and damage translates the bounds back with the shift.
    paint_shift: Point,
    /// The node's rectangle in paint coordinates, `rect` translated by
    /// `paint_shift`.
    ///
    /// The walk computes it from the layout without the scroll offsets, so a
    /// scroll leaves it exactly as it was and the walk can tell a node that
    /// only scrolled from one that has to paint again.
    paint_rect: Rect,
    /// The node's content box in paint coordinates, `content` translated by
    /// `paint_shift`.
    paint_content: Rect,
    /// The rectangle the node's subtree is clipped to, set by the same walk.
    clip: Rect,
    /// The translation applied to the node's flowing children after layout,
    /// outside the layout engine. It is how a scroll view moves its content:
    /// the walk that computes absolute rectangles adds it to the paint shift
    /// of every child that stays in the flow, which moves the child's window
    /// rectangle by minus the offset, and [`WidgetTree::set_scroll_offset`]
    /// re-runs that walk for the subtree alone. An absolutely positioned
    /// child is left where its insets place it.
    scroll_offset: Point,
    /// The signal that pins the node's scroll offset to the end of its
    /// flowing content while it reads `true`, set by the element's builders.
    /// While it does, [`WidgetTree::layout`] pins the offset after each pass;
    /// see the resolution in the layout pass module.
    follow: Option<RwSignal<bool>>,
    /// The flowing child the node keeps in place, by index, and how far
    /// below the top of its content box, set by the element's builders.
    /// [`WidgetTree::layout`] applies it after each pass.
    pin: Option<(usize, f32)>,
    /// The node's interaction state: hovered, pressed, disabled and focus.
    interaction: Interaction,
    /// The cursor the node's element set for the pointer over it, or `None`
    /// when the node leaves the choice to an ancestor or the platform.
    cursor: Option<crate::window::Cursor>,
    /// The input handlers the node owns, set by its element's builders.
    handlers: Handlers,
    /// The role the element's builders set for the accessibility projection,
    /// which overrides the role the widget reports.
    a11y_role: Option<Role>,
    /// The accessible label the element's builders set, which overrides the
    /// label the widget reports.
    a11y_label: Option<String>,
    /// The checked state the element's builders set, which overrides the
    /// state the widget reports.
    a11y_checked: Option<Checked>,
    /// The description, value and picked state the element set, which
    /// override the widget's, boxed since few nodes set any.
    a11y_extra: Option<Box<A11yExtra>>,
    /// The value and range the element's builders set, which override the
    /// range the widget reports. Boxed, since few nodes have one.
    a11y_range: Option<Box<ValueRange>>,
    /// The place in a set the element's builders set, which overrides the
    /// place the widget reports.
    a11y_position: Option<usize>,
    /// The size of the set the element's builders set, which overrides the
    /// size the widget reports.
    a11y_set_size: Option<usize>,
    /// The heading level the element's builders set, which overrides the
    /// level the widget reports.
    a11y_level: Option<usize>,
    /// Where the node's element was created, for the devtools.
    #[cfg(feature = "devtools")]
    created_at: Option<&'static std::panic::Location<'static>>,
    /// The node's cached paint output, filled by the paint pass.
    ops: Vec<PaintOp>,
    /// The node's cached paint output that draws inside its border, clipped
    /// to it, filled by the paint pass from [`Widget::paint_inside`].
    inner_ops: Vec<PaintOp>,
    /// The bounds the node painted last frame and the clip it painted under.
    painted: Option<PaintedBounds>,
    /// The springs that move the node's paint props, when its element asked
    /// for a transition.
    transition: Option<Box<Transition>>,
    /// The opacity the node's subtree was last drawn at, which the paint
    /// pass keeps and the display list reads.
    opacity: f32,
    /// How the node fades in and out, when its element asked for it.
    presence: Option<Box<Presence>>,
    /// What the node needs back if its exit is cancelled, and the owner its
    /// removal cleans up, while it fades out of a parent that removed it.
    exit: Option<Box<Exit>>,
}

impl Node {
    /// Whether the node and its subtree are out of the interface: hidden,
    /// or fading out of a parent that removed it. Such a subtree takes no
    /// input, holds no focus, and is left out of the accessibility
    /// projection.
    fn withdrawn(&self) -> bool {
        self.resolved.layout.hidden || self.exit.is_some()
    }
}

/// The accessibility state an element sets that few nodes have.
#[derive(Default)]
struct A11yExtra {
    /// The description, which overrides the widget's.
    description: Option<String>,
    /// The value, which overrides the widget's.
    value: Option<String>,
    /// Whether the node is the picked choice of its set.
    selected: Option<bool>,
    /// Whether what the node opens is open.
    expanded: Option<bool>,
}

/// A node fading out of a parent that removed it.
struct Exit {
    /// The owner of what the node's build created, cleaned up when the node
    /// is finally removed.
    owner: Owner,
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

/// How many pending accessibility changes the tree queues before the next
/// projection rebuilds from scratch instead.
const A11Y_DIRTY_CAP: usize = 4096;

/// A mounted focus trap, and the focus to restore when it ends.
#[derive(Clone, Copy)]
struct FocusTrap {
    /// The node whose subtree keeps the keyboard.
    node: WidgetId,
    /// The node that held the focus when the trap last became able to hold
    /// it, by mounting visible or by showing again.
    restore: Option<WidgetId>,
    /// Whether the trap could hold the focus when it was last looked at, so
    /// a change between showing and hiding is seen once.
    active: bool,
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
    /// The overlays, in the order they draw, bottom first, as
    /// [`overlay::Layer`] orders them.
    overlays: Vec<overlay::Overlay>,
    /// How many overlays opened so far, which orders them within a layer.
    overlay_count: u64,
    /// Nodes that need painting, in the order they were first marked.
    paint_queue: Vec<WidgetId>,
    /// Whether any node needs a new layout pass.
    layout_dirty: bool,
    /// The painted bounds the next frame must repaint: the nodes removed
    /// since the last frame, and the subtrees a child reorder restacked.
    pending_damage: Vec<PaintedBounds>,
    /// The viewport the last layout pass ran for, or `None` before the first.
    viewport: Option<Size>,
    /// The scale the next layout pass measures at. A widget that sizes itself
    /// from pixel data measures against it.
    scale: f32,
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
    /// The nodes whose button handlers took a press, each with the button,
    /// while the button is held.
    button_holders: Vec<(crate::input::PointerButton, WidgetId)>,
    /// The keyboard modifiers the last key or modifiers event reported,
    /// which the button handlers hear.
    pub(crate) modifiers: crate::input::Modifiers,
    /// The nodes that asked for the pointer lock, in mount order, each with
    /// the signal that says whether it wants it now. Removed nodes are
    /// dropped lazily.
    pointer_locks: Vec<(WidgetId, RwSignal<bool>)>,
    /// The drag the held press makes, while it is held.
    drag: Option<interaction::DragState>,
    /// Whether a press was released without its release, so a primary
    /// release that arrives with no press held belongs to that press and
    /// reaches no handler. The next primary press clears it.
    released_early: bool,
    /// The node that holds the keyboard focus, when one does.
    focused: Option<WidgetId>,
    /// The node the focus fell off of because it became disabled or hidden.
    ///
    /// The next key event still bubbles from that node, so a shortcut above a
    /// disabled control keeps working. Any focus change consumes the anchor,
    /// and so does any press that is not outside an active focus trap.
    stale_focus: Option<WidgetId>,
    /// Whether the focus last moved by keyboard navigation, so a focus ring
    /// can draw for keyboard users alone. A Tab or an arrow key turns this on
    /// and a pointer press turns it off.
    keyboard_nav: bool,
    /// The autofocus requests that mounted nodes queued and no layout pass
    /// has resolved yet.
    pending_autofocus: Vec<WidgetId>,
    /// The nodes waiting to give the focus to the first focusable node
    /// inside them.
    pending_focus_first: Vec<WidgetId>,
    /// Every live node that asked to give the focus to the first focusable
    /// node inside it, so an overlay that shows again asks again.
    focus_first: Vec<WidgetId>,
    /// Every live node that asked for the focus when it mounted, so an
    /// overlay that shows again asks again.
    autofocused: Vec<WidgetId>,
    /// Whether a focus move asked the next frame to scroll the focused node
    /// into view. The layout of that frame resolves it and clears it.
    reveal_focus: bool,
    /// A rectangle a node asked to show, relative to the node's rectangle,
    /// until the frame's passes have applied it.
    reveal_request: Option<(WidgetId, Rect)>,
    /// The mounted focus traps, oldest first.
    ///
    /// The last mounted trap wins, and removing a node ends every trap it
    /// holds, which restores the one mounted before it and the focus held
    /// when the oldest removed trap mounted.
    focus_traps: Vec<FocusTrap>,
    /// The nodes that carry a follow signal, so the follow resolution after
    /// a layout pass visits them alone instead of every node. A removed node
    /// leaves a stale id here until the next resolution drops it.
    following: Vec<WidgetId>,
    /// The nodes that pin a child, so the pin resolution after a layout
    /// pass visits them alone. A removed node leaves a stale id here until
    /// the next resolution drops it.
    pinned: Vec<WidgetId>,
    /// The theme that paint-time token resolution reads.
    theme: Theme,
    /// The nodes whose projected accessibility state may have changed, in the
    /// order the changes arrived.
    ///
    /// Duplicates are allowed and removed when the changes are taken. When
    /// the list outgrows [`A11Y_DIRTY_CAP`], it is cleared and
    /// [`WidgetTree::a11y_reset`] is raised, which makes the next projection
    /// rebuild from scratch instead.
    a11y_dirty: Vec<WidgetId>,
    /// Whether the next projection must be rebuilt from scratch.
    a11y_reset: bool,
    /// The top nodes a screen reader reached when the accessibility changes
    /// were last taken, as [`WidgetTree::a11y_tops`] gives them, so an
    /// overlay that opened, closed, or showed or hid with its host counts as
    /// a change of the roots.
    a11y_tops: Vec<WidgetId>,
    /// The announcements waiting for the next projection, oldest first.
    announcements: Vec<crate::semantics::Announcement>,
    /// The time the paint pass moves springs to, the frame's time on the
    /// app's clock.
    now: Instant,
    /// Whether springs jump to their targets instead of moving.
    reduced_motion: bool,
    /// Whether a node has ever mounted with a transition.
    transitions_mounted: bool,
    /// The nodes whose springs were still moving at their last paint, which
    /// the next frame paints again.
    animating: Vec<WidgetId>,
    /// The list the next frame's animating nodes are taken into, kept so
    /// its capacity is reused.
    animating_taken: Vec<WidgetId>,
    /// The nodes the paint pass found drawn at a new opacity, kept so its
    /// capacity is reused.
    opacity_changed: Vec<WidgetId>,
    /// The stack a walk of a subtree uses, kept so its capacity is reused.
    walk: Vec<WidgetId>,
    /// The nodes whose exit finished at their last paint, which the next
    /// frame removes.
    exited: Vec<WidgetId>,
}

impl WidgetTree {
    /// Creates an empty tree.
    pub(crate) fn new() -> WidgetTree {
        WidgetTree {
            nodes: SlotMap::with_key(),
            taffy: taffy::TaffyTree::new(),
            root: None,
            overlays: Vec::new(),
            overlay_count: 0,
            paint_queue: Vec::new(),
            layout_dirty: false,
            pending_damage: Vec::new(),
            viewport: None,
            scale: 1.0,
            layout_runs: 0,
            hovered: None,
            pointer: Point::default(),
            pointer_inside: false,
            pressed: None,
            button_holders: Vec::new(),
            modifiers: crate::input::Modifiers::default(),
            pointer_locks: Vec::new(),
            drag: None,
            released_early: false,
            focused: None,
            stale_focus: None,
            keyboard_nav: false,
            pending_autofocus: Vec::new(),
            pending_focus_first: Vec::new(),
            focus_first: Vec::new(),
            autofocused: Vec::new(),
            reveal_focus: false,
            reveal_request: None,
            focus_traps: Vec::new(),
            following: Vec::new(),
            pinned: Vec::new(),
            theme: Theme::light(),
            a11y_dirty: Vec::new(),
            a11y_reset: false,
            a11y_tops: Vec::new(),
            announcements: Vec::new(),
            now: Instant::now(),
            reduced_motion: false,
            transitions_mounted: false,
            animating: Vec::new(),
            animating_taken: Vec::new(),
            opacity_changed: Vec::new(),
            walk: Vec::new(),
            exited: Vec::new(),
        }
    }

    /// Adds a node as [`WidgetTree::insert_owned`] does, with its owner under
    /// the parent node's owner. Only tests build trees this way.
    #[cfg(test)]
    pub(crate) fn insert(
        &mut self,
        parent: Option<WidgetId>,
        widget: Box<dyn Widget>,
        style: Style,
    ) -> WidgetId {
        self.insert_owned(parent, None, widget, style)
    }

    /// Adds a node for `widget` under `parent`, or as the root when `parent` is
    /// `None`, and returns the new node's id.
    ///
    /// The node's owner is created under `owner` when one is given, and under
    /// the parent node's owner otherwise. A slot mounts the child its build
    /// returned with the build's owner, so the child's subtree hangs under
    /// it, and what that build provided as context reaches everything the
    /// subtree builds later.
    ///
    /// The node starts [`Dirty::Layout`], because it has never been laid out.
    ///
    /// # Panics
    ///
    /// Panics when `parent` is not a live id, or when `parent` is `None` and
    /// the tree already has a root. Both are internal invariants: callers pass
    /// ids that this tree handed out.
    pub(crate) fn insert_owned(
        &mut self,
        parent: Option<WidgetId>,
        owner: Option<&Owner>,
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

        let id = self.new_node(parent, owner, widget, style);
        let taffy = self.nodes[id].taffy;
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
        if let Some(parent) = parent {
            self.push_a11y(parent);
        }
        id
    }

    /// Creates the node of `widget` with `style` and its layout node, with
    /// `parent` as its parent and its owner under `owner`, or under the
    /// parent's owner without one, and links it to neither parent.
    fn new_node(
        &mut self,
        parent: Option<WidgetId>,
        owner: Option<&Owner>,
        widget: Box<dyn Widget>,
        style: Style,
    ) -> WidgetId {
        let taffy = self
            .taffy
            .new_leaf(style.layout.to_taffy())
            .expect("the layout engine accepts a new leaf node");
        let owner = match (owner, parent) {
            (Some(owner), _) => owner.child(),
            (None, Some(parent)) => self
                .nodes
                .get(parent)
                .expect("the parent id is a live node")
                .owner
                .child(),
            (None, None) => Owner::new(),
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
            content: Rect::default(),
            paint_shift: Point::default(),
            paint_rect: Rect::default(),
            paint_content: Rect::default(),
            clip: Rect::default(),
            scroll_offset: Point::default(),
            follow: None,
            pin: None,
            interaction: Interaction::default(),
            cursor: None,
            handlers: Handlers::default(),
            a11y_role: None,
            a11y_label: None,
            a11y_checked: None,
            a11y_extra: None,
            a11y_range: None,
            a11y_position: None,
            a11y_set_size: None,
            a11y_level: None,
            #[cfg(feature = "devtools")]
            created_at: None,
            ops: Vec::new(),
            inner_ops: Vec::new(),
            painted: None,
            transition: None,
            opacity: 1.0,
            presence: None,
            exit: None,
        };
        let id = self.nodes.insert(node);
        self.taffy
            .set_node_context(taffy, Some(id))
            .expect("the new layout node is live");
        id
    }

    /// Inserts the top node of an overlay that `host` opened as `opening`
    /// says, and returns its id.
    ///
    /// The node has `host` as its parent, so keys bubble to the host and
    /// its owner hangs under `owner`, or the host's, but the host does not
    /// list it among its children: it is laid out, painted and hit tested
    /// on its own, over the main tree.
    ///
    /// # Panics
    ///
    /// Panics when `host` is not a live node.
    pub(crate) fn insert_overlay(
        &mut self,
        host: WidgetId,
        owner: Option<&Owner>,
        widget: Box<dyn Widget>,
        style: Style,
        opening: overlay::Opening,
    ) -> WidgetId {
        assert!(self.nodes.contains_key(host), "the host id is a live node");
        let id = self.new_node(Some(host), owner, widget, style);
        let order = self.overlay_order(host, opening.layer);
        let at = self
            .overlays
            .partition_point(|overlay| overlay.order <= order);
        self.overlays.insert(
            at,
            overlay::Overlay {
                root: id,
                placement: opening.placement,
                open: opening.open,
                origin: Point::default(),
                passes_pointer: opening.passes_pointer,
                layer: opening.layer,
                order,
            },
        );
        self.mark(id, Dirty::Layout);
        self.push_a11y(host);
        id
    }

    /// Removes the node `id` and every node below it.
    ///
    /// Each removed node cleans its reactive owner up, which stops its
    /// bindings, and contributes its last painted bounds to the removed
    /// damage. The node is detached from its parent, which is marked for
    /// layout. A removed node loses the focus and the press capture. A stale
    /// id does nothing.
    pub(crate) fn remove(&mut self, id: WidgetId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let parent = node.parent;
        // An overlay's top node is no child of its host, so the host's
        // layout stays as it is, while what lay under the overlay may now
        // be under the pointer.
        let overlay = self.is_overlay_root(id);
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
        loop {
            while let Some(current) = stack.pop() {
                order.push(current);
                if let Some(node) = self.nodes.get(current) {
                    stack.extend(node.children.iter().copied());
                }
            }
            // The overlays the removed nodes opened go with them, after
            // their hosts, so they are removed first.
            stack.extend(
                self.overlays
                    .iter()
                    .map(|overlay| overlay.root)
                    .filter(|root| {
                        !order.contains(root)
                            && self
                                .nodes
                                .get(*root)
                                .and_then(|node| node.parent)
                                .is_some_and(|host| order.contains(&host))
                    }),
            );
            if stack.is_empty() {
                break;
            }
        }
        // Where the focus goes back to is read while the removed overlays
        // still name their hosts. A trap enabled again since the last
        // layout pass counts as active, since only that pass records it.
        let restore = self
            .focus_traps
            .iter()
            .find(|trap| {
                order.contains(&trap.node) && (trap.active || self.can_hold_focus(trap.node))
            })
            .and_then(|trap| {
                let opener = self
                    .trap_return(trap.node, trap.restore)
                    .filter(|back| !order.contains(back));
                opener.or(trap.restore)
            })
            .filter(|restore| !order.contains(restore));
        self.overlays
            .retain(|overlay| !order.contains(&overlay.root));
        self.end_holds_of(&order);
        if let Some(focused) = self.focused.filter(|focused| order.contains(focused)) {
            self.focused = None;
            self.run_focus_within(Some(focused), None, &order);
        }
        if self.stale_focus.is_some_and(|stale| order.contains(&stale)) {
            self.stale_focus = None;
        }
        // A drag whose node is gone has nothing left to capture, and hover
        // only follows the pointer again once no press is held.
        if self.pressed.is_some_and(|pressed| order.contains(&pressed)) {
            self.pressed = None;
        }
        // A removed node keeps neither its autofocus request nor its trap:
        // the request is dropped, and the trap mounted before it, if any,
        // takes over. The focus goes back to where it was when the oldest
        // removed active trap became active, since every later one became
        // active inside it. A hidden trap already gave its focus back,
        // unless a dialog that still shows held it back. Then nothing gets
        // it back, since that would put the focus behind the dialog.
        self.pending_autofocus.retain(|id| !order.contains(id));
        self.pending_focus_first.retain(|id| !order.contains(id));
        self.focus_first.retain(|id| !order.contains(id));
        self.autofocused.retain(|id| !order.contains(id));
        self.focus_traps.retain(|trap| !order.contains(&trap.node));

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
            let exit = node.exit.take();
            owner.cleanup();
            if let Some(exit) = exit {
                exit.owner.cleanup();
            }
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
            if !overlay {
                self.mark(parent, Dirty::Layout);
            }
            self.push_a11y(parent);
        }
        self.drop_stale_hoisted_children(parent, &removed_taffy);
        if overlay && self.pointer_inside && self.pressed.is_none() {
            self.refresh_hover();
        }
        if self.focused.is_none()
            && let Some(restore) = restore.filter(|&restore| !self.behind_trap(restore))
        {
            self.request_focus(restore);
        }
    }

    /// Removes the root and every node below it, leaving the tree empty for a
    /// new root.
    ///
    /// A held press ends first, as [`WidgetTree::release_press`] ends it, so
    /// the pressed node hears `on_pointer_up` while it still exists and the
    /// release that follows reaches no handler. The removal drops the focus,
    /// the focus traps and the autofocus requests, as removing any subtree
    /// does, and the hover goes too, so the next layout pass hovers the new
    /// node under the pointer. The accessibility projection is marked for a
    /// rebuild from scratch, because every node it showed is gone. The
    /// theme, the pointer position and the keyboard navigation mode stay.
    pub(crate) fn unmount_root(&mut self) {
        self.release_press();
        if let Some(root) = self.root {
            self.remove(root);
        }
        self.hovered = None;
        self.a11y_dirty.clear();
        self.a11y_reset = true;
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
        self.push_a11y(id);
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

    /// Moves the paint props of the node `id` under `spring` whenever their
    /// resolved values change. A stale id does nothing.
    pub(crate) fn set_transition(&mut self, id: WidgetId, spring: Spring) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.transition = Some(Box::new(Transition::new(spring)));
            self.transitions_mounted = true;
        }
    }

    /// Whether a node has ever mounted with a transition.
    pub(crate) fn transitions_mounted(&self) -> bool {
        self.transitions_mounted
    }

    /// Sets the time the next paint pass moves springs to.
    pub(crate) fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    /// Sets whether springs jump to their targets instead of moving.
    pub(crate) fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Whether a spring was still moving at the last paint, so the next
    /// frame has motion to paint.
    pub(crate) fn is_animating(&self) -> bool {
        !self.animating.is_empty() || !self.exited.is_empty()
    }

    /// Gives the node `id` a presence that fades it in under `enter` when it
    /// mounts and out under `exit` when a parent removes it, either of which
    /// may be absent. A stale id does nothing.
    pub(crate) fn set_presence(
        &mut self,
        id: WidgetId,
        enter: Option<Spring>,
        exit: Option<Spring>,
    ) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.presence = Some(Box::new(Presence::new(enter, exit)));
            self.transitions_mounted = true;
        }
    }

    /// Whether `id` names a live node.
    pub(crate) fn contains(&self, id: WidgetId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Starts the node `id` fading out instead of removing it, when its
    /// element asked for an exit, and keeps `owner` to clean up when the
    /// exit finishes. Hands `owner` back when the node has no exit, so the
    /// caller removes it at once.
    ///
    /// The node keeps its place in the layout until its fade ends, so
    /// nothing moves into it while it fades. A held press inside it ends
    /// first, as [`WidgetTree::release_press`] ends it. Its subtree then
    /// counts as withdrawn, as a hidden one does: it takes no input, the
    /// hover and the focus leave it, a trap inside it hands the focus back,
    /// and the accessibility projection drops it.
    pub(crate) fn begin_exit(&mut self, id: WidgetId, owner: Owner) -> Result<(), Owner> {
        let Some(node) = self.nodes.get(id) else {
            return Err(owner);
        };
        let exits = node
            .presence
            .as_ref()
            .is_some_and(|presence| presence.exits());
        if !exits || node.exit.is_some() {
            return Err(owner);
        }
        // The press ends while the node still takes input, so the pressed
        // node hears its release and lets go of a grab it holds.
        if self
            .pressed
            .is_some_and(|pressed| self.is_within(pressed, id))
        {
            self.release_press();
        }
        let node = self.nodes.get_mut(id).expect("the node is live");
        if let Some(presence) = node.presence.as_deref_mut() {
            presence.set_exiting(true);
        }
        node.exit = Some(Box::new(Exit { owner }));
        let parent = node.parent;
        // Marked for layout, though nothing moves, so the next frame paints
        // the fade and its layout pass resolves the focus traps.
        self.mark(id, Dirty::Layout);
        if let Some(parent) = parent {
            self.push_a11y(parent);
        }
        self.drop_invalid_focus();
        if self.pointer_inside && self.pressed.is_none() {
            self.refresh_hover();
        }
        Ok(())
    }

    /// Stops the exit of the node `id`, which fades in again from where it
    /// is, and returns the owner the exit kept, or `None` when the node is
    /// not exiting.
    pub(crate) fn cancel_exit(&mut self, id: WidgetId) -> Option<Owner> {
        let node = self.nodes.get_mut(id)?;
        let exit = node.exit.take()?;
        if let Some(presence) = node.presence.as_deref_mut() {
            presence.set_exiting(false);
        }
        let parent = node.parent;
        self.exited.retain(|exited| *exited != id);
        // Marked for layout, though nothing moves, so the next frame paints
        // the fade and its layout pass resolves the focus traps.
        self.mark(id, Dirty::Layout);
        self.push_a11y_visible_subtree(id);
        if let Some(parent) = parent {
            self.push_a11y(parent);
        }
        Some(exit.owner)
    }

    /// Removes every node whose exit finished at its last paint, and cleans
    /// up the owner each exit kept.
    pub(crate) fn finish_exits(&mut self) {
        let mut exited = std::mem::take(&mut self.exited);
        for id in exited.drain(..) {
            let Some(exit) = self.nodes.get_mut(id).and_then(|node| node.exit.take()) else {
                continue;
            };
            self.remove(id);
            exit.owner.cleanup();
        }
        self.exited = exited;
    }

    /// Queues for paint every node whose springs were still moving at the
    /// last paint.
    ///
    /// The nodes are queued for paint alone, without the accessibility
    /// update a repaint of a projected node queues, since motion changes no
    /// semantics, and without their ancestors.
    pub(crate) fn queue_animations(&mut self) {
        std::mem::swap(&mut self.animating, &mut self.animating_taken);
        for id in self.animating_taken.drain(..) {
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
            if node.dirty == Dirty::Clean {
                node.dirty = Dirty::Paint;
                self.paint_queue.push(id);
            }
        }
    }

    /// Raises the node's dirty flag to `dirty` and queues it for painting the
    /// first time it leaves [`Dirty::Clean`].
    ///
    /// The node is queued for the accessibility projection too when the
    /// projection could show the change: on a layout change, and on a
    /// repaint of a node the projection shows.
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
        // A repaint changes what the projection shows only through the
        // node's own semantics, so a node the projection leaves out, such as
        // a blinking caret, queues nothing for it. A layout change can move
        // the bounds of the projected nodes around it.
        let projected = dirty == Dirty::Layout || Self::effective_role(node).is_some();
        if was_clean {
            self.paint_queue.push(id);
        }
        if projected && (was_clean || dirty == Dirty::Layout) {
            self.push_a11y(id);
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
    /// patches and its interaction state, with the window's size for the top
    /// node of an overlay that covers the window, and marks the node for
    /// what the change invalidated.
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
        let mut resolved = resolve(&node.base, variants, node.interaction);
        // An overlay that covers the window takes the window's size, over
        // whatever size its base asks for, which comes back once it stops
        // covering.
        if let Some(viewport) = self.viewport
            && self
                .overlays
                .iter()
                .any(|overlay| overlay.root == id && overlay.placement == overlay::Placement::Cover)
        {
            resolved.layout.width = crate::style::Length::px(viewport.width);
            resolved.layout.height = crate::style::Length::px(viewport.height);
        }
        if resolved == node.resolved {
            return;
        }
        let became_hidden = resolved.layout.hidden && !node.resolved.layout.hidden;
        let became_visible = !resolved.layout.hidden && node.resolved.layout.hidden;
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
        if (became_hidden || became_visible)
            && let Some(parent) = self.nodes.get(id).and_then(|node| node.parent)
        {
            self.push_a11y(parent);
        }
        if became_visible {
            self.push_a11y_visible_subtree(id);
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
        for overlay in &self.overlays {
            if self.overlay_shown(overlay) {
                self.visit_from(overlay.root, 0, visit);
            }
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

    /// Returns whether any projected accessibility state may have changed
    /// since the last [`WidgetTree::take_a11y_changes`].
    pub(crate) fn has_a11y_changes(&self) -> bool {
        self.a11y_reset || !self.a11y_dirty.is_empty() || !self.announcements.is_empty()
    }

    /// Takes the projected nodes that changed since the last call.
    ///
    /// A change to an unprojected node changes the child list of its nearest
    /// projected ancestor, so it is reported as that ancestor, or as
    /// [`A11yChanges::roots_changed`] when no ancestor projects. Stale ids,
    /// ids that became hidden, and duplicates drop out. When the change queue
    /// overflowed since the last call, `reset` is set and the changed list is
    /// empty.
    pub(crate) fn take_a11y_changes(&mut self) -> A11yChanges {
        let mut changes = A11yChanges {
            reset: std::mem::take(&mut self.a11y_reset),
            announcements: std::mem::take(&mut self.announcements),
            ..A11yChanges::default()
        };
        let dirty = std::mem::take(&mut self.a11y_dirty);
        if !self.a11y_tops().eq(self.a11y_tops.iter().copied()) {
            // A dialog opening or closing moves where the reader's reach
            // starts. What it hid comes back with nodes the projection
            // forgot, and what it now hides drops out, so the projection
            // builds everything anew. Overlays opening or closing above the
            // same start only change the roots.
            if self.a11y_tops().next() != self.a11y_tops.first().copied() {
                changes.reset = true;
            }
            let tops: Vec<WidgetId> = self.a11y_tops().collect();
            self.a11y_tops = tops;
            changes.roots_changed = true;
        }
        if changes.reset {
            return changes;
        }
        let mut seen = HashSet::new();
        for id in dirty {
            if !seen.insert(id)
                || !self.a11y_id_is_visible(id)
                || !self.a11y_tops.contains(&self.a11y_top_of(id))
            {
                continue;
            }
            let projected = if self.nodes.get(id).and_then(Self::effective_role).is_some() {
                Some(id)
            } else {
                self.nearest_projected_ancestor(id)
            };
            match projected {
                Some(projected) if projected == id || seen.insert(projected) => {
                    changes.changed.push(projected);
                }
                Some(_) => {}
                None => changes.roots_changed = true,
            }
        }
        changes
    }

    /// Returns whether `id` is live and neither it nor an ancestor is hidden.
    fn a11y_id_is_visible(&self, id: WidgetId) -> bool {
        let mut next = Some(id);
        while let Some(current) = next {
            let Some(node) = self.nodes.get(current) else {
                return false;
            };
            if node.withdrawn() {
                return false;
            }
            next = node.parent;
        }
        true
    }

    /// Records that the projected state of `id` may have changed.
    ///
    /// Changes queue freely and are deduplicated when taken. A queue that
    /// outgrows [`A11Y_DIRTY_CAP`] is dropped and the next projection
    /// rebuilds from scratch, which is cheaper than replaying thousands of
    /// one-node updates.
    pub(crate) fn push_a11y(&mut self, id: WidgetId) {
        if self.a11y_dirty.len() >= A11Y_DIRTY_CAP {
            self.a11y_dirty.clear();
            self.a11y_reset = true;
            return;
        }
        self.a11y_dirty.push(id);
    }

    /// Queues `messages`, which the announcer attached to `id` took, for the
    /// next projection, or drops them when `id` is gone, hidden or fading
    /// out.
    pub(crate) fn announce(&mut self, id: WidgetId, messages: Vec<crate::semantics::Announcement>) {
        if !self.a11y_id_is_visible(id) {
            return;
        }
        for message in messages {
            crate::semantics::push_capped(&mut self.announcements, message);
        }
    }

    /// Records `id` and its visible subtree as changed.
    ///
    /// A node that becomes visible again reattaches in the projection, so
    /// every descendant that is visible again has to be rebuilt too.
    fn push_a11y_visible_subtree(&mut self, id: WidgetId) {
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let Some(node) = self.nodes.get(current) else {
                continue;
            };
            if node.withdrawn() {
                continue;
            }
            let children = node.children.clone();
            self.push_a11y(current);
            stack.extend_from_slice(&children);
        }
    }

    /// Sets the role the projection reports for `id`.
    ///
    /// Called once, while the node is being mounted. A stale id does nothing.
    pub(crate) fn set_a11y_role(&mut self, id: WidgetId, role: Option<Role>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.a11y_role = role;
        }
    }

    /// Sets the accessible label of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_label`](crate::element::Element::accessible_label)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_label(&mut self, id: WidgetId, label: Option<String>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.a11y_label == label {
            return;
        }
        node.a11y_label = label;
        self.push_a11y(id);
    }

    /// Sets the accessible checked state of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_checked`](crate::element::Element::accessible_checked)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_checked(&mut self, id: WidgetId, checked: Option<Checked>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.a11y_checked == checked {
            return;
        }
        node.a11y_checked = checked;
        self.push_a11y(id);
    }

    /// Sets the accessible description of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_description`](crate::element::Element::accessible_description)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_description(&mut self, id: WidgetId, description: Option<String>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let extra = node.a11y_extra.get_or_insert_default();
        if extra.description == description {
            return;
        }
        extra.description = description;
        self.push_a11y(id);
    }

    /// Sets the accessible value of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_value`](crate::element::Element::accessible_value)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_value(&mut self, id: WidgetId, value: Option<String>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let extra = node.a11y_extra.get_or_insert_default();
        if extra.value == value {
            return;
        }
        extra.value = value;
        self.push_a11y(id);
    }

    /// Sets whether `id` is the picked choice of its set and records the
    /// change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_selected`](crate::element::Element::accessible_selected)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_selected(&mut self, id: WidgetId, selected: Option<bool>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let extra = node.a11y_extra.get_or_insert_default();
        if extra.selected == selected {
            return;
        }
        extra.selected = selected;
        self.push_a11y(id);
    }

    /// Sets whether what `id` opens is open and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_expanded`](crate::element::Element::accessible_expanded)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_expanded(&mut self, id: WidgetId, expanded: Option<bool>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let extra = node.a11y_extra.get_or_insert_default();
        if extra.expanded == expanded {
            return;
        }
        extra.expanded = expanded;
        self.push_a11y(id);
    }

    /// Sets the accessible value and range of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_range`](crate::element::Element::accessible_range)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_range(&mut self, id: WidgetId, range: Option<ValueRange>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.a11y_range.as_deref() == range.as_ref() {
            return;
        }
        node.a11y_range = range.map(Box::new);
        self.push_a11y(id);
    }

    /// Sets the place in its set of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_position`](crate::element::Element::accessible_position)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_position(&mut self, id: WidgetId, position: Option<usize>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.a11y_position == position {
            return;
        }
        node.a11y_position = position;
        self.push_a11y(id);
    }

    /// Sets the heading level of `id` and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_level`](crate::element::Element::accessible_level)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_level(&mut self, id: WidgetId, level: Option<usize>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.a11y_level == level {
            return;
        }
        node.a11y_level = level;
        self.push_a11y(id);
    }

    /// Sets the size of the set `id` holds and records the change.
    ///
    /// Called when the node mounts and again by the binding of an
    /// [`accessible_set_size`](crate::element::Element::accessible_set_size)
    /// builder value. A stale id does nothing.
    pub(crate) fn set_a11y_set_size(&mut self, id: WidgetId, size: Option<usize>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.a11y_set_size == size {
            return;
        }
        node.a11y_set_size = size;
        self.push_a11y(id);
    }

    /// Projects every visible node in paint order, for an accessibility
    /// backend.
    ///
    /// A node is projected when its element set a role or its widget reports
    /// one; the children of an unprojected node attach to the nearest
    /// projected ancestor. A hidden node and its subtree are skipped, like
    /// in [`WidgetTree::visit_nodes`]. The tree itself stays private: this is
    /// how a caller outside the crate reads the accessibility tree.
    pub(crate) fn semantics_nodes(&self) -> Vec<SemanticsNode> {
        let mut nodes = Vec::new();
        let mut index_of = HashMap::new();
        for top in self.a11y_tops() {
            self.project_from(top, None, &mut nodes, &mut index_of);
        }
        nodes
    }

    /// Returns the top nodes a screen reader reaches, in paint order: the
    /// main tree's root and the top nodes of the overlays that show and are
    /// not fading out. While a dialog shows, they start at the topmost one,
    /// so a reader stays inside the dialog and never meets the tree behind
    /// it.
    pub(crate) fn a11y_tops(&self) -> impl Iterator<Item = WidgetId> + '_ {
        let shown = |overlay: &&overlay::Overlay| {
            self.overlay_shown(overlay)
                && self
                    .nodes
                    .get(overlay.root)
                    .is_some_and(|node| !node.withdrawn())
        };
        let dialog = self
            .overlays
            .iter()
            .rposition(|overlay| overlay.layer == overlay::Layer::Dialog && shown(&overlay));
        let root = if dialog.is_some() { None } else { self.root };
        root.into_iter().chain(
            self.overlays[dialog.unwrap_or(0)..]
                .iter()
                .filter(shown)
                .map(|overlay| overlay.root),
        )
    }

    /// Returns whether a screen reader and the keyboard reach `id`: whether
    /// the top node it hangs from is one of [`WidgetTree::a11y_tops`], so no
    /// dialog drawn above it hides it.
    pub(crate) fn within_reach(&self, id: WidgetId) -> bool {
        let top = self.a11y_top_of(id);
        self.a11y_tops().any(|reached| reached == top)
    }

    /// Returns the top node `id` hangs from: the main tree's root, or the
    /// top node of the overlay it sits in.
    fn a11y_top_of(&self, id: WidgetId) -> WidgetId {
        let mut top = id;
        while let Some(parent) = self.pointer_parent(top) {
            top = parent;
        }
        top
    }

    /// Returns the projected nodes that no projected ancestor holds, in paint
    /// order.
    pub(crate) fn semantics_roots(&self) -> Vec<WidgetId> {
        let mut roots = Vec::new();
        for top in self.a11y_tops() {
            let Some(node) = self.nodes.get(top) else {
                continue;
            };
            if node.withdrawn() {
                continue;
            }
            if Self::effective_role(node).is_some() {
                roots.push(top);
            } else {
                self.collect_projected_children(top, &mut roots);
            }
        }
        roots
    }

    /// Returns whether `id` projects: it is live, nothing above or at it is
    /// hidden, no dialog hides it, and it reports a role. Builds no
    /// description.
    pub(crate) fn is_projected(&self, id: WidgetId) -> bool {
        self.a11y_id_is_visible(id)
            && self.within_reach(id)
            && self
                .nodes
                .get(id)
                .is_some_and(|node| Self::effective_role(node).is_some())
    }

    /// Projects one node with the children the projection gives it, or
    /// [`None`] when the id is stale, hidden above or at `id`, or reports no
    /// role.
    pub(crate) fn semantics_node(&self, id: WidgetId) -> Option<SemanticsNode> {
        if !self.a11y_id_is_visible(id) {
            return None;
        }
        let node = self.nodes.get(id)?;
        let role = Self::effective_role(node)?;
        let parent = self.nearest_projected_ancestor(id);
        let mut out = Self::project_node(id, node, role, parent);
        self.collect_projected_children(id, &mut out.children);
        Some(out)
    }

    /// Returns the nearest ancestor of `id` that projects, or [`None`] when
    /// every ancestor is unprojected.
    fn nearest_projected_ancestor(&self, id: WidgetId) -> Option<WidgetId> {
        // The top node of an overlay projects as a root of its own, beside
        // the main tree, as the projection walks it.
        let mut next = self.pointer_parent(id);
        while let Some(current) = next {
            let node = self.nodes.get(current)?;
            if !node.withdrawn() && Self::effective_role(node).is_some() {
                return Some(current);
            }
            next = self.pointer_parent(current);
        }
        None
    }

    /// Walks `id`'s visible subtree in paint order and appends the projected
    /// nodes to `nodes`.
    fn project_from(
        &self,
        id: WidgetId,
        projected_parent: Option<WidgetId>,
        nodes: &mut Vec<SemanticsNode>,
        index_of: &mut HashMap<WidgetId, usize>,
    ) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        let Some(role) = Self::effective_role(node) else {
            for &child in &node.children {
                self.project_from(child, projected_parent, nodes, index_of);
            }
            return;
        };
        let index = nodes.len();
        nodes.push(Self::project_node(id, node, role, projected_parent));
        index_of.insert(id, index);
        if let Some(parent) = projected_parent
            && let Some(&parent_index) = index_of.get(&parent)
        {
            nodes[parent_index].children.push(id);
        }
        for &child in &node.children {
            self.project_from(child, Some(id), nodes, index_of);
        }
    }

    /// Builds the projected form of one node.
    fn project_node(
        id: WidgetId,
        node: &Node,
        role: Role,
        parent: Option<WidgetId>,
    ) -> SemanticsNode {
        // The widget builds its description, value included, once per
        // projection, since a text widget copies its whole text into it.
        let widget: &dyn Widget = node.widget.as_ref();
        let semantics = widget.semantics();
        let semantics = semantics.unwrap_or_else(|| Semantics::new(role));
        let Semantics {
            label,
            value,
            checked,
            range,
            position,
            set_size,
            level,
            mut text_runs,
            text_selection,
            links,
            description,
            selected,
            expanded,
            ..
        } = semantics;
        let content = node.content.origin();
        for run in &mut text_runs {
            run.bounds = run.bounds.translated(content);
        }
        let mut label = node.a11y_label.clone().or(label);
        let extra = node.a11y_extra.as_deref();
        let mut value = extra.and_then(|extra| extra.value.clone()).or(value);
        // A screen reader announces a heading or a link by its name, so one
        // with no name of its own takes its text, and the text is no value
        // of it, which some readers would announce again or take for a
        // link's address.
        if matches!(role, Role::Heading | Role::Link) {
            let text = value.take();
            label = label.or(text);
        }
        SemanticsNode {
            id,
            parent,
            children: Vec::new(),
            role,
            label,
            value,
            checked: node.a11y_checked.or(checked),
            range: node.a11y_range.as_deref().copied().or(range),
            position: node.a11y_position.or(position),
            set_size: node.a11y_set_size.or(set_size),
            level: node.a11y_level.or(level),
            text_runs,
            text_selection,
            links,
            description: extra
                .and_then(|extra| extra.description.clone())
                .or(description),
            selected: extra.and_then(|extra| extra.selected).or(selected),
            expanded: extra.and_then(|extra| extra.expanded).or(expanded),
            bounds: node.rect,
            focusable: node.interaction.has(Interaction::FOCUSABLE),
            disabled: node.interaction.has(Interaction::DISABLED),
            focused: node.interaction.has(Interaction::FOCUSED),
            pressable: node.handlers.on_press.is_some(),
        }
    }

    /// Appends the ids of `id`'s projected descendants to `out`, walking
    /// through unprojected children.
    fn collect_projected_children(&self, id: WidgetId, out: &mut Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        for &child in &node.children {
            let Some(child_node) = self.nodes.get(child) else {
                continue;
            };
            if child_node.withdrawn() {
                continue;
            }
            if Self::effective_role(child_node).is_some() {
                out.push(child);
            } else {
                self.collect_projected_children(child, out);
            }
        }
    }

    /// Returns the role the projection reports for the node: the one its
    /// element set, else the one its widget reports.
    fn effective_role(node: &Node) -> Option<Role> {
        node.a11y_role.or_else(|| {
            let widget: &dyn Widget = node.widget.as_ref();
            widget.role()
        })
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
            let layout_parent = if self.is_overlay_root(id) {
                None
            } else {
                node.parent.map(|parent| self.nodes[parent].taffy)
            };
            assert_eq!(
                self.taffy.parent(node.taffy),
                layout_parent,
                "the parent agrees, and an overlay is a layout root"
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
        for trap in &self.focus_traps {
            assert!(
                self.nodes.contains_key(trap.node),
                "every mounted focus trap is a live node"
            );
        }
        for overlay in &self.overlays {
            let host = self.nodes.get(overlay.root).and_then(|node| node.parent);
            assert!(
                host.is_some_and(|host| self
                    .nodes
                    .get(host)
                    .is_some_and(|host| !host.children.contains(&overlay.root))),
                "every overlay's top node is live, with a live host that does not list it"
            );
        }
        for id in &self.pending_autofocus {
            assert!(
                self.nodes.contains_key(*id),
                "every queued autofocus request belongs to a live node"
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
