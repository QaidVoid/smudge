//! Keyboard focus: which node holds it, how it moves, and how keys route.
//!
//! The focused node hears key events before they bubble to its ancestors.
//! Focus moves by a press on a focusable node or by Tab navigation, and the
//! keyboard navigation mode records which of the two moved it last, so a
//! focus ring can draw for keyboard users alone. A pointer press turns the
//! mode off again.

use crate::geometry::{Point, Rect};
use crate::input::{Handled, Key, KeyEvent};
use crate::reactive::prelude::{Get, Set};
use crate::style::Position;
use crate::widget::Widget;

use super::interaction::Interaction;
use super::overlay::Layer;
use super::{FocusTrap, WidgetId, WidgetTree};

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;

/// Returns whether `event` is a fresh press of Enter or Space, the keys that
/// activate a control.
///
/// Releases and key repeats activate nothing, so holding a key fires a button
/// once.
fn is_activation_key(event: &KeyEvent) -> bool {
    event.pressed && !event.repeat && matches!(event.key, Key::Enter | Key::Space)
}

impl WidgetTree {
    /// Routes a key event from the focused node through its ancestors.
    ///
    /// The focused node hears the key first, then its parent, and so on to the
    /// root, until a handler reports [`Handled::Yes`]. With nothing holding
    /// the focus, the key starts at the root, so a shortcut mounted there
    /// hears a key pressed anywhere in the window. While a focus trap is
    /// active, a key that would start outside it, because the focus is
    /// outside it or nothing holds the focus, starts at the trap instead and
    /// bubbles from there to the root. A press of Tab or an
    /// arrow key turns the keyboard navigation mode on, whether or not a
    /// handler took the key, and an unhandled Tab or Shift+Tab moves the focus
    /// to the next or previous focusable node in tree order, confined to the
    /// active focus trap when one is mounted. Releases and every other key
    /// only route.
    ///
    /// A focused node that became disabled or hidden since it took the focus
    /// loses it first, and its own handler does not run: a disabled button
    /// must not activate from the keyboard. The key still bubbles from it to
    /// the ancestors that can hear it, so shortcuts above it keep working.
    pub(crate) fn key(&mut self, event: &KeyEvent) {
        if event.pressed && event.key == Key::Escape && self.release_pointer_lock() {
            return;
        }
        // A node that lost the focus by becoming disabled or hidden still
        // anchors the next key, so shortcuts above it keep working. The key
        // starts at its parent: the node itself no longer holds the focus,
        // even if it has been enabled again since. It starts higher when
        // that parent is inside a hidden or fading subtree, above the
        // subtree, whose handlers hear nothing. With no anchor either,
        // because nothing ever held the focus or a press consumed the last
        // one, the key starts at the root.
        let start = match self.focused {
            Some(focused) => Some(focused),
            None => self
                .stale_focus
                .take()
                .and_then(|anchor| self.nodes.get(anchor)?.parent)
                .or(self.root),
        };
        let start = self.outside_withdrawn(start);
        // An active trap holds the keyboard: a key that would start outside
        // it starts at the trap, so the trap's own handlers hear it before
        // the shortcuts above it, and a control behind it never does.
        let start = self.confine_to_trap(start);
        self.drop_invalid_focus();
        let handled = self.route_key(start, event);
        if !event.pressed {
            return;
        }
        if handled != Handled::Yes && self.context_menu_key(event) {
            return;
        }
        if handled != Handled::Yes && self.dismiss_on_escape(event) {
            return;
        }
        match event.key {
            Key::Tab => {
                self.set_keyboard_nav(true);
                if handled != Handled::Yes {
                    self.move_focus(event.modifiers.shift);
                }
            }
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => {
                self.set_keyboard_nav(true);
            }
            _ => {}
        }
    }

    /// Returns the active focus trap when `start` lies outside it or is
    /// `None`, and `start` otherwise, so routed input never starts behind an
    /// active trap.
    fn confine_to_trap(&self, start: Option<WidgetId>) -> Option<WidgetId> {
        match self.active_focus_trap() {
            Some(trap) if start.is_none_or(|start| !self.is_within(start, trap)) => Some(trap),
            _ => start,
        }
    }

    /// Delivers the event to `start` and its ancestors, deepest first, and
    /// returns whether a handler took it.
    ///
    /// `start` is the node the key routes from: the focused node, the parent
    /// of the node the focus fell off of, or the root when nothing anchors
    /// the key.
    fn route_key(&mut self, start: Option<WidgetId>, event: &KeyEvent) -> Handled {
        let mut next = start;
        // Whether the key has bubbled out of an overlay into its host, past
        // which the arrows of the nodes around the host never move the focus
        // out of the overlay.
        let mut left_overlay = false;
        // Only the nearest container that navigates with the arrows decides,
        // so a nested one keeps its keys from the one around it. Inside an
        // overlay it takes them before they bubble on, so an open menu keeps
        // them from the shortcuts around its opener. Elsewhere it takes only
        // the keys no handler above it took.
        let mut arrows_seen = false;
        let mut deferred = None;
        while let Some(id) = next {
            next = self.nodes.get(id).and_then(|node| node.parent);
            let arrows_here = !left_overlay;
            left_overlay |= self.is_overlay_root(id);
            // A disabled or hidden node's handlers never run, but the key keeps
            // bubbling, so shortcuts above it still work.
            if self
                .nodes
                .get(id)
                .is_some_and(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
            {
                continue;
            }
            // A control that activates on Enter or Space fires its press
            // handler before any key handler of its own hears the key, and a
            // handler that ran takes the key.
            if is_activation_key(event)
                && self
                    .nodes
                    .get(id)
                    .is_some_and(|node| node.handlers.activate_on_keys)
                && self.run_press(id)
            {
                return Handled::Yes;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            let mut handled = Handled::No;
            owner.with(|| {
                if let Some(on_key) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_key.as_mut())
                {
                    handled = on_key(event);
                }
            });
            if handled == Handled::Yes {
                return Handled::Yes;
            }
            if arrows_here
                && !arrows_seen
                && let Some(axis) = self.nodes.get(id).and_then(|node| node.handlers.arrows)
            {
                arrows_seen = true;
                if !self.within_overlay(id) {
                    deferred = Some((id, axis));
                } else if self.navigate_in(id, axis, event) {
                    return Handled::Yes;
                }
            }
            // An Escape nothing inside an open overlay took closes it before
            // it reaches the nodes around the overlay's host.
            if event.pressed
                && event.key == Key::Escape
                && let Some(at) = self
                    .overlays
                    .iter()
                    .position(|overlay| overlay.root == id && overlay.dismissible())
                && self.overlays[at].open.get()
            {
                // A tooltip or a popup drawn above this overlay, which the
                // focus is not in, closes first, as Escape does in the tree.
                let open = self.overlays[at + 1..]
                    .iter()
                    .rev()
                    .find(|overlay| {
                        overlay.transient() && overlay.open.get() && self.overlay_shown(overlay)
                    })
                    .unwrap_or(&self.overlays[at])
                    .open
                    .clone();
                open.set(false);
                return Handled::Yes;
            }
            // Nothing behind a dialog hears a key the dialog left alone.
            if self
                .overlays
                .iter()
                .any(|overlay| overlay.root == id && overlay.layer == Layer::Dialog)
            {
                return Handled::No;
            }
        }
        if let Some((container, axis)) = deferred
            && self.navigate_in(container, axis, event)
        {
            return Handled::Yes;
        }
        Handled::No
    }

    /// Returns whether `id` lies inside an overlay, its top node included.
    fn within_overlay(&self, id: WidgetId) -> bool {
        let mut next = Some(id);
        while let Some(current) = next {
            if self.is_overlay_root(current) {
                return true;
            }
            next = self.nodes.get(current).and_then(|node| node.parent);
        }
        false
    }

    /// Returns whether an active focus trap holds the keyboard and `id`
    /// lies outside it, so a key meant for `id` would reach the trap.
    pub(crate) fn behind_trap(&self, id: WidgetId) -> bool {
        self.active_focus_trap()
            .is_some_and(|trap| !self.is_within(id, trap))
    }

    /// Delivers committed input method text to the focused node.
    ///
    /// The commit walks up from the focused node, like a key event, and the
    /// first `on_ime_commit` handler that runs takes it. A committed string
    /// can arrive with no key event at all: a commit is how an input method
    /// delivers the text the user composed, so an editor that reads keys
    /// alone would lose it. A focused node that lost the focus by becoming
    /// disabled or hidden does not hear the commit, but its ancestors still
    /// can.
    pub(crate) fn ime_commit(&mut self, text: &str) {
        self.route_from_focus(|tree, id| {
            let Some(on_ime_commit) = tree
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_ime_commit.as_mut())
            else {
                return false;
            };
            on_ime_commit(text);
            true
        });
    }

    /// Delivers a change of the composing text to the focused node.
    ///
    /// Non-empty text shows or updates the preedit, empty text clears it,
    /// which is how a canceled composition arrives. The routing is the
    /// commit's: the deepest `on_ime_preedit` handler in the focused chain
    /// takes it, and a node that can no longer hold the focus does not hear
    /// it while its ancestors still can.
    pub(crate) fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        self.route_from_focus(|tree, id| {
            let Some(on_ime_preedit) = tree
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_ime_preedit.as_mut())
            else {
                return false;
            };
            on_ime_preedit(text, cursor);
            true
        });
    }

    /// Runs `deliver` on the focused node and its ancestors, deepest first,
    /// and stops at the first handler that runs.
    ///
    /// The event starts at the node that held the focus when it arrived. A
    /// focused node that became disabled or hidden delivers nothing, but the
    /// event keeps bubbling past it, like a key does. While a focus trap is
    /// active, an event that would start outside it starts at the trap, as a
    /// key does, so text typed while a dialog is open never reaches an
    /// editor behind it.
    fn route_from_focus(&mut self, mut deliver: impl FnMut(&mut Self, WidgetId) -> bool) {
        let start = match self.focused {
            Some(focused) => Some(focused),
            None => self
                .stale_focus
                .take()
                .and_then(|anchor| self.nodes.get(anchor)?.parent),
        };
        let start = self.outside_withdrawn(start);
        let start = self.confine_to_trap(start);
        self.drop_invalid_focus();
        let mut next = start;
        while let Some(id) = next {
            next = self.nodes.get(id).and_then(|node| node.parent);
            if self
                .nodes
                .get(id)
                .is_some_and(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
            {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            let mut ran = false;
            owner.with(|| {
                ran = deliver(self, id);
            });
            if ran {
                return;
            }
        }
    }

    /// Moves the focus to the next focusable node in tree order, or to the
    /// previous one when `backwards` is set, wrapping at both ends.
    ///
    /// When the walk finds nothing to move to while a node holds the focus,
    /// which happens when the active focus trap holds no focusable node, the
    /// focus stays where it is. The node the focus lands on is scrolled into
    /// view by the next layout.
    fn move_focus(&mut self, backwards: bool) {
        let next = self.next_focusable(self.focused, backwards);
        if next.is_none() && self.focused.is_some() {
            return;
        }
        self.set_focus(next);
        self.scroll_focus_into_view();
    }

    /// Focuses `id`, or clears the focus when `id` is `None`.
    ///
    /// The node that loses the focus clears its focused bit and hears
    /// `on_focus` with `false` before the node that gains it is marked and
    /// hears `on_focus` with `true`. A node that already holds the focus keeps
    /// it without hearing `on_focus` again. The keyboard focus bit follows the
    /// keyboard navigation mode. Any change consumes the key anchor a dropped
    /// focus left behind, because the focus has moved on purpose, and drops a
    /// pending request to scroll the focus into view, which a caller that
    /// wants the new focus revealed makes again.
    pub(super) fn set_focus(&mut self, id: Option<WidgetId>) {
        if self.focused == id {
            return;
        }
        let old = self.focused;
        self.stale_focus = None;
        self.reveal_focus = false;
        if let Some(old) = self.focused {
            self.set_bit(old, Interaction::FOCUSED, false);
            self.set_bit(old, Interaction::KEYBOARD_FOCUS, false);
        }
        self.focused = id;
        if let Some(new) = id {
            self.set_bit(new, Interaction::FOCUSED, true);
            if self.keyboard_nav {
                self.set_bit(new, Interaction::KEYBOARD_FOCUS, true);
            }
        }
        if let Some(old) = old {
            self.push_a11y(old);
        }
        if let Some(new) = id {
            self.push_a11y(new);
        }
        self.run_focus_within(old, id, &[]);
    }

    /// Runs `on_focus_within` of the nodes the focus left, with `false`,
    /// deepest first, and of the nodes it entered, with `true`, outermost
    /// first, as it moves from `old` to `new`, through the hosts of overlays.
    /// The nodes of `removed`, which are being removed, hear nothing.
    pub(super) fn run_focus_within(
        &mut self,
        old: Option<WidgetId>,
        new: Option<WidgetId>,
        removed: &[WidgetId],
    ) {
        let before = self.lineage(old);
        let after = self.lineage(new);
        let left = before
            .iter()
            .filter(|id| !after.contains(id) && !removed.contains(id))
            .map(|&id| (id, false));
        let entered = after
            .iter()
            .rev()
            .filter(|id| !before.contains(id))
            .map(|&id| (id, true));
        let changes: Vec<(WidgetId, bool)> = left.chain(entered).collect();
        for (id, inside) in changes {
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            owner.with(|| {
                if let Some(handler) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_focus_within.as_mut())
                {
                    handler(inside);
                }
            });
        }
    }

    /// Focuses `id` without a pointer press, as an accessibility action,
    /// autofocus or a restored focus does, and returns whether it took the
    /// focus.
    ///
    /// A node that is not focusable, or that cannot hold the focus because it
    /// is disabled or hidden, is left alone and `false` is returned. A node
    /// that already holds the focus keeps it. Either way a node that holds
    /// the focus afterwards is scrolled into view by the next layout.
    pub(crate) fn request_focus(&mut self, id: WidgetId) -> bool {
        if !self.focusable(id) || !self.can_hold_focus(id) {
            return false;
        }
        self.set_focus(Some(id));
        self.scroll_focus_into_view();
        true
    }

    /// Turns the keyboard navigation mode on or off.
    ///
    /// The focused node carries the keyboard focus bit only while the mode is
    /// on, so a style can react to keyboard focus alone. A pointer press
    /// clears the mode, which clears the bit of a node that stays focused.
    pub(super) fn set_keyboard_nav(&mut self, on: bool) {
        if self.keyboard_nav == on {
            return;
        }
        self.keyboard_nav = on;
        if let Some(focused) = self.focused {
            self.set_bit(focused, Interaction::KEYBOARD_FOCUS, on);
        }
    }

    /// Returns the node that holds the keyboard focus, when one does.
    pub(crate) fn focused(&self) -> Option<WidgetId> {
        self.focused
    }

    /// Clears the focus when the node that holds it can no longer keep it.
    ///
    /// A node that became disabled or hidden loses the focus at once, so a
    /// focus style never draws on a control that is switched off, instead of
    /// holding it until the next key event notices. The node hears
    /// `on_focus` with `false` as it goes. The node stays as the anchor the
    /// next key event bubbles from, so a shortcut above it keeps working.
    pub(super) fn drop_invalid_focus(&mut self) {
        let Some(focused) = self
            .focused
            .filter(|focused| !self.can_hold_focus(*focused))
        else {
            return;
        };
        self.set_focus(None);
        self.stale_focus = Some(focused);
    }

    /// Returns whether the keyboard navigation mode is on.
    pub(crate) fn keyboard_nav(&self) -> bool {
        self.keyboard_nav
    }

    /// Returns the area the focused widget reports to the input method, when
    /// it reports one.
    ///
    /// The widget paints in paint coordinates, so the rectangle it reports is
    /// in paint coordinates too, and this translates it back into window
    /// coordinates with the node's shift. A focused node that has since
    /// become disabled or hidden reports none, as it would hear no key, and
    /// so does one outside an active focus trap, which hears no text.
    pub(crate) fn ime_area(&self) -> Option<Rect> {
        let focused = self.focused.filter(|&id| self.can_hold_focus(id))?;
        if self
            .active_focus_trap()
            .is_some_and(|trap| !self.is_within(focused, trap))
        {
            return None;
        }
        let node = self.nodes.get(focused)?;
        let shift = node.paint_shift;
        let widget: &dyn Widget = node.widget.as_ref();
        widget.ime_area().map(|area| {
            area.translated(Point {
                x: -shift.x,
                y: -shift.y,
            })
        })
    }

    /// Returns the next focusable node in tree order after `from`, or the
    /// previous one when `backwards` is set, wrapping at both ends.
    ///
    /// Without a focused node, forward navigation starts at the first
    /// focusable node and backward navigation at the last. A hidden node and
    /// its subtree are skipped, and a disabled node is skipped while its
    /// children are still visited, matching hit testing. While a focus trap
    /// is mounted, the order holds only the focusable nodes inside it, so a
    /// Tab press from outside the trap enters it and one inside it never
    /// leaves.
    fn next_focusable(&self, from: Option<WidgetId>, backwards: bool) -> Option<WidgetId> {
        let mut order = Vec::new();
        if let Some(scope) = self.active_focus_trap().or(self.root) {
            self.collect_tab_stops(scope, true, &mut order);
            for top in self.overlays_within(scope) {
                self.collect_tab_stops(top, true, &mut order);
            }
        }
        if order.is_empty() {
            return None;
        }
        match from.and_then(|id| order.iter().position(|&candidate| candidate == id)) {
            Some(index) if backwards => Some(order[(index + order.len() - 1) % order.len()]),
            Some(index) => Some(order[(index + 1) % order.len()]),
            None if backwards => order.last().copied(),
            None => order.first().copied(),
        }
    }

    /// Appends the focusable nodes of the subtree, in paint order.
    pub(super) fn collect_focusable(&self, id: WidgetId, order: &mut Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        let focusable = node.interaction.has(Interaction::FOCUSABLE)
            && !node.interaction.has(Interaction::DISABLED);
        if focusable {
            order.push(id);
        }
        for &child in &node.children {
            self.collect_focusable(child, order);
        }
    }

    /// Appends the nodes of the subtree that Tab stops at, in paint order.
    ///
    /// They are its focusable nodes, except that a container that navigates
    /// with the arrows, other than the subtree's top when `top` is set, is
    /// one stop: the node inside it that holds the focus, or else the one
    /// marked selected, or else its first. Tab so enters a row of tabs at
    /// the shown one and leaves it with the next press.
    pub(super) fn collect_tab_stops(&self, id: WidgetId, top: bool, order: &mut Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if !top && node.handlers.arrows.is_some() && !node.withdrawn() {
            let mut inside = Vec::new();
            self.collect_focusable(id, &mut inside);
            inside.retain(|&inner| inner != id);
            let selected = |inner: &WidgetId| {
                self.nodes
                    .get(*inner)
                    .and_then(|node| node.a11y_extra.as_ref())
                    .and_then(|extra| extra.selected)
                    == Some(true)
            };
            let stop = inside
                .iter()
                .find(|&&inner| Some(inner) == self.focused)
                .or_else(|| inside.iter().find(|inner| selected(inner)))
                .or_else(|| inside.first());
            if node.interaction.has(Interaction::FOCUSABLE)
                && !node.interaction.has(Interaction::DISABLED)
            {
                order.push(id);
            }
            order.extend(stop);
            return;
        }
        if node.withdrawn() {
            return;
        }
        if node.interaction.has(Interaction::FOCUSABLE)
            && !node.interaction.has(Interaction::DISABLED)
        {
            order.push(id);
        }
        for &child in &node.children {
            self.collect_tab_stops(child, false, order);
        }
    }

    /// Returns whether `id` may keep the focus: it is not disabled, and
    /// neither it nor any ancestor is hidden.
    ///
    /// A node only clipped out of view, for example scrolled away, keeps it.
    fn can_hold_focus(&self, id: WidgetId) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if node.interaction.has(Interaction::DISABLED) {
            return false;
        }
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

    /// Returns the position of each node in its parent's children, from the
    /// root's child down to the focused node, or `None` when nothing holds
    /// the focus.
    ///
    /// The focused root gives an empty path.
    pub(crate) fn focus_path(&self) -> Option<Vec<usize>> {
        let mut current = self.focused?;
        let mut path = Vec::new();
        while let Some(parent) = self.nodes.get(current)?.parent {
            let children = &self.nodes.get(parent)?.children;
            path.push(children.iter().position(|&child| child == current)?);
            current = parent;
        }
        path.reverse();
        Some(path)
    }

    /// Focuses the node that `path`, as [`WidgetTree::focus_path`] gives it,
    /// leads to from the root.
    ///
    /// The node takes it when it is focusable, can hold the focus and lies
    /// inside the active focus trap, if one is mounted. Then the autofocus
    /// requests of every node that is visible now are dropped, so the next
    /// layout pass does not move the focus away again. A request whose node
    /// is hidden stays queued, to resolve when the node shows. Returns
    /// whether the node took the focus.
    pub(crate) fn restore_focus(&mut self, path: &[usize]) -> bool {
        let Some(mut id) = self.root else {
            return false;
        };
        for &index in path {
            let Some(&child) = self.nodes.get(id).and_then(|node| node.children.get(index)) else {
                return false;
            };
            id = child;
        }
        if self
            .active_focus_trap()
            .is_some_and(|trap| !self.is_within(id, trap))
            || !self.request_focus(id)
        {
            return false;
        }
        let mut pending = std::mem::take(&mut self.pending_autofocus);
        pending.retain(|&request| self.hidden_in_tree(request));
        self.pending_autofocus = pending;
        true
    }

    /// Queues an autofocus request for the node `id`, to resolve once a
    /// layout pass has laid it out visible.
    ///
    /// A node's element calls this at mount. The request is resolved once,
    /// when [`WidgetTree::resolve_autofocus`] runs and finds the node
    /// visible, and a later frame never asks again.
    pub(crate) fn queue_autofocus(&mut self, id: WidgetId) {
        self.pending_autofocus.push(id);
    }

    /// Focuses the node that mounted with autofocus and has been laid out
    /// visible.
    ///
    /// A request whose node is hidden, or sits under a hidden ancestor,
    /// stays queued until a layout pass finds it visible. The other requests
    /// are walked in paint order, and the last one that can hold the focus
    /// wins, so when several nodes show in one frame the node painted last
    /// takes it. While a focus trap is mounted, only a node inside it can
    /// win, so a node that mounts elsewhere cannot pull the focus out of the
    /// trap. A visible request that cannot hold the focus, because its node
    /// is disabled, is skipped and an earlier node then wins; a skipped
    /// request is dropped and never asked again.
    pub(super) fn resolve_autofocus(&mut self) {
        if self.pending_autofocus.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending_autofocus);
        let mut ready = Vec::new();
        pending.retain(|&id| {
            let hidden = self.hidden_in_tree(id);
            if !hidden {
                ready.push(id);
            }
            hidden
        });
        self.pending_autofocus = pending;
        if ready.is_empty() {
            return;
        }
        let Some(root) = self.root else {
            return;
        };
        let trap = self.active_focus_trap();
        let mut order = Vec::new();
        self.collect_pending(root, &ready, &mut order);
        for top in self.overlays_within(root) {
            self.collect_pending(top, &ready, &mut order);
        }
        for id in order.into_iter().rev() {
            if trap.is_some_and(|trap| !self.is_within(id, trap)) {
                continue;
            }
            if self.request_focus(id) {
                return;
            }
        }
    }

    /// Returns `id`, or the parent of the outermost node from `id` up that
    /// is withdrawn, so an event that would start inside a hidden or fading
    /// subtree starts above it and no handler inside it hears the event.
    pub(super) fn outside_withdrawn(&self, id: Option<WidgetId>) -> Option<WidgetId> {
        let mut start = id;
        let mut next = id;
        while let Some(current) = next {
            let Some(node) = self.nodes.get(current) else {
                break;
            };
            next = node.parent;
            if node.withdrawn() {
                start = node.parent;
            }
        }
        start
    }

    /// Returns whether the live node `id` or one of its ancestors is hidden.
    ///
    /// A stale id counts as visible, so a caller that resolves it finds
    /// nothing there.
    fn hidden_in_tree(&self, id: WidgetId) -> bool {
        let mut next = Some(id);
        while let Some(current) = next {
            let Some(node) = self.nodes.get(current) else {
                return false;
            };
            if node.withdrawn() {
                return true;
            }
            next = node.parent;
        }
        false
    }

    /// Marks the mounted node `id` as a focus trap.
    ///
    /// While a trap is the last one mounted and not hidden, Tab movement is
    /// confined to its subtree, keys start there when the focus is outside
    /// it, and a press outside it does not move the focus out. Removing the
    /// node ends the trap, restores the one mounted before it, and returns
    /// the focus to the node that held it when the trap became active.
    /// Hiding the trap returns the focus the same way, and showing it again
    /// records the focus anew; see [`WidgetTree::resolve_focus_traps`].
    pub(crate) fn push_focus_trap(&mut self, id: WidgetId) {
        let active = self.can_hold_focus(id);
        self.focus_traps.push(FocusTrap {
            node: id,
            restore: if active { self.focused } else { None },
            active,
        });
    }

    /// Records the focus of every trap that became able to hold it since the
    /// last look, and gives it back for every trap that stopped being able
    /// to.
    ///
    /// A trap that shows, or whose disabled state or hidden ancestor no
    /// longer keeps it from the focus, records the node that holds the focus
    /// now. A trap that hides, or becomes disabled, returns the focus to the
    /// node it recorded, when the focus sits inside the trap or nowhere and
    /// that node can still take it. The layout pass calls this, because that
    /// is where a hidden state lands, before an autofocus request inside a
    /// trap that just showed moves the focus in.
    pub(super) fn resolve_focus_traps(&mut self) {
        for at in 0..self.focus_traps.len() {
            let trap = self.focus_traps[at];
            let active = self.can_hold_focus(trap.node);
            if active == trap.active {
                continue;
            }
            self.focus_traps[at].active = active;
            if active {
                self.focus_traps[at].restore = self.focused;
                continue;
            }
            let focus_inside = self
                .focused
                .is_none_or(|focused| self.is_within(focused, trap.node));
            if focus_inside && let Some(restore) = self.trap_return(trap.node, trap.restore) {
                self.request_focus(restore);
            }
        }
    }

    /// Returns the trap that keyboard movement is confined to, when one is
    /// active.
    ///
    /// The last mounted trap wins. A trap that cannot hold the focus,
    /// because it became disabled or hidden or sits under a hidden ancestor,
    /// yields to the one mounted before it; without one, movement ranges the
    /// whole tree, and the hidden trap's own subtree is out of reach either
    /// way.
    pub(super) fn active_focus_trap(&self) -> Option<WidgetId> {
        self.focus_traps
            .iter()
            .rev()
            .map(|trap| trap.node)
            .find(|&trap| self.can_hold_focus(trap))
    }

    /// Returns whether `id` is `ancestor` or one of its descendants.
    pub(super) fn is_within(&self, id: WidgetId, ancestor: WidgetId) -> bool {
        let mut next = Some(id);
        while let Some(current) = next {
            if current == ancestor {
                return true;
            }
            next = self.nodes.get(current).and_then(|node| node.parent);
        }
        false
    }

    /// Moves the focus among the focusable nodes inside `container` along
    /// `axis`, as `event` asks, and returns whether it moved.
    ///
    /// The arrows along the axis step to the next or the previous focusable
    /// node inside it, in paint order and wrapping at both ends, Home and
    /// End go to the first and the last, and a letter goes to the next one
    /// whose accessible name starts with it. With the focus outside the
    /// container, as on the node that opened a menu, the first step goes to
    /// the first node or the last. Disabled and hidden nodes are passed
    /// over, and so are the overlays the container's content opened.
    fn navigate_in(
        &mut self,
        container: WidgetId,
        axis: crate::style::Direction,
        event: &KeyEvent,
    ) -> bool {
        if !event.pressed {
            return false;
        }
        let focused = self.focused;
        let mut order = Vec::new();
        self.collect_focusable(container, &mut order);
        order.retain(|&id| id != container);
        if order.is_empty() {
            return false;
        }
        let at = order.iter().position(|&id| Some(id) == focused);
        let last = order.len() - 1;
        let step = |forward: bool| match at {
            Some(at) if forward => (at + 1) % order.len(),
            Some(at) => (at + last) % order.len(),
            None if forward => 0,
            None => last,
        };
        let vertical = axis == crate::style::Direction::Column;
        let target = match &event.key {
            Key::ArrowDown if vertical => step(true),
            Key::ArrowUp if vertical => step(false),
            Key::ArrowRight if !vertical => step(true),
            Key::ArrowLeft if !vertical => step(false),
            Key::Home => 0,
            Key::End => last,
            Key::Character(text)
                if !event.modifiers.control && !event.modifiers.alt && !event.modifiers.meta =>
            {
                let mut letters = text.chars();
                let (Some(letter), None) = (letters.next(), letters.next()) else {
                    return false;
                };
                let starts = |id: WidgetId| {
                    self.nodes
                        .get(id)
                        .and_then(|node| node.a11y_label.as_deref())
                        .and_then(|label| label.chars().next())
                        .is_some_and(|first| first.to_lowercase().eq(letter.to_lowercase()))
                };
                let from = at.map_or(0, |at| at + 1);
                let Some(found) = (0..order.len())
                    .map(|offset| (from + offset) % order.len())
                    .find(|&index| starts(order[index]))
                else {
                    return false;
                };
                found
            }
            _ => return false,
        };
        self.set_keyboard_nav(true);
        self.request_focus(order[target])
    }

    /// Returns where the focus goes when the trap at `node` ends: the node
    /// that opened the overlay the trap sits in, when it can take the focus,
    /// and otherwise `restore`, the node that held the focus when the trap
    /// began. An overlay opened from another one, as a menu of a menu bar
    /// opened while the one beside it closed, so goes back to its own
    /// opener.
    pub(super) fn trap_return(
        &self,
        node: WidgetId,
        restore: Option<WidgetId>,
    ) -> Option<WidgetId> {
        let mut next = Some(node);
        while let Some(id) = next {
            if self.is_overlay_root(id) {
                let host = self.nodes.get(id).and_then(|node| node.parent);
                if let Some(host) =
                    host.filter(|&host| self.focusable(host) && self.can_hold_focus(host))
                {
                    return Some(host);
                }
                break;
            }
            next = self.nodes.get(id).and_then(|node| node.parent);
        }
        restore
    }

    /// Returns the top nodes of the overlays that show and that a node inside
    /// `scope` opened, in the order they draw, which follow the scope's
    /// own nodes in the focus order.
    pub(super) fn overlays_within(&self, scope: WidgetId) -> Vec<WidgetId> {
        self.overlays
            .iter()
            .filter(|overlay| self.overlay_shown(overlay))
            .map(|overlay| overlay.root)
            .filter(|&top| top != scope && self.is_within(top, scope))
            .collect()
    }

    /// Appends the nodes of `pending` that the subtree holds, in paint order.
    ///
    /// A hidden node and its subtree are skipped, because a hidden node
    /// cannot hold the focus.
    fn collect_pending(&self, id: WidgetId, pending: &[WidgetId], order: &mut Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        if pending.contains(&id) {
            order.push(id);
        }
        for &child in &node.children {
            self.collect_pending(child, pending, order);
        }
    }

    /// Asks the next layout to scroll the focused node into view.
    ///
    /// The request waits for the layout because the rectangles it reads are
    /// only current once the frame has laid out the move that came with it,
    /// such as a node an autofocus request just showed. See
    /// [`WidgetTree::resolve_reveal`].
    fn scroll_focus_into_view(&mut self) {
        self.reveal_focus = true;
    }

    /// Scrolls the ancestors of the focused node that asked for it so the
    /// node is in view, when a focus move requested it, and then the
    /// ancestors of a node that asked to show a rectangle of itself through
    /// [`Element::reveal`](crate::element::Element::reveal) so that rectangle
    /// is in view.
    ///
    /// The walk goes up from the focused node. Every ancestor with an
    /// `on_reveal` handler that the node flows in, rather than sits in
    /// absolutely positioned, moves its offset on each axis by the least
    /// that brings the target inside its content box, or aligns the
    /// target's top or left with the box's when the target is taller or
    /// wider, clamped to the range its flowing content scrolls. The target
    /// starts as the focused node's rectangle, and after each such ancestor
    /// it becomes the part of the moved target that ancestor shows, so the
    /// ancestor around it reveals that. An ancestor whose offset changes hears its handler with the new
    /// offset, in its owner. An ancestor that does not move hears nothing.
    ///
    /// A focused node that cannot hold the focus, because it or an ancestor
    /// is hidden or it is disabled, scrolls nothing. The request stays until
    /// the layout pass clears it, so a settle pass in the same frame can
    /// apply it again to the geometry it settled.
    pub(super) fn resolve_reveal(&mut self) {
        if self.reveal_focus
            && let Some(focused) = self.focused.filter(|&id| self.can_hold_focus(id))
        {
            let target = self.nodes[focused].rect;
            self.reveal_rect(focused, target);
        }
        if let Some((id, rect)) = self.reveal_request
            && self.can_hold_focus(id)
        {
            let origin = self.nodes[id].content.origin();
            let target = rect.translated(origin);
            self.reveal_rect(id, target);
        }
    }

    /// Asks the tree to show `rect`, relative to the content box of `id`, in
    /// the scroll views around the node, as [`WidgetTree::resolve_reveal`]
    /// does for the focused node. A later request replaces it.
    pub(crate) fn request_reveal(&mut self, id: WidgetId, rect: Rect) {
        self.reveal_request = Some((id, rect));
    }

    /// Forgets the reveal request once the frame applied it.
    pub(super) fn clear_reveal_request(&mut self) {
        self.reveal_request = None;
    }

    /// Returns whether a node asked to show a rectangle that no frame has
    /// shown yet.
    pub(super) fn has_reveal_request(&self) -> bool {
        self.reveal_request.is_some()
    }

    /// Scrolls the ancestors of `from` that asked for it so `target`, in
    /// window coordinates, is in view, as [`WidgetTree::resolve_reveal`]
    /// describes.
    fn reveal_rect(&mut self, from: WidgetId, target: Rect) {
        let mut target = target;
        let mut child = from;
        // An overlay is not inside the views around its host, so they never
        // scroll to bring it into view.
        let mut next = self.pointer_parent(from);
        while let Some(id) = next {
            let node = &self.nodes[id];
            next = self.pointer_parent(id);
            let flows = self.nodes[child].resolved.layout.position != Position::Absolute;
            child = id;
            if !flows || node.handlers.on_reveal.is_none() {
                continue;
            }
            let view = node.content;
            let current = node.scroll_offset;
            let Some(max) = self.max_scroll_offset(id) else {
                continue;
            };
            // A non-finite offset or extent, which a binding can hand over,
            // leaves this ancestor where it is rather than panicking in the
            // clamps below.
            if ![
                view.x0, view.x1, view.y0, view.y1, current.x, current.y, max.x, max.y,
            ]
            .iter()
            .all(|value| value.is_finite())
            {
                continue;
            }
            let wanted = Point {
                x: (current.x + reveal_delta(target.x0, target.x1, view.x0, view.x1))
                    .clamp(0.0, max.x.max(0.0)),
                y: (current.y + reveal_delta(target.y0, target.y1, view.y0, view.y1))
                    .clamp(0.0, max.y.max(0.0)),
            };
            if wanted != current && wanted.x.is_finite() && wanted.y.is_finite() {
                self.set_scroll_offset(id, wanted);
                self.run_reveal(id, wanted);
                target = target.translated(Point {
                    x: current.x - wanted.x,
                    y: current.y - wanted.y,
                });
            }
            target = Rect::new(
                target.x0.max(view.x0).min(view.x1),
                target.y0.max(view.y0).min(view.y1),
                target.x1.max(view.x0).min(view.x1),
                target.y1.max(view.y0).min(view.y1),
            );
        }
    }

    /// Runs the `on_reveal` handler of `id` with `offset`, inside the node's
    /// owner.
    fn run_reveal(&mut self, id: WidgetId, offset: Point) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_reveal) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_reveal.as_mut())
            {
                on_reveal(offset);
            }
        });
    }
}

/// Returns how far a view whose visible span along one axis runs from
/// `view_start` to `view_end` scrolls forward to show a target spanning
/// `start` to `end` on that axis, all in window coordinates, negative to
/// scroll back.
///
/// A target that fits is brought in by the nearest edge, and one longer than
/// the view is aligned with its start, its top or its left. A target already
/// inside needs nothing.
fn reveal_delta(start: f32, end: f32, view_start: f32, view_end: f32) -> f32 {
    if start < view_start || end - start > view_end - view_start {
        start - view_start
    } else if end > view_end {
        end - view_end
    } else {
        0.0
    }
}
