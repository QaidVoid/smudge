//! The paint pass.
//!
//! The pass regenerates the cached operations of every node whose layout or
//! paint properties changed, and reports the pixels each one may have changed
//! as damage. Regenerating one node's operations touches no other node, so a
//! frame that changes a single box repaints that box and nothing else.
//!
//! Damage has to cover the area a node painted last frame as well as the area
//! it paints now. The first is where stale pixels have to be cleared, and the
//! second is where new pixels appear. A node that moved therefore damages both
//! of its rectangles, and a node that only changed color damages one.

use crate::damage::DamageAccumulator;
use crate::geometry::Rect;
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
    /// node's painted bounds are the union of its operations'
    /// bounds, kept together with the clip the layout pass gave it, and the two
    /// are intersected in physical pixels when the damage is finished. A child
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
        for id in self.paint_queue.drain(..) {
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
            if node.dirty == Dirty::Clean {
                continue;
            }

            let old = node.painted.take();
            if let Some(bounds) = old {
                damage.add_clipped(bounds.bounds, bounds.clip);
            }

            node.ops.clear();
            if !node.clip.is_empty() {
                self.theme.reset_consulted();
                let cx = PaintCx {
                    rect: node.rect,
                    style: &node.resolved,
                    theme: &self.theme,
                };
                node.widget.paint(&cx, &mut node.ops);
                node.uses_tokens = self.theme.consulted();
            } else {
                // A node that paints nothing shows no theme value.
                node.uses_tokens = false;
            }

            let bounds = node
                .ops
                .iter()
                .fold(Rect::default(), |bounds, op| bounds.union(op.bounds()));
            node.painted = if bounds.is_empty() {
                None
            } else {
                Some(PaintedBounds {
                    bounds,
                    clip: node.clip,
                })
            };
            if let Some(bounds) = node.painted
                && node.painted != old
            {
                damage.add_clipped(bounds.bounds, bounds.clip);
            }
            node.dirty = Dirty::Clean;
            painted += 1;
        }

        for bounds in self.pending_damage.drain(..) {
            damage.add_clipped(bounds.bounds, bounds.clip);
        }

        painted
    }
}

#[cfg(test)]
#[path = "paint_pass_tests.rs"]
mod tests;
