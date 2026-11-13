//! Keyboard focus: which node holds it, how it moves, and how keys route.
//!
//! The focused node hears key events before they bubble to its ancestors.
//! Focus moves by a press on a focusable node or by Tab navigation, and the
//! keyboard navigation mode records which of the two moved it last, so a
//! focus ring can draw for keyboard users alone. A pointer press turns the
//! mode off again.

use crate::geometry::{Point, Rect};
use crate::input::{Handled, Key, KeyEvent};
use crate::widget::Widget;

use super::interaction::Interaction;
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
        // A node that lost the focus by becoming disabled or hidden still
        // anchors the next key, so shortcuts above it keep working. The key
        // starts at its parent: the node itself no longer holds the focus,
        // even if it has been enabled again since. With no anchor either,
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
        // An active trap holds the keyboard: a key that would start outside
        // it starts at the trap, so the trap's own handlers hear it before
        // the shortcuts above it, and a control behind it never does.
        let start = self.confine_to_trap(start);
        self.drop_invalid_focus();
        let handled = self.route_key(start, event);
        if !event.pressed {
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
        while let Some(id) = next {
            next = self.nodes.get(id).and_then(|node| node.parent);
            // A disabled or hidden node's handlers never run, but the key keeps
            // bubbling, so shortcuts above it still work.
            if self.nodes.get(id).is_some_and(|node| {
                node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
            }) {
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
        }
        Handled::No
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
        let start = self.confine_to_trap(start);
        self.drop_invalid_focus();
        let mut next = start;
        while let Some(id) = next {
            next = self.nodes.get(id).and_then(|node| node.parent);
            if self.nodes.get(id).is_some_and(|node| {
                node.interaction.has(Interaction::DISABLED) || node.resolved.layout.hidden
            }) {
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
    /// focus stays where it is.
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
    /// focus left behind, because the focus has moved on purpose.
    pub(super) fn set_focus(&mut self, id: Option<WidgetId>) {
        if self.focused == id {
            return;
        }
        let old = self.focused;
        self.stale_focus = None;
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
    }

    /// Focuses `id` on behalf of an accessibility action, and returns whether
    /// it took the focus.
    ///
    /// A node that is not focusable, or that cannot hold the focus because it
    /// is disabled or hidden, is left alone and `false` is returned. A node
    /// that already holds the focus keeps it.
    pub(crate) fn request_focus(&mut self, id: WidgetId) -> bool {
        if !self.focusable(id) || !self.can_hold_focus(id) {
            return false;
        }
        self.set_focus(Some(id));
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
            self.collect_focusable(scope, &mut order);
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
    fn collect_focusable(&self, id: WidgetId, order: &mut Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.resolved.layout.hidden {
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
            if node.resolved.layout.hidden {
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
    /// is hidden stays queued, to resolve when the node shows.
    pub(crate) fn restore_focus(&mut self, path: &[usize]) {
        let Some(mut id) = self.root else {
            return;
        };
        for &index in path {
            let Some(&child) = self.nodes.get(id).and_then(|node| node.children.get(index)) else {
                return;
            };
            id = child;
        }
        if self
            .active_focus_trap()
            .is_some_and(|trap| !self.is_within(id, trap))
            || !self.request_focus(id)
        {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending_autofocus);
        pending.retain(|&request| self.hidden_in_tree(request));
        self.pending_autofocus = pending;
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
        for id in order.into_iter().rev() {
            if trap.is_some_and(|trap| !self.is_within(id, trap)) {
                continue;
            }
            if self.request_focus(id) {
                self.scroll_focus_into_view();
                return;
            }
        }
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
            if node.resolved.layout.hidden {
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
            if focus_inside && let Some(restore) = trap.restore {
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

    /// Appends the nodes of `pending` that the subtree holds, in paint order.
    ///
    /// A hidden node and its subtree are skipped, because a hidden node
    /// cannot hold the focus.
    fn collect_pending(&self, id: WidgetId, pending: &[WidgetId], order: &mut Vec<WidgetId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.resolved.layout.hidden {
            return;
        }
        if pending.contains(&id) {
            order.push(id);
        }
        for &child in &node.children {
            self.collect_pending(child, pending, order);
        }
    }

    /// Scrolls the nearest scrolling ancestor so the focused node is in view.
    ///
    /// Nothing scrolls yet: scroll offsets arrive with the scroll view, and
    /// focus movement calls this hook so the behavior has one home.
    fn scroll_focus_into_view(&mut self) {}
}
