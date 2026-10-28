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
use super::{WidgetId, WidgetTree};

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
    /// root, until a handler reports [`Handled::Yes`]. A press of Tab or an
    /// arrow key turns the keyboard navigation mode on, whether or not a
    /// handler took the key, and an unhandled Tab or Shift+Tab moves the focus
    /// to the next or previous focusable node in tree order. Releases and
    /// every other key only route.
    ///
    /// A focused node that became disabled or hidden since it took the focus
    /// loses it first, and its own handler does not run: a disabled button
    /// must not activate from the keyboard. The key still bubbles from it to
    /// the ancestors that can hear it, so shortcuts above it keep working.
    pub(crate) fn key(&mut self, event: &KeyEvent) {
        // A node that lost the focus by becoming disabled or hidden still
        // anchors the next key, so shortcuts above it keep working. The key
        // starts at its parent: the node itself no longer holds the focus,
        // even if it has been enabled again since.
        let start = match self.focused {
            Some(focused) => Some(focused),
            None => self
                .stale_focus
                .take()
                .and_then(|anchor| self.nodes.get(anchor)?.parent),
        };
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

    /// Delivers the event to `start` and its ancestors, deepest first, and
    /// returns whether a handler took it.
    ///
    /// `start` is the node that held the focus when the key arrived. Without
    /// one the event reaches no handler.
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
    /// event keeps bubbling past it, like a key does.
    fn route_from_focus(&mut self, mut deliver: impl FnMut(&mut Self, WidgetId) -> bool) {
        let start = match self.focused {
            Some(focused) => Some(focused),
            None => self
                .stale_focus
                .take()
                .and_then(|anchor| self.nodes.get(anchor)?.parent),
        };
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
    fn move_focus(&mut self, backwards: bool) {
        let next = self.next_focusable(self.focused, backwards);
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
    /// become disabled or hidden reports none, as it would hear no key.
    pub(crate) fn ime_area(&self) -> Option<Rect> {
        let focused = self.focused.filter(|&id| self.can_hold_focus(id))?;
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
    /// children are still visited, matching hit testing.
    fn next_focusable(&self, from: Option<WidgetId>, backwards: bool) -> Option<WidgetId> {
        let mut order = Vec::new();
        if let Some(root) = self.root {
            self.collect_focusable(root, &mut order);
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

    /// Scrolls the nearest scrolling ancestor so the focused node is in view.
    ///
    /// Nothing scrolls yet: scroll offsets arrive with the scroll view, and
    /// focus movement calls this hook so the behavior has one home.
    fn scroll_focus_into_view(&mut self) {}
}
