//! The display list pass.
//!
//! The pass flattens the tree into one list of drawing items in paint order,
//! so a renderer replays one buffer instead of walking the tree. Every node
//! stores its operations in its own paint coordinates, so the structure the
//! list needs is the clip that a clipping node puts around its children and
//! the offset scope that a node with a scroll offset puts around the children
//! its scroll moves: a renderer replays the items translated by the open
//! offsets, which lands the operations back in the window without the tree
//! repainting anything that only scrolled.

use crate::geometry::Point;
use crate::paint::DisplayList;
use crate::style::{Overflow, Position};

use super::{WidgetId, WidgetTree};

impl WidgetTree {
    /// Rebuilds `list` from the tree in paint order.
    ///
    /// A node contributes its cached operations, then its children in order.
    /// The children of a node that clips are wrapped in a
    /// [`PushClip`](crate::paint::DisplayItem::PushClip) carrying the area
    /// inside the node's border, in the node's paint coordinates, and a
    /// matching [`PopClip`](crate::paint::DisplayItem::PopClip), so a child
    /// that overflows its parent is cut off by the renderer before it reaches
    /// the border. The children a node's scroll offset moves are wrapped in a
    /// [`PushOffset`](crate::paint::DisplayItem::PushOffset) scope of minus
    /// the offset, so a renderer draws them moved by the scroll without the
    /// operations being repainted. A hidden node and its subtree contribute
    /// nothing.
    ///
    /// The list is cleared first, which keeps its allocated capacity, and
    /// [`DisplayList::finish`] runs at the end to check that the clips and
    /// offsets balance.
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
            list.push_clip(self.inside_border(id));
        }
        // The runs of children the scroll offset moves share one offset
        // scope, and an absolutely positioned child between them is emitted
        // outside it, because the scroll does not move it.
        let scroll = node.scroll_offset;
        let offset = Point {
            x: -scroll.x,
            y: -scroll.y,
        };
        let mut offset_open = false;
        for &child in &node.children {
            let Some(child_style) = self.style(child) else {
                continue;
            };
            if child_style.layout.hidden {
                continue;
            }
            let flowing = child_style.layout.position == Position::Relative;
            if offset != Point::default() {
                if flowing && !offset_open {
                    list.push_offset(offset);
                    offset_open = true;
                } else if !flowing && offset_open {
                    list.pop_offset();
                    offset_open = false;
                }
            }
            self.emit(child, list);
        }
        if offset_open {
            list.pop_offset();
        }
        if clips {
            list.pop_clip();
        }
    }
}
