//! Pointer routing: hit testing, hover and press.
//!
//! [`App::handle_event`](crate::app::App::handle_event) hands the pointer
//! events it receives to the tree, which finds the node under the pointer and
//! keeps the interaction bits current. That state lives on the nodes instead
//! of in signals: a change marks the node for style re-resolution, which is
//! where style variants will read it. Application code that needs a hover
//! change in its own logic writes its own signal from
//! [`on_hover`](crate::element::Element::on_hover).

use crate::geometry::Point;
use crate::input::PointerButton;

use super::{WidgetId, WidgetTree};

/// The pointer state of one node, kept as bits.
///
/// The routing sets and clears the bits as the pointer moves and presses. A
/// change marks the node for style re-resolution, so a style that depends on
/// the state of a node is resolved again before it paints.
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
}

impl Handlers {
    /// Returns whether any handler is set.
    ///
    /// Hit testing makes the deepest node under the pointer that has a handler
    /// the target, so a node without any lets a press pass to its ancestors.
    pub(crate) fn has_any(&self) -> bool {
        self.on_press.is_some()
            || self.on_hover.is_some()
            || self.on_pointer_down.is_some()
            || self.on_press_outside.is_some()
    }
}

impl WidgetTree {
    /// Routes a pointer move to the node under `position`.
    ///
    /// The hovered chain becomes the new target and its ancestors. While a
    /// press is held, the captured node keeps the pointer and hover does not
    /// change, because a drag needs the moves to stay with the node the press
    /// started on.
    pub(crate) fn pointer_move(&mut self, position: Point) {
        if self.pressed.is_some() {
            return;
        }
        let target = self.target(position);
        self.set_hover_target(target);
    }

    /// Routes a pointer down at `position`.
    ///
    /// A primary press makes the node under the pointer the target, runs its
    /// `on_pointer_down` with the position, sets its pressed bit and captures
    /// it. Every node that asks for presses outside the target hears one. A
    /// press also updates hover, because a press can arrive without a move
    /// before it, except while a press is already held: a non-primary down
    /// then leaves hover and the capture alone, and a primary press replaces
    /// the capture.
    pub(crate) fn pointer_down(&mut self, position: Point, button: PointerButton) {
        if self.pressed.is_some() && button != PointerButton::Primary {
            return;
        }
        let target = self.target(position);
        self.set_hover_target(target);
        if button != PointerButton::Primary {
            return;
        }
        if let Some(previous) = self.pressed.take() {
            self.set_bit(previous, Interaction::PRESSED, false);
        }
        self.run_press_outside(target);
        let Some(target) = target else {
            return;
        };
        self.run_pointer_down(target, position);
        self.set_bit(target, Interaction::PRESSED, true);
        self.pressed = Some(target);
    }

    /// Routes a pointer up at `position`.
    ///
    /// Releasing the primary button clears the captured node's pressed bit,
    /// and runs its `on_press` when the release happens over the captured
    /// node, which is the case when the node under the pointer is the captured
    /// node or one of its descendants. Hover follows the pointer again either
    /// way.
    pub(crate) fn pointer_up(&mut self, position: Point, button: PointerButton) {
        if button != PointerButton::Primary {
            return;
        }
        let target = self.target(position);
        let Some(captured) = self.pressed.take() else {
            self.set_hover_target(target);
            return;
        };
        self.set_bit(captured, Interaction::PRESSED, false);
        self.set_hover_target(target);
        if target.is_some_and(|target| self.chain(Some(target)).contains(&captured)) {
            self.run_press(captured);
        }
    }

    /// Clears the hover of the whole tree, as when the pointer leaves the
    /// window.
    ///
    /// A press that is still held keeps its capture, so a drag that leaves the
    /// window stays with the node it started on.
    pub(crate) fn pointer_left(&mut self) {
        self.set_hover_target(None);
    }

    /// Installs the handlers a node was mounted with.
    ///
    /// Called once, while the node is being mounted. A stale id does nothing.
    pub(crate) fn set_handlers(&mut self, id: WidgetId, handlers: Handlers) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.handlers = handlers;
        }
    }

    /// Sets whether the node is disabled, and marks the node when the state
    /// changed.
    ///
    /// Called when a node mounts and again by the binding of a
    /// [`disabled`](crate::element::Element::disabled) builder value. A stale
    /// id does nothing.
    pub(crate) fn set_disabled(&mut self, id: WidgetId, disabled: bool) {
        self.set_bit(id, Interaction::DISABLED, disabled);
    }

    /// Returns the deepest node under `position` that has a handler, or `None`
    /// when no node under the pointer can respond.
    ///
    /// The deepest hit node starts the search, and the first ancestor with a
    /// handler becomes the target.
    fn target(&self, position: Point) -> Option<WidgetId> {
        let root = self.root?;
        let mut id = self.hit(root, position)?;
        while !self.nodes.get(id)?.handlers.has_any() {
            id = self.nodes.get(id)?.parent?;
        }
        Some(id)
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
        if node.style.layout.hidden || !node.clip.contains(position) {
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

    /// Makes `target` and its ancestors the hovered chain.
    ///
    /// The nodes that leave the chain clear their bit and hear `on_hover`
    /// with `false` before the nodes that join it are set and hear `on_hover`
    /// with `true`.
    fn set_hover_target(&mut self, target: Option<WidgetId>) {
        if target == self.hovered {
            return;
        }
        let old = self.chain(self.hovered);
        let new = self.chain(target);
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
    fn set_bit(&mut self, id: WidgetId, bit: u8, on: bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if !node.interaction.set(bit, on) {
            return;
        }
        self.interaction_changed(id);
        if bit == Interaction::HOVERED {
            self.run_hover(id, on);
        }
    }

    /// Marks that the interaction state of `id` changed.
    ///
    /// Style variants will resolve from the interaction bits, so a change has
    /// to reach the style the node paints with. Nothing reads the bits yet, so
    /// this stays empty rather than repainting nodes whose pixels cannot have
    /// changed.
    fn interaction_changed(&mut self, _id: WidgetId) {}

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

    /// Runs `on_press` of `id` inside the node's owner.
    fn run_press(&mut self, id: WidgetId) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_press) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_press.as_mut())
            {
                on_press();
            }
        });
    }

    /// Runs `on_pointer_down` of `id` inside the node's owner.
    fn run_pointer_down(&mut self, id: WidgetId, position: Point) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_pointer_down) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_down.as_mut())
            {
                on_pointer_down(position);
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
    fn collect_press_outside(&self, id: Option<WidgetId>, outside: &mut Vec<WidgetId>) {
        let Some(id) = id else {
            return;
        };
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.handlers.on_press_outside.is_some() {
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
