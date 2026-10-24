//! The layout pass.
//!
//! The pass runs the layout engine only when a node needs it, then walks the
//! tree once to turn the engine's parent-relative rectangles into absolute
//! rectangles and clips. Logical coordinates are rounded to whole logical
//! pixels, which is the engine's default, so a layout rectangle never carries a
//! fraction of a pixel.

use taffy::{AvailableSpace as TaffySpace, LayoutInput, LayoutOutput, Size as TaffySize};

use crate::geometry::{Point, Rect, Size};
use crate::style::{Length, Overflow};
use crate::widget::{AvailableSpace, MeasureInput};

use super::{Dirty, WidgetId, WidgetTree};

/// Converts the engine's space variant to the type a widget's measure sees.
fn from_engine(space: TaffySpace) -> AvailableSpace {
    match space {
        TaffySpace::Definite(size) => AvailableSpace::Definite(size),
        TaffySpace::MinContent => AvailableSpace::MinContent,
        TaffySpace::MaxContent => AvailableSpace::MaxContent,
    }
}

impl WidgetTree {
    /// Lays the tree out for `viewport` and returns whether the layout engine
    /// ran.
    ///
    /// The engine runs only when a node is marked [`Dirty::Layout`] or when the
    /// viewport differs from the previous pass, so a paint-only change costs
    /// nothing here. The root is sized to the viewport, which overrides any
    /// size set on the root element.
    ///
    /// Each node ends the pass with an absolute rectangle and clip. A node
    /// whose rectangle or clip changed is marked [`Dirty::Paint`], because it
    /// has to paint at its new position. A hidden node and its subtree get an
    /// empty rectangle and an empty clip, so they paint nothing. Hiding a node
    /// therefore changes its clip, which marks it so the paint pass clears the
    /// pixels it painted last frame. Every [`Dirty::Layout`] flag drops to
    /// [`Dirty::Paint`], because a laid out node still has to be painted.
    ///
    /// The root's clip is the viewport. Every other node is clipped to its
    /// parent's clip, intersected with the parent's rectangle when the parent
    /// clips its children.
    ///
    /// # Panics
    ///
    /// Panics when a node has no live layout node, which is an internal
    /// invariant: every node in the tree owns exactly one layout node.
    pub(crate) fn layout(&mut self, viewport: Size) -> bool {
        let Some(rects_changed) = self.layout_pass(viewport) else {
            return false;
        };
        // A node that the pass moved under a pointer that stays still keeps a
        // stale hover, because no pointer event announces the move. The chain
        // is rebuilt from the last pointer position, unless a held press keeps
        // the hover on its capture.
        if rects_changed && self.pointer_inside && self.pressed.is_none() {
            let target = self.hover_target(self.pointer);
            self.set_hover_target(target);
            // A hover style can change the layout, and nothing would ask for
            // another frame to apply it. One more pass settles it in this
            // frame. Hover is not recomputed after it, so a style that moves
            // the node off the pointer cannot loop. Such a hover stays stale
            // until the pointer moves or a later layout pass changes rects.
            if self.layout_dirty {
                self.layout_pass(viewport);
            }
        }
        true
    }

    /// Runs the layout engine once when anything needs it, and returns whether
    /// any rectangle or clip changed, or `None` when no pass ran.
    fn layout_pass(&mut self, viewport: Size) -> Option<bool> {
        let viewport_changed = self.viewport != Some(viewport);
        if !self.layout_dirty && !viewport_changed {
            return None;
        }
        self.viewport = Some(viewport);

        let Some(root) = self.root else {
            self.layout_dirty = false;
            return None;
        };

        self.apply_layout(root, |layout| {
            layout.width = Length::px(viewport.width);
            layout.height = Length::px(viewport.height);
        });

        let available = TaffySize {
            width: TaffySpace::Definite(viewport.width),
            height: TaffySpace::Definite(viewport.height),
        };
        let WidgetTree { nodes, taffy, .. } = self;
        let root_node = nodes[root].taffy;
        taffy
            .compute_layout_with_measure(
                root_node,
                available,
                |inputs: LayoutInput,
                 _node: taffy::NodeId,
                 context: Option<&mut WidgetId>,
                 style: &taffy::Style|
                 -> LayoutOutput {
                    let node = context.and_then(|id| nodes.get_mut(*id));
                    // The leaf algorithm resolves the node's own size, limits,
                    // padding and border before it measures, so the widget is
                    // offered the content box. Its result is used only for an
                    // axis the style leaves open, but the engine calls it on
                    // the final pass regardless.
                    taffy::compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known, available| {
                            let measured = node.and_then(|node| {
                                node.widget.measure(MeasureInput {
                                    known_width: known.width,
                                    known_height: known.height,
                                    available_width: from_engine(available.width),
                                    available_height: from_engine(available.height),
                                })
                            });
                            measured
                                .map(|size| TaffySize {
                                    width: size.width,
                                    height: size.height,
                                })
                                .unwrap_or(TaffySize::ZERO)
                        },
                    )
                },
            )
            .expect("the root is a live layout node");

        self.layout_runs += 1;

        let clip = Rect::new(0.0, 0.0, viewport.width, viewport.height);
        let rects_changed = self.update_subtree(root, Point::default(), clip, false);

        self.layout_dirty = false;
        for node in self.nodes.values_mut() {
            if node.dirty == Dirty::Layout {
                node.dirty = Dirty::Paint;
            }
        }
        Some(rects_changed)
    }

    /// Writes the absolute rectangle and clip of `id` and its subtree, given
    /// the subtree's origin and inherited clip, and returns whether any node
    /// in the subtree changed either one.
    ///
    /// `hidden` reports whether an ancestor is hidden, which makes this node
    /// and everything below it empty.
    fn update_subtree(&mut self, id: WidgetId, origin: Point, clip: Rect, hidden: bool) -> bool {
        let node = self.nodes.get(id).expect("the id is a live node");
        let taffy_node = node.taffy;
        let hidden = hidden || node.resolved.layout.hidden;
        let overflow = node.resolved.layout.overflow;

        let layout = *self
            .taffy
            .layout(taffy_node)
            .expect("every node has a live layout node");
        let clip = if hidden { Rect::default() } else { clip };
        let rect = if hidden {
            Rect::default()
        } else {
            Rect::new(
                origin.x + layout.location.x,
                origin.y + layout.location.y,
                origin.x + layout.location.x + layout.size.width,
                origin.y + layout.location.y + layout.size.height,
            )
        };

        let node = self.nodes.get_mut(id).expect("the id is a live node");
        let changed = node.rect != rect || node.clip != clip;
        node.rect = rect;
        node.clip = clip;

        if changed {
            self.mark(id, Dirty::Paint);
        }

        let child_clip = if overflow == Overflow::Clip {
            clip.intersect(rect)
        } else {
            clip
        };
        let child_origin = Point {
            x: rect.x0,
            y: rect.y0,
        };

        let mut subtree_changed = changed;
        let mut index = 0;
        while let Some(child) = self
            .nodes
            .get(id)
            .and_then(|node| node.children.get(index))
            .copied()
        {
            subtree_changed |= self.update_subtree(child, child_origin, child_clip, hidden);
            index += 1;
        }
        subtree_changed
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
