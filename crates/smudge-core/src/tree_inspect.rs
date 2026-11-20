//! The tree's side of [`crate::inspect`]: walking every node, picking the
//! node under a point and copying one node's details.

use std::panic::Location;

use crate::geometry::{Point, Rect};
use crate::inspect::{
    A11yWarning, A11yWarningKind, BoxModel, FocusInfo, NodeDetails, PointerInfo, States, TreeEntry,
};
use crate::semantics::Role;
use crate::style::bounded_or;

use super::interaction::Interaction;
use super::{WidgetId, WidgetTree};

impl WidgetTree {
    /// Records where the element of `id` was created. A stale id does
    /// nothing.
    pub(crate) fn set_created_at(&mut self, id: WidgetId, at: &'static Location<'static>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.created_at = Some(at);
        }
    }

    /// Calls `visit` with every node, hidden ones included, parents before
    /// their children and children in paint order, the overlays after the
    /// main tree.
    pub(crate) fn visit_tree(&self, visit: &mut dyn FnMut(TreeEntry)) {
        if let Some(root) = self.root {
            self.visit_entry(root, 0, false, visit);
        }
        for overlay in &self.overlays {
            let hidden = !self.overlay_shown(overlay);
            self.visit_entry(overlay.root, 0, hidden, visit);
        }
    }

    /// Visits `id` and its subtree for [`WidgetTree::visit_tree`], with
    /// `hidden` saying whether an ancestor is hidden.
    fn visit_entry(
        &self,
        id: WidgetId,
        depth: usize,
        hidden: bool,
        visit: &mut dyn FnMut(TreeEntry),
    ) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let hidden = hidden || node.resolved.layout.hidden;
        visit(TreeEntry {
            id,
            depth,
            type_name: node.widget.type_name(),
            rect: node.rect,
            hidden,
        });
        for &child in &node.children {
            self.visit_entry(child, depth + 1, hidden, visit);
        }
    }

    /// Returns the topmost node at `position`, disabled ones included, as
    /// input would find it.
    pub(crate) fn node_at(&self, position: Point) -> Option<WidgetId> {
        self.hit_any(position, true)
    }

    /// Returns a copy of what the inspector shows of `id`, or `None` for a
    /// stale id.
    pub(crate) fn node_details(&self, id: WidgetId) -> Option<NodeDetails> {
        let node = self.nodes.get(id)?;
        let mut hidden = false;
        let mut next = Some(id);
        while let Some(ancestor) = next.and_then(|id| self.nodes.get(id)) {
            hidden |= ancestor.resolved.layout.hidden;
            next = ancestor.parent;
        }
        let layout = self.taffy.layout(node.taffy).ok();
        let border = node.rect;
        let (margin, padding) = match layout {
            Some(layout) if !hidden => (
                Rect::new(
                    border.x0 - bounded_or(layout.margin.left, 0.0),
                    border.y0 - bounded_or(layout.margin.top, 0.0),
                    border.x1 + bounded_or(layout.margin.right, 0.0),
                    border.y1 + bounded_or(layout.margin.bottom, 0.0),
                ),
                Rect::new(
                    border.x0 + layout.border.left,
                    border.y0 + layout.border.top,
                    border.x1 - layout.border.right,
                    border.y1 - layout.border.bottom,
                ),
            ),
            _ => (border, border),
        };
        let painted = node.painted.map(|painted| {
            let back = Point {
                x: -painted.shift.x,
                y: -painted.shift.y,
            };
            let bounds = painted.bounds.translated(back);
            let inner = painted.inner.translated(back);
            match (bounds.is_empty(), inner.is_empty()) {
                (_, true) => bounds,
                (true, false) => inner,
                (false, false) => bounds.union(inner),
            }
        });
        Some(NodeDetails {
            id,
            type_name: node.widget.type_name(),
            created_at: node.created_at,
            base: node.base.clone(),
            resolved: node.resolved.clone(),
            states: States {
                hovered: node.interaction.has(Interaction::HOVERED),
                pressed: node.interaction.has(Interaction::PRESSED),
                focused: node.interaction.has(Interaction::FOCUSED),
                disabled: node.interaction.has(Interaction::DISABLED),
                invalid: node.interaction.has(Interaction::INVALID),
            },
            dirty: node.dirty,
            boxes: BoxModel {
                margin,
                border,
                padding,
                content: node.content,
            },
            painted,
            ops: node.ops.len() + node.inner_ops.len(),
            semantics: self.semantics_node(id),
            bindings: crate::reactive::runtime::bindings_of(&node.owner),
            hidden,
        })
    }

    /// Returns where the focus is and the order Tab visits nodes in, which
    /// is the order [`WidgetTree::next_focusable`] walks.
    pub(crate) fn focus_info(&self) -> FocusInfo {
        let trap = self.active_focus_trap();
        let mut order = Vec::new();
        if let Some(scope) = trap.or(self.root) {
            self.collect_tab_stops(scope, true, &mut order);
            for top in self.overlays_within(scope) {
                self.collect_tab_stops(top, true, &mut order);
            }
        }
        let rect = |id: WidgetId| self.nodes.get(id).map(|node| node.rect);
        FocusInfo {
            focused: self.focused(),
            keyboard: self.keyboard_nav(),
            order: order
                .into_iter()
                .filter_map(|id| Some((id, rect(id)?)))
                .collect(),
            trap: trap.and_then(rect),
        }
    }

    /// Returns the hovered chain, the held press and the pressable nodes.
    pub(crate) fn pointer_info(&self) -> PointerInfo {
        let mut info = PointerInfo::default();
        let mut next = self.hovered;
        while let Some(node) = next.and_then(|id| self.nodes.get(id)) {
            if !node.interaction.has(Interaction::DISABLED) {
                info.hovered.push(node.rect);
            }
            next = next.and_then(|id| self.pointer_parent(id));
        }
        info.pressed = self
            .pressed
            .and_then(|id| self.nodes.get(id))
            .map(|node| node.rect);
        if let Some(root) = self.root {
            self.collect_pressable(root, &mut info.pressable);
        }
        for overlay in &self.overlays {
            if self.overlay_shown(overlay) {
                self.collect_pressable(overlay.root, &mut info.pressable);
            }
        }
        info
    }

    /// Appends the rectangles of the visible, enabled nodes of the subtree
    /// that run a handler when pressed.
    fn collect_pressable(&self, id: WidgetId, out: &mut Vec<Rect>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        if node.handlers.takes_press() && !node.interaction.has(Interaction::DISABLED) {
            out.push(node.rect);
        }
        for &child in &node.children {
            self.collect_pressable(child, out);
        }
    }

    /// Returns the visible nodes that assistive technology cannot describe
    /// well: the projected ones in paint order, then the pressable nodes
    /// that are not projected, in paint order too.
    pub(crate) fn accessibility_warnings(&self) -> Vec<A11yWarning> {
        let mut warnings = Vec::new();
        for node in self.semantics_nodes() {
            let named = node
                .label
                .as_deref()
                .is_some_and(|label| !label.trim().is_empty())
                || !node.text_runs.is_empty();
            let kind = match node.role {
                Role::TextInput | Role::MultilineTextInput
                    if node
                        .label
                        .as_deref()
                        .is_none_or(|label| label.trim().is_empty()) =>
                {
                    Some(A11yWarningKind::TextInputWithoutName)
                }
                _ if node.focusable && !named && node.value.is_none() => {
                    Some(A11yWarningKind::FocusableWithoutName)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                warnings.push(A11yWarning {
                    id: node.id,
                    rect: node.bounds,
                    kind,
                });
            }
        }
        let mut pressable = Vec::new();
        if let Some(root) = self.root {
            self.collect_unprojected_pressable(root, &mut pressable);
        }
        for overlay in &self.overlays {
            if self.overlay_shown(overlay) {
                self.collect_unprojected_pressable(overlay.root, &mut pressable);
            }
        }
        warnings.extend(pressable.into_iter().map(|(id, rect)| A11yWarning {
            id,
            rect,
            kind: A11yWarningKind::PressableWithoutRole,
        }));
        warnings
    }

    /// Appends the visible, enabled nodes of the subtree that run a handler
    /// when pressed but are not projected, so they report no role.
    fn collect_unprojected_pressable(&self, id: WidgetId, out: &mut Vec<(WidgetId, Rect)>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        if node.handlers.takes_press()
            && !node.interaction.has(Interaction::DISABLED)
            && self.semantics_node(id).is_none()
        {
            out.push((id, node.rect));
        }
        for &child in &node.children {
            self.collect_unprojected_pressable(child, out);
        }
    }
}
