use std::collections::HashMap;
use std::rc::Rc;

use crate::element::{KeptContent, KeptPlacement};
use crate::reactive::Owner;

use super::{WidgetId, WidgetTree};

struct KeptPanel {
    store: Rc<KeptContent>,
    key: Rc<str>,
    owner: Owner,
}

#[derive(Default)]
pub(super) struct Retained {
    stores: HashMap<*const KeptContent, HashMap<Rc<str>, WidgetId>>,
    roots: HashMap<WidgetId, KeptPanel>,
    placements: HashMap<WidgetId, KeptPlacement>,
    pending_placements: Vec<WidgetId>,
    parked: Vec<WidgetId>,
    pub(super) focus_before: Option<Vec<WidgetId>>,
    pub(super) hover_before: Option<Vec<WidgetId>>,
    pub(super) moved: bool,
    drag_moved: bool,
    pub(super) applying_updates: bool,
    check_focus: bool,
    check_hover: bool,
    check_press: bool,
}

impl WidgetTree {
    pub(crate) fn root_owner(&self) -> Option<&Owner> {
        self.owner(self.root?)
    }

    pub(crate) fn kept_child(&self, store: &Rc<KeptContent>, key: &str) -> Option<WidgetId> {
        self.retained
            .stores
            .get(&Rc::as_ptr(store))?
            .get(key)
            .copied()
    }

    pub(crate) fn keep_child(
        &mut self,
        store: Rc<KeptContent>,
        key: Rc<str>,
        id: WidgetId,
        owner: Owner,
    ) {
        self.retained
            .stores
            .entry(Rc::as_ptr(&store))
            .or_default()
            .insert(Rc::clone(&key), id);
        self.retained
            .roots
            .insert(id, KeptPanel { store, key, owner });
    }

    pub(super) fn kept_root(&self, id: WidgetId) -> bool {
        self.retained.roots.contains_key(&id)
    }

    pub(crate) fn keep_placement(&mut self, id: WidgetId, placement: KeptPlacement, pending: bool) {
        self.retained.placements.insert(id, placement);
        if pending {
            if !self.retained.pending_placements.contains(&id) {
                self.retained.pending_placements.push(id);
            }
        } else {
            self.retained.pending_placements.retain(|&slot| slot != id);
        }
    }

    pub(crate) fn kept_placement_can_claim(&self, id: WidgetId) -> bool {
        let mut current = id;
        loop {
            let Some(node) = self.nodes.get(current) else {
                return false;
            };
            if node.exit.is_some() {
                return false;
            }
            let Some(parent) = node.parent else {
                return self.root == Some(current);
            };
            current = parent;
        }
    }

    pub(super) fn resume_kept_placements(&self, id: WidgetId) {
        if self.retained.placements.is_empty() {
            return;
        }
        let mut stack: Vec<_> = self
            .overlays
            .iter()
            .rev()
            .filter_map(|overlay| {
                (overlay.root != id
                    && self.is_within(overlay.root, id)
                    && self.kept_placement_can_claim(overlay.root))
                .then_some(overlay.root)
            })
            .collect();
        stack.push(id);
        while let Some(current) = stack.pop() {
            let node = &self.nodes[current];
            if node.exit.is_some() {
                continue;
            }
            if let Some(placement) = self.retained.placements.get(&current) {
                placement.place(current);
            }
            stack.extend(node.children.iter().rev().copied());
        }
    }

    pub(super) fn retry_pending_kept_placements(&self, parent: WidgetId) {
        for &slot in &self.retained.pending_placements {
            if self.is_within(slot, parent)
                && self.kept_placement_can_claim(slot)
                && let Some(placement) = self.retained.placements.get(&slot)
            {
                placement.place(slot);
            }
        }
    }

    pub(super) fn forget_kept_node(&mut self, id: WidgetId) {
        if self.retained.placements.remove(&id).is_some() {
            self.retained.pending_placements.retain(|&slot| slot != id);
        }
        if let Some(panel) = self.retained.roots.remove(&id) {
            let store = Rc::as_ptr(&panel.store);
            if let Some(keys) = self.retained.stores.get_mut(&store) {
                keys.remove(&panel.key);
                if keys.is_empty() {
                    self.retained.stores.remove(&store);
                }
            }
            panel.owner.cleanup();
        }
    }

    pub(super) fn park_kept_child(&mut self, id: WidgetId) {
        self.remember_move(id);
        self.detach_moved_child(id);
        self.retained.parked.push(id);
    }

    pub(super) fn remember_move(&mut self, id: WidgetId) {
        self.retained.moved = true;
        if self.retained.focus_before.is_none()
            && self.focused.is_some_and(|focus| self.is_within(focus, id))
        {
            self.retained.focus_before = Some(self.lineage(self.focused));
        }
        if self.retained.hover_before.is_none()
            && self.hovered.is_some_and(|hover| self.is_within(hover, id))
        {
            self.retained.hover_before = Some(self.chain(self.hovered));
        }
        self.retained.drag_moved |= self
            .drag
            .as_ref()
            .and_then(|drag| drag.observer_origin)
            .is_some_and(|origin| self.is_within(origin, id));
    }

    fn is_kept(&self, id: WidgetId) -> bool {
        if self.retained.roots.is_empty() {
            return false;
        }
        let mut next = Some(id);
        while let Some(current) = next {
            if self.kept_root(current) {
                return true;
            }
            next = self.nodes.get(current).and_then(|node| node.parent);
        }
        false
    }

    pub(crate) fn set_applying_updates(&mut self, applying: bool) {
        self.retained.applying_updates = applying;
    }

    pub(super) fn drop_focus_after_kept_moves(&mut self) {
        if self.retained.applying_updates
            && self
                .focused
                .is_some_and(|focus| self.is_kept(focus) && !self.can_hold_focus(focus))
        {
            self.retained.check_focus = true;
        } else {
            self.drop_invalid_focus();
        }
    }

    pub(super) fn defer_kept_press_release(&mut self) -> bool {
        let kept = self.pressed.is_some_and(|pressed| self.is_kept(pressed));
        self.retained.check_press |= kept;
        kept
    }

    pub(super) fn defer_kept_hover(&mut self, subtree: WidgetId) -> bool {
        let kept = self
            .hovered
            .is_some_and(|hover| self.is_kept(hover) && self.is_within(hover, subtree));
        self.retained.check_hover |= kept;
        kept
    }

    pub(crate) fn finish_kept_moves(&mut self) {
        let check_focus =
            std::mem::take(&mut self.retained.check_focus) || self.retained.focus_before.is_some();
        if self.retained.focus_before.is_some() {
            self.run_focus_within(self.focused, self.focused, &[]);
        }
        if check_focus {
            self.drop_invalid_focus();
        }
        if self.retained.hover_before.is_some() && self.holds_pointer() {
            self.set_hover_target(self.hovered);
        }
        if std::mem::take(&mut self.retained.drag_moved) {
            self.finish_drag_move();
        }
        if std::mem::take(&mut self.retained.check_press)
            && self.outside_withdrawn(self.pressed) != self.pressed
        {
            self.release_press();
        }
        if std::mem::take(&mut self.retained.check_hover)
            && self.retained.hover_before.is_none()
            && self.pointer_inside
            && !self.holds_pointer()
            && self.outside_withdrawn(self.hovered) != self.hovered
        {
            self.refresh_hover();
        }
        let mut parked = std::mem::take(&mut self.retained.parked);
        for id in parked.drain(..) {
            if self.nodes.get(id).is_some_and(|node| node.parent.is_none()) {
                if self.hovered.is_some_and(|hover| self.is_within(hover, id)) {
                    self.set_hover_target(None);
                }
                self.remove_subtree(id, false);
            }
        }
        self.retained.parked = parked;
    }
}
