//! The layout pass.
//!
//! The pass runs the layout engine only when a node needs it, then walks the
//! tree once to turn the engine's parent-relative rectangles into absolute
//! rectangles and clips. Logical coordinates are rounded to whole logical
//! pixels, which is the engine's default, so a layout rectangle never carries a
//! fraction of a pixel.

use taffy::{AvailableSpace as TaffySpace, LayoutInput, LayoutOutput, Size as TaffySize};

use crate::geometry::{Point, Rect, Size};
use crate::style::{Length, Overflow, Position};
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
    /// parent's clip, intersected with the area inside the parent's border
    /// when the parent clips its children. A node scrolled with
    /// [`WidgetTree::set_scroll_offset`] places its flowing children that much
    /// further up and to the left.
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
            self.refresh_hover();
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

    /// Sets the scroll offset of `id` and moves its flowing descendants.
    ///
    /// The offset is a translation applied after layout, so this never runs
    /// the layout engine: it re-runs the walk that computes absolute
    /// rectangles for the subtree of `id` alone, from the origin and the clip
    /// its parent gives it, which is what makes scrolling cheap next to a
    /// layout pass. A node whose rectangle changed is marked for painting, so
    /// its operations are regenerated at the moved position and its old and
    /// new bounds reach the damage of the frame.
    ///
    /// The walk leaves the absolutely positioned children of `id` where their
    /// insets place them. Before the first layout pass, and while `id` or an
    /// ancestor is hidden, the offset is only stored, and the next walk that
    /// reaches the subtree applies it. A stale id does nothing. An equal
    /// offset does nothing.
    pub(crate) fn set_scroll_offset(&mut self, id: WidgetId, offset: Point) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.scroll_offset == offset {
            return;
        }
        node.scroll_offset = offset;
        let parent = node.parent;
        let position = node.resolved.layout.position;
        let Some(viewport) = self.viewport else {
            return;
        };
        let mut ancestor = Some(id);
        while let Some(current) = ancestor {
            let node = &self.nodes[current];
            if node.resolved.layout.hidden {
                return;
            }
            ancestor = node.parent;
        }
        let (origin, clip) = match parent {
            Some(parent) => {
                let frame = self.child_frame(parent);
                (frame.origin(position), frame.clip)
            }
            None => (
                Point::default(),
                Rect::new(0.0, 0.0, viewport.width, viewport.height),
            ),
        };
        if self.update_subtree(id, origin, clip, false) {
            // The subtree moved under a pointer that stays still, so the
            // hovered chain is rebuilt from the last pointer position, as the
            // layout pass does. A hover style can mark a node for layout,
            // which the frame's layout pass applies after this.
            if self.pointer_inside && self.pressed.is_none() {
                self.refresh_hover();
            }
        }
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

        let content = if hidden {
            Rect::default()
        } else {
            inset(inset(rect, layout.border), layout.padding)
        };

        let node = self.nodes.get_mut(id).expect("the id is a live node");
        let changed = node.rect != rect || node.content != content || node.clip != clip;
        node.rect = rect;
        node.content = content;
        node.clip = clip;
        let frame = ChildFrame::new(rect, clip, overflow, layout.border, node.scroll_offset);

        if changed {
            self.mark(id, Dirty::Paint);
        }

        let mut subtree_changed = changed;
        let mut index = 0;
        while let Some(child) = self
            .nodes
            .get(id)
            .and_then(|node| node.children.get(index))
            .copied()
        {
            let origin = frame.origin(self.nodes[child].resolved.layout.position);
            subtree_changed |= self.update_subtree(child, origin, frame.clip, hidden);
            index += 1;
        }
        subtree_changed
    }

    /// Returns the area inside the border of `id`, which is what a node that
    /// clips its children clips them to.
    ///
    /// # Panics
    ///
    /// Panics when `id` is stale or has no live layout node, which are
    /// internal invariants for an id the caller took from the tree.
    pub(super) fn inside_border(&self, id: WidgetId) -> Rect {
        let node = &self.nodes[id];
        let layout = self
            .taffy
            .layout(node.taffy)
            .expect("every node has a live layout node");
        inset(node.rect, layout.border)
    }

    /// Returns the origins and the clip the walk gives the children of `id`,
    /// from the rectangle and clip the last walk left on it.
    ///
    /// # Panics
    ///
    /// Panics when `id` is stale or has no live layout node, which are
    /// internal invariants for an id taken from a live node's parent.
    fn child_frame(&self, id: WidgetId) -> ChildFrame {
        let node = &self.nodes[id];
        let layout = self
            .taffy
            .layout(node.taffy)
            .expect("every node has a live layout node");
        ChildFrame::new(
            node.rect,
            node.clip,
            node.resolved.layout.overflow,
            layout.border,
            node.scroll_offset,
        )
    }

    /// Rebuilds the hovered chain from the last pointer position.
    ///
    /// The layout pass and a scroll offset change both move nodes under a
    /// pointer that stays still, and no pointer event announces the move, so
    /// both rebuild the chain the same way.
    fn refresh_hover(&mut self) {
        let target = self.hover_target(self.pointer);
        self.set_hover_target(target);
    }
}

/// Where the walk places the children of one node, and what it clips them to.
struct ChildFrame {
    /// The origin of a child in the flow, which the node's scroll offset
    /// moves.
    flow: Point,
    /// The origin of an absolutely positioned child, which is placed by its
    /// insets outside the flow, so a scroll offset does not move it.
    absolute: Point,
    /// The clip every child inherits.
    clip: Rect,
}

impl ChildFrame {
    /// Builds the frame of a node from its rectangle, its clip, its overflow,
    /// its border widths and its scroll offset.
    ///
    /// A node that clips its children clips them to the area inside its
    /// border, so content that scrolls or overflows never covers the border.
    fn new(
        rect: Rect,
        clip: Rect,
        overflow: Overflow,
        border: taffy::Rect<f32>,
        scroll: Point,
    ) -> ChildFrame {
        ChildFrame {
            flow: Point {
                x: rect.x0 - scroll.x,
                y: rect.y0 - scroll.y,
            },
            absolute: rect.origin(),
            clip: if overflow == Overflow::Clip {
                clip.intersect(inset(rect, border))
            } else {
                clip
            },
        }
    }

    /// Returns the origin of a child with `position`.
    fn origin(&self, position: Position) -> Point {
        match position {
            Position::Absolute => self.absolute,
            Position::Relative => self.flow,
        }
    }
}

/// Shrinks `rect` by `edges` on each side, never past an empty rectangle.
fn inset(rect: Rect, edges: taffy::Rect<f32>) -> Rect {
    let x0 = rect.x0 + edges.left;
    let y0 = rect.y0 + edges.top;
    Rect::new(
        x0,
        y0,
        (rect.x1 - edges.right).max(x0),
        (rect.y1 - edges.bottom).max(y0),
    )
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
