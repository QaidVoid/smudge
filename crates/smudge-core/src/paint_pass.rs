//! The paint pass.
//!
//! The pass regenerates the cached operations of every node whose layout or
//! paint properties changed, and reports the pixels each one may have changed
//! as damage. Regenerating one node's operations touches no other node, so a
//! frame that changes a single box repaints that box and nothing else.
//!
//! Widgets paint in paint coordinates, which leave out the scroll offsets of
//! the enclosing scroll views. A scrolled subtree keeps its operations while
//! it scrolls, and the display list pass wraps the moving children in an
//! offset scope so renderers land them in the window. Damage, which renderers
//! consume in window coordinates, translates the painted bounds back with the
//! shift each bounds record carries.
//!
//! Damage has to cover the area a node painted last frame as well as the area
//! it paints now. The first is where stale pixels have to be cleared, and the
//! second is where new pixels appear. A node that moved therefore damages both
//! of its rectangles, and a node that only changed color damages one.

use crate::damage::DamageAccumulator;
use crate::geometry::Rect;
use crate::paint::PaintOp;
use crate::widget::PaintCx;

use super::{Dirty, PaintedBounds, WidgetTree};

impl WidgetTree {
    /// Paints every queued node and adds the pixels each one may have changed
    /// to `damage`.
    ///
    /// A node is painted when it is still live and its flag is not
    /// [`Dirty::Clean`], which means its layout or its paint properties
    /// changed. The pass adds the bounds the node painted last frame to
    /// `damage`, regenerates the node's operations, and adds the bounds it
    /// paints now unless they equal the old ones, which already cover them. A
    /// node's painted bounds are the union of its operations' bounds in the
    /// node's paint coordinates, kept together with the clip the layout pass
    /// gave it and the shift the bounds are relative to, and the two are
    /// intersected in physical pixels when the damage is finished. The
    /// operations of [`Widget::paint_inside`](crate::widget::Widget::paint_inside)
    /// keep a union of their own, which is also intersected with the node's
    /// rectangle inside its border, again in physical pixels. A child
    /// that overflows a clipping parent therefore damages only the part inside
    /// the clip, including the pixels the clip edge passes through. A node
    /// whose clip is empty, such as a hidden node, keeps no operations and its
    /// widget is not asked to paint.
    ///
    /// Before a widget paints, the pass clears the theme's consultation flag,
    /// and after it paints, the node's `uses_tokens` flag records whether the
    /// widget resolved a token. That is how a theme switch finds the nodes
    /// that have to repaint, including widgets that resolve tokens inside
    /// their own paint.
    ///
    /// Every painted node ends [`Dirty::Clean`]. The bounds of nodes removed
    /// since the last frame, and of the subtrees a reorder restacked, are
    /// added to `damage` as well.
    ///
    /// The pass reuses the queue and every node's operation buffer, so it
    /// allocates nothing once those buffers have grown to the size they need.
    ///
    /// Returns the number of nodes painted.
    pub(crate) fn paint(&mut self, damage: &mut DamageAccumulator) -> u32 {
        let mut painted = 0u32;
        // The queue is taken out while it is walked, because finding a
        // node's rectangle inside its border reads the tree, and it is put
        // back cleared so its capacity is reused.
        let mut queue = std::mem::take(&mut self.paint_queue);
        for &id in &queue {
            if !self.nodes.contains_key(id) {
                continue;
            }
            let inside = self.inside_border(id);
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
            if node.dirty == Dirty::Clean {
                continue;
            }

            let old = node.painted.take();
            if let Some(bounds) = old {
                bounds.damage(damage);
            }

            node.ops.clear();
            node.inner_ops.clear();
            if !node.clip.is_empty() {
                self.theme.reset_consulted();
                let cx = PaintCx {
                    rect: node.paint_rect,
                    content: node.paint_content,
                    style: &node.resolved,
                    theme: &self.theme,
                };
                node.widget.paint(&cx, &mut node.ops);
                node.widget.paint_inside(&cx, &mut node.inner_ops);
                node.uses_tokens = self.theme.consulted();
            } else {
                // A node that paints nothing shows no theme value.
                node.uses_tokens = false;
            }

            let union = |ops: &[PaintOp]| {
                ops.iter()
                    .fold(Rect::default(), |bounds, op| bounds.union(op.bounds()))
            };
            let bounds = union(&node.ops);
            let inner = union(&node.inner_ops);
            node.painted = if bounds.is_empty() && inner.is_empty() {
                None
            } else {
                Some(PaintedBounds {
                    bounds,
                    inner,
                    inside,
                    clip: node.clip,
                    shift: node.paint_shift,
                })
            };
            if let Some(bounds) = node.painted
                && node.painted != old
            {
                bounds.damage(damage);
            }
            node.dirty = Dirty::Clean;
            painted += 1;
        }
        queue.clear();
        self.paint_queue = queue;

        for bounds in self.pending_damage.drain(..) {
            bounds.damage(damage);
        }

        painted
    }
}

#[cfg(test)]
#[path = "paint_pass_tests.rs"]
mod tests;
