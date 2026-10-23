//! The layout pass.
//!
//! The pass runs the layout engine only when a node needs it, then walks the
//! tree once to turn the engine's parent-relative rectangles into absolute
//! rectangles and clips. Logical coordinates are rounded to whole logical
//! pixels, which is the engine's default, so a layout rectangle never carries a
//! fraction of a pixel.

use taffy::{AvailableSpace, LayoutInput, LayoutOutput, Size as TaffySize};

use crate::geometry::{Point, Rect, Size};
use crate::style::{Length, Overflow};

use super::{Dirty, WidgetId, WidgetTree};

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
    #[cfg_attr(not(test), expect(dead_code, reason = "the frame calls it"))]
    pub(crate) fn layout(&mut self, viewport: Size) -> bool {
        let viewport_changed = self.viewport != Some(viewport);
        if !self.layout_dirty && !viewport_changed {
            return false;
        }
        self.viewport = Some(viewport);

        let Some(root) = self.root else {
            self.layout_dirty = false;
            return false;
        };

        self.apply_layout(root, |layout| {
            layout.width = Length::px(viewport.width);
            layout.height = Length::px(viewport.height);
        });

        let available = TaffySize {
            width: AvailableSpace::Definite(viewport.width),
            height: AvailableSpace::Definite(viewport.height),
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
                            node.and_then(|node| node.widget.measure(known, available))
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
        self.update_subtree(root, Point::default(), clip, false);

        self.layout_dirty = false;
        for node in self.nodes.values_mut() {
            if node.dirty == Dirty::Layout {
                node.dirty = Dirty::Paint;
            }
        }
        true
    }

    /// Writes the absolute rectangle and clip of `id` and its subtree, given
    /// the subtree's origin and inherited clip.
    ///
    /// `hidden` reports whether an ancestor is hidden, which makes this node
    /// and everything below it empty.
    fn update_subtree(&mut self, id: WidgetId, origin: Point, clip: Rect, hidden: bool) {
        let node = self.nodes.get(id).expect("the id is a live node");
        let taffy_node = node.taffy;
        let hidden = hidden || node.style.layout.hidden;
        let overflow = node.style.layout.overflow;

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

        let mut index = 0;
        while let Some(child) = self
            .nodes
            .get(id)
            .and_then(|node| node.children.get(index))
            .copied()
        {
            self.update_subtree(child, child_origin, child_clip, hidden);
            index += 1;
        }
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
