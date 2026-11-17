//! Pointer routing: hit testing, hover and press.
//!
//! [`App::handle_event`](crate::app::App::handle_event) hands the pointer
//! events it receives to the tree, which finds the node under the pointer and
//! keeps the interaction bits current. That state lives on the nodes instead
//! of in signals: a change marks the node for style re-resolution, which is
//! where the style variants read it. Application code that needs a hover
//! change in its own logic writes its own signal from
//! [`on_hover`](crate::element::Element::on_hover). Keyboard focus and key
//! routing live beside it in [`super::focus`].

use crate::geometry::Point;
use crate::geometry::Rect;
use crate::input::{Drag, Handled, KeyEvent, PointerButton};

use super::{WidgetId, WidgetTree};

/// The interaction state of one node, kept as bits.
///
/// The routing sets and clears the bits as the pointer moves, buttons are
/// pressed and focus moves. A change marks the node for style re-resolution,
/// so a style that depends on the state of a node is resolved again before it
/// paints.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Interaction(u8);

impl Interaction {
    /// The pointer is over the node or over one of its descendants.
    pub(crate) const HOVERED: u8 = 1 << 0;

    /// A primary press started on the node and is still held.
    pub(crate) const PRESSED: u8 = 1 << 1;

    /// The node is left out of hit testing, so it receives no pointer events
    /// of its own.
    pub(crate) const DISABLED: u8 = 1 << 2;

    /// The node holds the keyboard focus.
    pub(crate) const FOCUSED: u8 = 1 << 3;

    /// The node holds the keyboard focus and the focus last moved by keyboard
    /// navigation, so a focus ring can draw for keyboard users alone.
    pub(crate) const KEYBOARD_FOCUS: u8 = 1 << 4;

    /// The node can take the keyboard focus, by a press or by Tab.
    pub(crate) const FOCUSABLE: u8 = 1 << 5;

    /// Returns whether `bit` is set.
    pub(crate) fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    /// Sets or clears `bit`, and returns whether the state changed.
    pub(crate) fn set(&mut self, bit: u8, on: bool) -> bool {
        let changed = self.has(bit) != on;
        if changed {
            if on {
                self.0 |= bit;
            } else {
                self.0 &= !bit;
            }
        }
        changed
    }
}

/// A key handler: it decides whether the event stops bubbling to ancestors.
pub(crate) type KeyHandler = Box<dyn FnMut(&KeyEvent) -> Handled>;

/// A commit handler: it inserts the text an input method committed.
pub(crate) type CommitHandler = Box<dyn FnMut(&str)>;

/// A preedit handler: it shows or clears the text an input method composes.
pub(crate) type PreeditHandler = Box<dyn FnMut(&str, Option<(usize, usize)>)>;

/// The input handlers one node owns.
///
/// The builder methods on [`Element`](crate::element::Element) set them, and
/// the tree keeps them for as long as the node lives. Every handler runs
/// inside its node's reactive owner, so it sees the same context the node's
/// bindings see.
#[derive(Default)]
pub(crate) struct Handlers {
    /// Runs when the primary button is released over the node after a press
    /// that started on it.
    pub(crate) on_press: Option<Box<dyn FnMut()>>,
    /// Runs when the node enters or leaves the hovered chain.
    pub(crate) on_hover: Option<Box<dyn FnMut(bool)>>,
    /// Runs when a pointer button is pressed on the node.
    pub(crate) on_pointer_down: Option<Box<dyn FnMut(Point)>>,
    /// Runs when a primary press targets a node that is neither this node nor
    /// one of its descendants.
    pub(crate) on_press_outside: Option<Box<dyn FnMut()>>,
    /// Runs when a key event reaches the node while it or a descendant holds
    /// the focus. Returning [`Handled::Yes`] stops the event from bubbling.
    pub(crate) on_key: Option<KeyHandler>,
    /// Runs when the wheel scrolls while the node is in the hovered chain.
    /// Returning [`Handled::Yes`] stops the event from bubbling.
    pub(crate) on_wheel: Option<Box<dyn FnMut(Point) -> Handled>>,
    /// Runs when a pointer move happens while a press on the node is held,
    /// or, with no press held, when the node is the hover target.
    pub(crate) on_pointer_move: Option<Box<dyn FnMut(Point)>>,
    /// Runs when the primary button is released after a press on the node,
    /// whether or not the release happened over the node, or, with no press
    /// held, when it happens over the node as the hover target.
    pub(crate) on_pointer_up: Option<Box<dyn FnMut(Point)>>,
    /// Runs when the input method commits text while the node or a
    /// descendant holds the focus. Routed like a key event, not a pointer
    /// event, so it takes no part in hit testing.
    pub(crate) on_ime_commit: Option<CommitHandler>,
    /// Runs when the input method changes its composing text while the node
    /// or a descendant holds the focus. An empty text clears the
    /// composition. Routed like a commit.
    pub(crate) on_ime_preedit: Option<PreeditHandler>,
    /// Runs when the node gains or loses the keyboard focus.
    pub(crate) on_focus: Option<Box<dyn FnMut(bool)>>,
    /// Runs, while a primary press is held, when the node is the deepest one
    /// under the pointer that sets it. It takes no part in hit testing.
    pub(crate) on_drag_over: Option<Box<dyn FnMut(Point)>>,
    /// Runs while a primary press that began inside the node's rectangle is
    /// held, and when it ends. Returning [`Handled::Yes`] for a move keeps it
    /// from the observers around the node. It takes no part in hit testing.
    pub(crate) on_drag: Option<Box<dyn FnMut(Drag) -> Handled>>,
    /// Answers whether the node responds to the pointer at a point relative
    /// to its rectangle. Where it answers `false`, hit testing passes the
    /// node by as if it had no handlers.
    pub(crate) responds_at: Option<Box<dyn Fn(Point) -> bool>>,
    /// Runs with the node's new scroll offset when the tree scrolled the node
    /// to bring the focused node into view. Its presence is what lets the
    /// tree scroll the node for that, and it takes no part in hit testing.
    pub(crate) on_reveal: Option<Box<dyn FnMut(Point)>>,
    /// Whether a fresh press of Enter or Space runs the node's
    /// [`on_press`](Self::on_press) handler, which is how a button fires from
    /// the keyboard.
    pub(crate) activate_on_keys: bool,
}

/// The drag a held primary press makes, from the press until its release,
/// even when the node that took the press goes away.
pub(crate) struct DragState {
    /// The deepest node the press began over, whose nearest scrolling
    /// ancestor gives the area a drag-over falls back to.
    origin: Option<WidgetId>,
    /// The visible area of that node when the press began, the fallback for
    /// an origin that went away.
    origin_clip: Rect,
    /// The nodes that hear the drag, deepest first: those that set `on_drag`
    /// and whose rectangle held the point the press began at.
    observers: Vec<WidgetId>,
    /// The node and local position the last drag-over reached, so a delivery
    /// that would repeat it is skipped.
    last_over: Option<(WidgetId, Point)>,
}

/// Returns `point` moved into `rect`, a hair inside its far edges so the
/// result still lies in it.
fn clamp_into(point: Point, rect: Rect) -> Point {
    let inside = |value: f32, low: f32, high: f32| value.max(low).min((high - 0.5).max(low));
    Point {
        x: inside(point.x, rect.x0, rect.x1),
        y: inside(point.y, rect.y0, rect.y1),
    }
}

/// Returns whether `position` lies in the region where `node` responds to
/// the pointer, which is everywhere unless it set
/// [`responds_at`](Handlers::responds_at).
fn in_region(node: &super::Node, position: Point) -> bool {
    node.handlers.responds_at.as_ref().is_none_or(|region| {
        let origin = node.rect.origin();
        region(Point {
            x: position.x - origin.x,
            y: position.y - origin.y,
        })
    })
}

impl Handlers {
    /// Returns whether any handler is set that makes the node a pointer
    /// target.
    ///
    /// Hit testing makes the deepest node under the pointer that can respond
    /// the target, so a node without any lets a press pass to its ancestors. A
    /// node can also respond by being focusable, which
    /// [`Interaction::FOCUSABLE`] tracks. A commit handler is left out,
    /// because a commit never comes from the pointer.
    pub(crate) fn has_any(&self) -> bool {
        self.on_press.is_some()
            || self.on_hover.is_some()
            || self.on_pointer_down.is_some()
            || self.on_press_outside.is_some()
            || self.on_key.is_some()
            || self.on_wheel.is_some()
            || self.on_pointer_move.is_some()
            || self.on_pointer_up.is_some()
    }

    /// Returns whether any handler is set that makes the node take a press.
    ///
    /// The wheel is left out: it reaches the hovered chain, so a node that
    /// only scrolls, such as a scroll view, lets presses on it pass to the
    /// nearest ancestor that takes them.
    pub(crate) fn takes_press(&self) -> bool {
        self.on_press.is_some()
            || self.on_hover.is_some()
            || self.on_pointer_down.is_some()
            || self.on_press_outside.is_some()
            || self.on_key.is_some()
            || self.on_pointer_move.is_some()
            || self.on_pointer_up.is_some()
    }
}

impl WidgetTree {
    /// Routes a pointer move to the node under `position`.
    ///
    /// While a press is held, the captured node keeps the pointer and hears
    /// its `on_pointer_move`, so a drag stays with the node the press started
    /// on; hover does not change and does not come back while the drag lasts,
    /// even when the pointer returns over the captured node. Without a press,
    /// the hovered chain becomes the new target and its ancestors, and the
    /// node under the pointer hears its `on_pointer_move`.
    pub(crate) fn pointer_move(&mut self, position: Point) {
        self.pointer = position;
        self.pointer_inside = true;
        if self.drag.is_some() {
            self.deliver_drag_over();
            self.deliver_drag(position);
        }
        if let Some(captured) = self.pressed {
            self.run_pointer_move(captured, position);
            return;
        }
        let target = self.hover_target(position);
        self.set_hover_target(target);
        if let Some(target) = target {
            self.run_pointer_move(target, position);
        }
    }

    /// Routes a pointer down at `position`.
    ///
    /// A primary press makes the node under the pointer the target, runs its
    /// `on_pointer_down` with the position, sets its pressed bit and captures
    /// it. Every node that asks for presses outside the target hears one. A
    /// press also updates hover, because a press can arrive without a move
    /// before it, except while a press is already held: a non-primary down
    /// then leaves hover and the capture alone, and a primary press replaces
    /// the capture. The replaced node hears `on_pointer_up` first, because the
    /// release of its press never arrived, so a drag it started still ends.
    /// Any processed press turns the keyboard navigation mode
    /// off, and a primary press focuses the target when it can take the focus
    /// or clears the focus when it cannot. While a focus trap is mounted, a
    /// press outside the trap leaves the focus where it is instead.
    pub(crate) fn pointer_down(&mut self, position: Point, button: PointerButton) {
        self.pointer = position;
        self.pointer_inside = true;
        if self.pressed.is_some() && button != PointerButton::Primary {
            return;
        }
        let target = self.target(position);
        self.set_hover_target(self.hover_target(position));
        self.set_keyboard_nav(false);
        if button != PointerButton::Primary {
            return;
        }
        self.released_early = false;
        // A press outside the active focus trap is not a focus gesture: the
        // focus stays wherever the trap holds it, on nothing when it holds
        // nothing.
        let outside_trap = self
            .active_focus_trap()
            .is_some_and(|trap| target.is_none_or(|target| !self.is_within(target, trap)));
        if !outside_trap {
            if target.is_some_and(|id| self.focusable(id)) {
                self.set_focus(target);
            } else {
                self.set_focus(None);
            }
            // A press is a focus gesture, so it also ends the reach of a
            // focus that a disabled or hidden node lost earlier, and drops a
            // reveal an earlier focus move asked for, even when the press
            // lands on the node that move focused.
            self.stale_focus = None;
            self.reveal_focus = false;
        }
        self.end_drag();
        if let Some(previous) = self.pressed.take() {
            self.set_bit(previous, Interaction::PRESSED, false);
            self.run_pointer_up(previous, position);
        }
        self.run_press_outside(target);
        let Some(target) = target else {
            return;
        };
        self.start_drag(position);
        self.deliver_drag_over();
        self.run_pointer_down(target, position);
        self.set_bit(target, Interaction::PRESSED, true);
        self.pressed = Some(target);
    }

    /// Routes a pointer up at `position`.
    ///
    /// Releasing the primary button clears the captured node's pressed bit,
    /// runs its `on_press` when the release happens over the captured node,
    /// which is the case when the node under the pointer is the captured node
    /// or one of its descendants, and runs its `on_pointer_up` whether or not
    /// the release happened over it, so a drag that ends elsewhere still
    /// hears the button come up. With no press held, the hover target hears
    /// `on_pointer_up`, unless the release belongs to a press that
    /// [`WidgetTree::release_press`] already ended: then no handler runs.
    /// Hover follows the pointer again either way.
    pub(crate) fn pointer_up(&mut self, position: Point, button: PointerButton) {
        if button != PointerButton::Primary {
            return;
        }
        self.pointer = position;
        self.pointer_inside = true;
        // The drag outlives the capture: the node that took the press may be
        // gone, and the observers still hear the release.
        self.end_drag();
        let hovered = self.hover_target(position);
        let Some(captured) = self.pressed.take() else {
            self.set_hover_target(hovered);
            if std::mem::take(&mut self.released_early) {
                return;
            }
            if let Some(hovered) = hovered {
                self.run_pointer_up(hovered, position);
            }
            return;
        };
        self.set_bit(captured, Interaction::PRESSED, false);
        self.set_hover_target(hovered);
        // The release counts over the captured node by the hover target, which
        // passes over disabled content, so a press is not lost to a disabled
        // control inside the node it started on.
        if hovered.is_some_and(|hovered| self.chain(Some(hovered)).contains(&captured)) {
            self.run_press(captured);
        }
        self.run_pointer_up(captured, position);
    }

    /// Ends the held press without a release: the captured node loses its
    /// pressed bit and hears `on_pointer_up`, so a drag it started ends, but
    /// not `on_press`. Hover then follows the pointer again. The press's own
    /// release, when it still arrives, reaches no handler, because the
    /// window system may refuse the gesture it was released for. Without a
    /// held press it does nothing.
    pub(crate) fn release_press(&mut self) {
        self.end_drag();
        let Some(captured) = self.pressed.take() else {
            return;
        };
        self.released_early = true;
        self.set_bit(captured, Interaction::PRESSED, false);
        self.run_pointer_up(captured, self.pointer);
        let hovered = if self.pointer_inside {
            self.hover_target(self.pointer)
        } else {
            None
        };
        self.set_hover_target(hovered);
    }

    /// Clears the hover of the whole tree, as when the pointer leaves the
    /// window.
    ///
    /// A press that is still held keeps its capture, so a drag that leaves the
    /// window stays with the node it started on.
    pub(crate) fn pointer_left(&mut self) {
        self.pointer_inside = false;
        self.set_hover_target(None);
    }

    /// Routes a wheel scroll to the deepest node in the hovered chain that
    /// takes it.
    ///
    /// The hovered chain is the chain of the node under the pointer that can
    /// respond, deepest first, so content inside a scroll view is walked
    /// before the view itself. The first handler that returns
    /// [`Handled::Yes`] takes the whole event, so of two nested scroll views
    /// the one under the pointer scrolls and the one around it does not, and
    /// a handler that returns [`Handled::No`] lets the event bubble on. A
    /// disabled or hidden node's handler never runs, and the event keeps
    /// bubbling past it, as key routing does.
    pub(crate) fn wheel(&mut self, delta: Point) {
        let Some(target) = self.hover_target(self.pointer) else {
            return;
        };
        for id in self.chain(Some(target)) {
            if self.nodes.get(id).is_some_and(|node| {
                node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
            }) {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            let mut handled = Handled::No;
            owner.with(|| {
                if let Some(on_wheel) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_wheel.as_mut())
                {
                    handled = on_wheel(delta);
                }
            });
            if handled == Handled::Yes {
                return;
            }
        }
    }

    /// Returns whether the tree holds work only a frame can finish: a node to
    /// lay out or paint, a focused node to scroll into view, or the pixels of
    /// a removed node to clear.
    pub(crate) fn has_pending_work(&self) -> bool {
        self.layout_dirty
            || self.reveal_focus
            || self.has_reveal_request()
            || !self.paint_queue.is_empty()
            || !self.pending_damage.is_empty()
    }

    /// Installs the handlers a node was mounted with.
    ///
    /// Called once, while the node is being mounted. A stale id does nothing.
    pub(crate) fn set_handlers(&mut self, id: WidgetId, handlers: Handlers) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.handlers = handlers;
        }
    }

    /// Sets the cursor the pointer shows over the node, or `None` to leave
    /// the choice to an ancestor.
    ///
    /// Called when a node mounts and again by the binding of a
    /// [`cursor`](crate::element::Element::cursor) builder value. A stale id
    /// does nothing.
    pub(crate) fn set_cursor(&mut self, id: WidgetId, cursor: Option<crate::window::Cursor>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.cursor = cursor;
        }
    }

    /// Returns the cursor of the deepest enabled node under the pointer that
    /// set one, or the platform default when none did.
    ///
    /// The walk starts at the deepest node the pointer hits, not at the hover
    /// target, so a node that sets only a cursor shows it without becoming a
    /// target. A node whose [`responds_at`](Handlers::responds_at) region
    /// leaves the pointer out shows no cursor of its own there. While a
    /// press is held, the walk starts there only when that node lies inside
    /// the captured node, and at the captured node otherwise, because hover
    /// stays with the capture during a drag. A pointer outside the window
    /// shows the default, unless a press is held.
    pub(crate) fn cursor(&self) -> crate::window::Cursor {
        let hit = if self.pointer_inside {
            self.root.and_then(|root| self.hit(root, self.pointer))
        } else {
            None
        };
        let mut next = match self.pressed {
            Some(pressed) => Some(
                hit.filter(|&hit| self.is_within(hit, pressed))
                    .unwrap_or(pressed),
            ),
            None => hit,
        };
        while let Some(id) = next {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            if !node.interaction.has(Interaction::DISABLED)
                && let Some(cursor) = node.cursor
                && in_region(node, self.pointer)
            {
                return cursor;
            }
            next = node.parent;
        }
        crate::window::Cursor::default()
    }

    /// Sets whether the node is disabled, and marks the node when the state
    /// changed.
    ///
    /// Called when a node mounts and again by the binding of a
    /// [`disabled`](crate::element::Element::disabled) builder value. A node
    /// that is switched off loses the keyboard focus and, while no press is
    /// held, gives up its place in the hovered chain, because hover passes
    /// over a disabled node. A stale id does nothing.
    pub(crate) fn set_disabled(&mut self, id: WidgetId, disabled: bool) {
        self.set_bit(id, Interaction::DISABLED, disabled);
        self.push_a11y(id);
        if !disabled {
            return;
        }
        self.drop_invalid_focus();
        // While a press is held the capture keeps the hover of its node, so
        // the chain is only rebuilt when the pointer is free.
        if self.pressed.is_none()
            && self
                .hovered
                .is_some_and(|hovered| self.chain(Some(hovered)).contains(&id))
        {
            let target = self.hover_target(self.pointer);
            self.set_hover_target(target);
        }
    }

    /// Sets whether the node can take the keyboard focus, and marks the node
    /// when the state changed.
    ///
    /// Called once, while the node is being mounted. A stale id does nothing.
    pub(crate) fn set_focusable(&mut self, id: WidgetId, focusable: bool) {
        self.set_bit(id, Interaction::FOCUSABLE, focusable);
    }

    /// Returns the deepest node under `position` that takes a press, or
    /// `None` when no node under the pointer takes one.
    ///
    /// The deepest hit node starts the search, and the first ancestor that
    /// takes a press becomes the target. A node that only scrolls with the
    /// wheel takes none, so a press on a scroll view reaches what is around
    /// it. When that ancestor is disabled, the press
    /// lands nowhere: a disabled control swallows presses on its content
    /// rather than passing them to the nodes around it.
    fn target(&self, position: Point) -> Option<WidgetId> {
        let root = self.root?;
        let mut id = self.hit(root, position)?;
        while !self.takes_press_at(id, position) {
            id = self.nodes.get(id)?.parent?;
        }
        let disabled = self.nodes.get(id)?.interaction.has(Interaction::DISABLED);
        (!disabled).then_some(id)
    }

    /// Returns the deepest node under `position` that hover lands on, or
    /// `None` when no enabled node under the pointer can respond.
    ///
    /// Unlike a press, hover passes over a disabled node to the nearest
    /// enabled one that can respond, so a container stays hovered while the
    /// pointer is over a disabled control inside it.
    pub(super) fn hover_target(&self, position: Point) -> Option<WidgetId> {
        let root = self.root?;
        let mut id = self.hit(root, position)?;
        loop {
            let node = self.nodes.get(id)?;
            if self.can_respond_at(id, position) && !node.interaction.has(Interaction::DISABLED) {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    /// Returns whether a press or hover can land on the node: it has a
    /// handler, it can take the focus, or it carries a style variant, which
    /// makes a press or a hover visible without any handler.
    fn can_respond(&self, id: WidgetId) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        node.handlers.has_any()
            || node.interaction.has(Interaction::FOCUSABLE)
            || node
                .variants
                .as_ref()
                .is_some_and(|variants| !variants.is_empty())
    }

    /// Returns whether a press or hover at `position` can land on the node:
    /// it can respond, and its [`responds_at`](Handlers::responds_at) region,
    /// when it sets one, holds the point.
    fn can_respond_at(&self, id: WidgetId, position: Point) -> bool {
        if !self.can_respond(id) {
            return false;
        }
        self.nodes
            .get(id)
            .is_some_and(|node| in_region(node, position))
    }

    /// Returns whether a press at `position` can land on the node: its
    /// handlers, its focus or its style variants take one, and its
    /// [`responds_at`](Handlers::responds_at) region, when it sets one, holds
    /// the point.
    fn takes_press_at(&self, id: WidgetId, position: Point) -> bool {
        self.nodes.get(id).is_some_and(|node| {
            (node.handlers.takes_press()
                || node.interaction.has(Interaction::FOCUSABLE)
                || node
                    .variants
                    .as_ref()
                    .is_some_and(|variants| !variants.is_empty()))
                && in_region(node, position)
        })
    }

    /// Returns whether the node can take the keyboard focus.
    pub(super) fn focusable(&self, id: WidgetId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| node.interaction.has(Interaction::FOCUSABLE))
    }

    /// Returns the deepest node that `position` hits, walking children in
    /// reverse paint order so the topmost node wins.
    ///
    /// A node is hit when the point is inside its rectangle and its clip, and
    /// it is neither hidden nor disabled. The children of a node are visited
    /// even when the point is outside the node's own rectangle, because a
    /// child of a node that does not clip its children reaches past them. A
    /// disabled node is not hit, but its children still are.
    fn hit(&self, id: WidgetId, position: Point) -> Option<WidgetId> {
        let node = self.nodes.get(id)?;
        if node.resolved.layout.hidden || !node.clip.contains(position) {
            return None;
        }
        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit(*child, position) {
                return Some(hit);
            }
        }
        if node.rect.contains(position) && !node.interaction.has(Interaction::DISABLED) {
            return Some(id);
        }
        None
    }

    /// Returns `target` and its ancestors, deepest first.
    ///
    /// A stale id, which a removed node can leave behind, ends the chain
    /// there.
    fn chain(&self, target: Option<WidgetId>) -> Vec<WidgetId> {
        let mut chain = Vec::new();
        let mut next = target;
        while let Some(id) = next {
            chain.push(id);
            next = self.nodes.get(id).and_then(|node| node.parent);
        }
        chain
    }

    /// Makes `target` and its enabled ancestors the hovered chain.
    ///
    /// A disabled ancestor stays out of the chain, so its `on_hover` never
    /// runs, while the enabled nodes around it still count as hovered.
    ///
    /// The nodes that leave the chain clear their bit and hear `on_hover`
    /// with `false` before the nodes that join it are set and hear `on_hover`
    /// with `true`.
    pub(super) fn set_hover_target(&mut self, target: Option<WidgetId>) {
        if target == self.hovered {
            return;
        }
        let old = self.chain(self.hovered);
        let mut new = self.chain(target);
        new.retain(|&id| {
            self.nodes
                .get(id)
                .is_some_and(|node| !node.interaction.has(Interaction::DISABLED))
        });
        for id in old.iter().filter(|id| !new.contains(id)) {
            self.set_bit(*id, Interaction::HOVERED, false);
        }
        for id in new.iter().rev().filter(|id| !old.contains(id)) {
            self.set_bit(*id, Interaction::HOVERED, true);
        }
        self.hovered = target;
    }

    /// Sets or clears one interaction bit of `id`, and reports the change to
    /// the node.
    ///
    /// A stale id does nothing.
    pub(super) fn set_bit(&mut self, id: WidgetId, bit: u8, on: bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if !node.interaction.set(bit, on) {
            return;
        }
        self.interaction_changed(id);
        if bit == Interaction::HOVERED {
            self.run_hover(id, on);
        } else if bit == Interaction::FOCUSED {
            self.run_focus(id, on);
        }
    }

    /// Marks that the interaction state of `id` changed.
    ///
    /// The style variants resolve from the interaction bits, hover and focus
    /// among them, so a change has to reach the style the node paints with.
    /// Resolution does nothing for a node whose resolved style does not
    /// change, which keeps a hovered or focused node without patches from
    /// repainting.
    fn interaction_changed(&mut self, id: WidgetId) {
        self.re_resolve(id);
    }

    /// Runs `on_hover` of `id` inside the node's owner.
    fn run_hover(&mut self, id: WidgetId, hovered: bool) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_hover) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_hover.as_mut())
            {
                on_hover(hovered);
            }
        });
    }

    /// Runs `on_focus` of `id` inside the node's owner.
    fn run_focus(&mut self, id: WidgetId, focused: bool) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_focus) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_focus.as_mut())
            {
                on_focus(focused);
            }
        });
    }

    /// Runs `on_press` of `id` inside the node's owner, and returns whether
    /// the handler ran.
    ///
    /// A node that became disabled or hidden while its press was held does not
    /// run it, because a disabled node's handlers never run. Key activation
    /// shares this path, so a button fires the same handler from either input.
    pub(crate) fn run_press(&mut self, id: WidgetId) -> bool {
        if self.nodes.get(id).is_none_or(|node| {
            node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
        }) {
            return false;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return false;
        };
        let mut ran = false;
        owner.with(|| {
            if let Some(on_press) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_press.as_mut())
            {
                on_press();
                ran = true;
            }
        });
        ran
    }

    /// Returns `position`, in window coordinates, relative to the top left of
    /// the rectangle of `id`, which is where the node's pointer handlers
    /// measure from. A stale id leaves it as it is.
    fn local_position(&self, id: WidgetId, position: Point) -> Point {
        let origin = self
            .nodes
            .get(id)
            .map(|node| node.rect.origin())
            .unwrap_or_default();
        Point {
            x: position.x - origin.x,
            y: position.y - origin.y,
        }
    }

    /// Starts the drag of a primary press that began at `position`: records
    /// the visible area of the node under it and the nodes that observe a
    /// drag that begins inside them.
    fn start_drag(&mut self, position: Point) {
        let origin = self.root.and_then(|root| self.hit(root, position));
        let origin_clip = origin
            .and_then(|id| self.nodes.get(id))
            .map(|node| node.clip)
            .unwrap_or_default();
        let observers = self
            .chain(origin)
            .into_iter()
            .filter(|&id| {
                self.nodes.get(id).is_some_and(|node| {
                    node.handlers.on_drag.is_some() && node.rect.contains(position)
                })
            })
            .collect();
        self.drag = Some(DragState {
            origin,
            origin_clip,
            observers,
            last_over: None,
        });
    }

    /// Tells every observer that still exists that the drag ended, and
    /// forgets the drag.
    fn end_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        for id in drag.observers {
            self.run_drag(id, Drag::Ended);
        }
    }

    /// Runs `on_drag` of the observers of the held drag with the move to
    /// `position`, deepest first. Once one returns [`Handled::Yes`], the ones
    /// around it hear [`Drag::Taken`] instead. An observer that became
    /// disabled or hidden skips the move.
    fn deliver_drag(&mut self, position: Point) {
        let Some(observers) = self.drag.as_ref().map(|drag| drag.observers.clone()) else {
            return;
        };
        let mut taken = false;
        for id in observers {
            if self.nodes.get(id).is_none_or(|node| {
                node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
            }) {
                continue;
            }
            let step = if taken {
                Drag::Taken
            } else {
                Drag::Moved(self.local_position(id, position))
            };
            if self.run_drag(id, step) == Handled::Yes {
                taken = true;
            }
        }
    }

    /// Returns the area a drag-over falls back to: the visible area of the
    /// nearest scrolling ancestor of the node the press began over, as it is
    /// now, so a drag past a scroll view's edge reaches the row at that edge
    /// while the view scrolls. Without one, it is the visible area of that
    /// node, and once that node is gone, the area it had when the press
    /// began.
    fn drag_fallback_area(&self, drag: &DragState) -> Rect {
        let mut next = drag.origin;
        while let Some(id) = next {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            if node.resolved.layout.overflow == crate::style::Overflow::Scroll {
                return self.children_clip(id).unwrap_or(node.clip);
            }
            next = node.parent;
        }
        drag.origin
            .and_then(|id| self.nodes.get(id))
            .map_or(drag.origin_clip, |node| node.clip)
    }

    /// Runs `on_drag_over` of the deepest node under the pointer that sets it,
    /// while a primary press is held.
    ///
    /// When nothing under the pointer listens, as when the pointer left the
    /// scroll view the drag began in, the pointer is moved into the visible
    /// area the press began over, so the drag reaches the content at that
    /// area's edge. This runs on every move of the press, and again when
    /// content moves under a still pointer.
    pub(super) fn deliver_drag_over(&mut self) {
        let Some(area) = self.drag.as_ref().map(|drag| self.drag_fallback_area(drag)) else {
            return;
        };
        let pointer = self.pointer;
        let found = self
            .drag_over_target(pointer)
            .map(|id| (id, pointer))
            .or_else(|| {
                let clamped = clamp_into(pointer, area);
                (clamped != pointer)
                    .then(|| self.drag_over_target(clamped).map(|id| (id, clamped)))
                    .flatten()
            });
        let Some((id, position)) = found else {
            return;
        };
        let local = self.local_position(id, position);
        if let Some(drag) = self.drag.as_mut() {
            if drag.last_over == Some((id, local)) {
                return;
            }
            drag.last_over = Some((id, local));
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_drag_over) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_drag_over.as_mut())
            {
                on_drag_over(local);
            }
        });
    }

    /// Returns the deepest enabled node under `position` that sets
    /// `on_drag_over`.
    fn drag_over_target(&self, position: Point) -> Option<WidgetId> {
        let mut next = self.root.and_then(|root| self.hit(root, position));
        while let Some(id) = next {
            let node = self.nodes.get(id)?;
            if node.handlers.on_drag_over.is_some() && !node.interaction.has(Interaction::DISABLED)
            {
                return Some(id);
            }
            next = node.parent;
        }
        None
    }

    /// Runs `on_drag` of `id` inside the node's owner and returns what it
    /// answered, or [`Handled::No`] when the node is gone.
    fn run_drag(&mut self, id: WidgetId, step: Drag) -> Handled {
        let Some(owner) = self.owner(id).cloned() else {
            return Handled::No;
        };
        let mut handled = Handled::No;
        owner.with(|| {
            if let Some(on_drag) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_drag.as_mut())
            {
                handled = on_drag(step);
            }
        });
        handled
    }

    /// Runs `on_pointer_down` of `id` inside the node's owner.
    fn run_pointer_down(&mut self, id: WidgetId, position: Point) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_pointer_down) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_down.as_mut())
            {
                on_pointer_down(local);
            }
        });
    }

    /// Runs `on_pointer_move` of `id` inside the node's owner.
    ///
    /// A node that became disabled or hidden hears nothing, like every other
    /// pointer handler.
    fn run_pointer_move(&mut self, id: WidgetId, position: Point) {
        if self.nodes.get(id).is_none_or(|node| {
            node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
        }) {
            return;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_pointer_move) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_move.as_mut())
            {
                on_pointer_move(local);
            }
        });
    }

    /// Runs `on_pointer_up` of `id` inside the node's owner.
    ///
    /// A node that became disabled or hidden hears nothing, like every other
    /// pointer handler.
    fn run_pointer_up(&mut self, id: WidgetId, position: Point) {
        if self.nodes.get(id).is_none_or(|node| {
            node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
        }) {
            return;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_pointer_up) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_up.as_mut())
            {
                on_pointer_up(local);
            }
        });
    }

    /// Runs `on_press_outside` of every node that is not in the target's
    /// chain, in paint order.
    ///
    /// A press with no target is outside every node, so a press on the empty
    /// window closes every popup that asks.
    fn run_press_outside(&mut self, target: Option<WidgetId>) {
        let chain = self.chain(target);
        let mut outside = Vec::new();
        self.collect_press_outside(self.root, &mut outside);
        for id in outside {
            if chain.contains(&id) {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            owner.with(|| {
                if let Some(on_press_outside) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_press_outside.as_mut())
                {
                    on_press_outside();
                }
            });
        }
    }

    /// Appends every node of the subtree that asks for presses outside it, in
    /// paint order.
    ///
    /// A hidden node and its subtree are skipped, and a disabled node is
    /// skipped while its children are still visited, because the handlers of
    /// a hidden or disabled node never run.
    fn collect_press_outside(&self, id: Option<WidgetId>, outside: &mut Vec<WidgetId>) {
        let Some(id) = id else {
            return;
        };
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.resolved.layout.hidden {
            return;
        }
        if node.handlers.on_press_outside.is_some() && !node.interaction.has(Interaction::DISABLED)
        {
            outside.push(id);
        }
        for &child in &node.children {
            self.collect_press_outside(Some(child), outside);
        }
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
