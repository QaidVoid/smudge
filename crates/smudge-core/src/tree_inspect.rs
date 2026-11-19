//! The tree's side of [`crate::inspect`]: walking every node, picking the
//! node under a point and copying one node's details.

use std::panic::Location;

use crate::geometry::{Point, Rect};
use crate::inspect::{BoxModel, NodeDetails, States, TreeEntry};

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
    /// their children and children in paint order.
    pub(crate) fn visit_tree(&self, visit: &mut dyn FnMut(TreeEntry)) {
        if let Some(root) = self.root {
            self.visit_entry(root, 0, false, visit);
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
        self.hit(self.root?, position, true)
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
                    border.x0 - layout.margin.left,
                    border.y0 - layout.margin.top,
                    border.x1 + layout.margin.right,
                    border.y1 + layout.margin.bottom,
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
            hidden,
        })
    }
}
