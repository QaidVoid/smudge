//! The display list pass.
//!
//! The pass flattens the tree into one list of drawing operations in paint
//! order, so a renderer replays one buffer instead of walking the tree. Every
//! node stores its operations in absolute coordinates already, so the only
//! structure the list needs is the clip that a clipping node puts around its
//! children.

use crate::paint::DisplayList;
use crate::style::Overflow;

use super::{WidgetId, WidgetTree};

impl WidgetTree {
    /// Rebuilds `list` from the tree in paint order.
    ///
    /// A node contributes its cached operations, then its children in order.
    /// The children of a node that clips are wrapped in a
    /// [`PushClip`](crate::paint::DisplayItem::PushClip) carrying the node's
    /// rectangle and a matching [`PopClip`](crate::paint::DisplayItem::PopClip),
    /// so a child that overflows its parent is cut off by the renderer. A
    /// hidden node and its subtree contribute nothing.
    ///
    /// The list is cleared first, which keeps its allocated capacity, and
    /// [`DisplayList::finish`] runs at the end to check that the clips balance.
    pub(crate) fn build_display_list(&self, list: &mut DisplayList) {
        list.clear();
        if let Some(root) = self.root {
            self.emit(root, list);
        }
        list.finish();
    }

    /// Appends `id` and its subtree to `list` in paint order.
    fn emit(&self, id: WidgetId, list: &mut DisplayList) {
        let Some(style) = self.style(id) else {
            return;
        };
        if style.layout.hidden {
            return;
        }
        let clips = style.layout.overflow == Overflow::Clip;
        let node = self.nodes.get(id).expect("the id is a live node");

        list.extend_ops(&node.ops);
        if clips {
            list.push_clip(node.rect);
        }
        for &child in &node.children {
            self.emit(child, list);
        }
        if clips {
            list.pop_clip();
        }
    }
}
