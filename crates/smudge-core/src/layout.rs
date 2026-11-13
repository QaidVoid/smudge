//! The layout pass.
//!
//! The pass runs the layout engine only when a node needs it, then walks the
//! tree once to turn the engine's parent-relative rectangles into absolute
//! rectangles and clips. Logical coordinates are rounded to whole logical
//! pixels, which is the engine's default, so a layout rectangle never carries a
//! fraction of a pixel.

use taffy::{
    AvailableSpace as TaffySpace, LayoutInput, LayoutOutput, ResolveOrZero, Size as TaffySize,
};

use crate::geometry::{Point, Rect, Size};
use crate::reactive::{Read, RwSignal};
use crate::style::{Length, Overflow, Position};
use crate::widget::{AvailableSpace, MeasureInput};

use super::{Dirty, PaintedBounds, WidgetId, WidgetTree};

/// Converts the engine's space variant to the type a widget's measure sees.
fn from_engine(space: TaffySpace) -> AvailableSpace {
    match space {
        TaffySpace::Definite(size) => AvailableSpace::Definite(size),
        TaffySpace::MinContent => AvailableSpace::MinContent,
        TaffySpace::MaxContent => AvailableSpace::MaxContent,
    }
}

impl WidgetTree {
    /// Sets the scale the next layout pass measures at, in physical pixels
    /// per logical pixel.
    ///
    /// A scale change changes every widget's measure answer without changing
    /// any style or offered space, so the layout engine's cached measures are
    /// all stale and every widget is asked again. The tree starts at 1.0.
    ///
    /// A scale that is not finite or not positive is ignored, the way
    /// [`PixelRect::from_logical`](crate::geometry::PixelRect::from_logical)
    /// ignores one.
    pub(crate) fn set_scale(&mut self, scale: f32) {
        if !scale.is_finite() || scale <= 0.0 || self.scale == scale {
            return;
        }
        self.scale = scale;
        self.layout_dirty = true;
        for (_, node) in self.nodes.iter() {
            self.taffy
                .mark_dirty(node.taffy)
                .expect("every widget node has a live layout node");
        }
    }

    /// Lays the tree out for `viewport` and returns whether the layout engine
    /// ran.
    ///
    /// The engine runs only when a node is marked [`Dirty::Layout`] or when the
    /// viewport differs from the previous pass, so a paint-only change costs
    /// nothing here. The root is sized to the viewport, which overrides any
    /// size set on the root element. After the pass, a node that mounted with
    /// autofocus takes the focus, a node whose follow signal reads `true`
    /// pins its scroll offset to the end of its content with the rectangles
    /// this pass just wrote, a focus move that asked for it scrolls the
    /// focused node into view, and a focus, hover, follow or reveal change
    /// that carries a layout patch runs the engine once more, so all of them
    /// settle in this frame. A focus move on a frame that lays nothing out is
    /// revealed with the rectangles the last pass left.
    ///
    /// Each node ends the pass with an absolute rectangle and clip. A node
    /// whose rectangle in paint coordinates changed, or whose clip became or
    /// stopped being empty, is marked [`Dirty::Paint`], because it has to
    /// paint again; a node that only moved in the window keeps its operations
    /// and damages its old and new area. A hidden node and its subtree get an
    /// empty rectangle and an empty clip, so they paint nothing. Hiding a node
    /// therefore empties its clip, which marks it so the paint pass clears the
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
        let ran = match self.layout_pass(viewport) {
            Some(rects_changed) => {
                // A trap that showed records the focus before an autofocus
                // inside it moves the focus in, and a trap that hid gives the
                // focus back.
                self.resolve_focus_traps();
                // A node that mounted for this frame with autofocus takes the
                // focus here: after the pass laid it out, so the styles the
                // pass resolved say whether it can hold the focus, and before
                // the frame paints, so it never shows a frame without the
                // focus.
                self.resolve_autofocus();
                // A node that the pass moved under a pointer that stays still
                // keeps a stale hover, because no pointer event announces the
                // move. The chain is rebuilt from the last pointer position,
                // unless a held press keeps the hover on its capture.
                if rects_changed && self.pointer_inside && self.pressed.is_none() {
                    self.refresh_hover();
                }
                // A pinned child goes back to its place with the sizes this
                // pass gave its siblings, so no frame shows a grown child over
                // the next.
                self.resolve_pins();
                // A following node pins its offset here, after the pass laid
                // its content out and before the frame paints, so a frame
                // that grows the content never shows the end out of view. The
                // painted heights the bindings read are one frame old.
                self.resolve_follow();
                true
            }
            None => false,
        };
        // The focused node comes into view after the pins and the follow
        // pin, so a view that follows its end still shows a focus that moved
        // inside it, and before the frame paints, so no frame shows the
        // focus out of view.
        self.resolve_reveal();
        // The focus autofocus set, the hover the refresh above set, and the
        // offsets a follow pin or a reveal wrote, whose moved content can
        // pick up a hover, can carry variant patches that change the layout,
        // and nothing would ask for another frame to apply them. One more
        // pass settles them all in this frame.
        let settled = self.layout_dirty && self.layout_pass(viewport).is_some();
        if settled {
            // The settle pass can change the extent a following view reaches,
            // the way a hover variant its own pin picked up can. Pin again so
            // the frame still ends at the end, and reveal again so a focus
            // style that resized the focused node still ends in view. The
            // offsets they write usually equal the ones already applied and
            // do nothing. When one moves, the hover it refreshes may carry a
            // layout patch that waits for a later pass, so the passes a frame
            // runs stay bounded at two.
            self.resolve_pins();
            self.resolve_follow();
            self.resolve_reveal();
        }
        self.reveal_focus = false;
        ran || settled
    }

    /// Sets the scroll offset of `id` and moves its flowing descendants.
    ///
    /// The offset is a translation applied after layout, so this never runs
    /// the layout engine: it re-runs the walk that computes absolute
    /// rectangles for the subtree of `id` alone, from the origin and the clip
    /// its parent gives it, which is what makes scrolling cheap next to a
    /// layout pass.
    ///
    /// The children that move keep their operations: the walk queues their
    /// painted bounds as damage at the old and the new position, and the
    /// display list pass wraps them in an offset scope that moves them in the
    /// window, so a scroll frame repaints no moved node at all.
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
        let (origin, clip, shift) = match parent {
            Some(parent) => {
                let frame = self.child_frame(parent);
                (frame.origin, frame.clip, frame.shift(position))
            }
            None => (
                Point::default(),
                Rect::new(0.0, 0.0, viewport.width, viewport.height),
                Point::default(),
            ),
        };
        if self.update_subtree(id, origin, clip, false, shift) {
            // The subtree moved under a pointer that stays still, so the
            // hovered chain is rebuilt from the last pointer position, as the
            // layout pass does. A hover style can mark a node for layout,
            // which the frame's layout pass applies after this.
            if self.pointer_inside && self.pressed.is_none() {
                self.refresh_hover();
            }
        }
    }

    /// Sets the signal that pins the scroll offset of `id` to the end of its
    /// flowing content while it reads `true`.
    ///
    /// The element's builders call this at mount. While the signal reads
    /// `true`, the layout pass pins the offset after it runs. See
    /// [`WidgetTree::resolve_follow`]. A stale id does nothing.
    pub(crate) fn set_follow(&mut self, id: WidgetId, signal: RwSignal<bool>) {
        if let Some(node) = self.nodes.get_mut(id) {
            if node.follow.is_none() {
                self.following.push(id);
            }
            node.follow = Some(signal);
        }
    }

    /// Sets the flowing child that `id` keeps in place, by index, and how far
    /// below the top of its content box it sits.
    ///
    /// The element's builders call this. Outside a pending layout pass the
    /// children move at once, with the sizes the last pass gave them, so a
    /// pin that changes on a frame that lays nothing out still applies. A
    /// stale id does nothing.
    pub(crate) fn set_pin(&mut self, id: WidgetId, pin: Option<(usize, f32)>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if !self.pinned.contains(&id) {
            self.pinned.push(id);
        }
        node.pin = pin;
        if !self.layout_dirty {
            self.apply_pin(id);
        }
    }

    /// Moves the children of every pinning node so each pinned child sits
    /// where its pin says, with the sizes the last pass gave the children.
    fn resolve_pins(&mut self) {
        let nodes = &self.nodes;
        self.pinned.retain(|id| nodes.contains_key(*id));
        for at in 0..self.pinned.len() {
            let id = self.pinned[at];
            self.apply_pin(id);
        }
    }

    /// Moves the children of `id` so its pinned child sits where the pin
    /// says, through the node's scroll offset.
    ///
    /// A hidden node, a missing pin, an index past the last child, and an
    /// absolutely positioned or hidden child leave the children where they
    /// are.
    ///
    /// # Panics
    ///
    /// Panics when the node or the pinned child has no live layout node,
    /// which is an internal invariant for ids the caller took from the tree.
    fn apply_pin(&mut self, id: WidgetId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let Some((index, y)) = node.pin else {
            return;
        };
        if node.resolved.layout.hidden {
            return;
        }
        let Some(child) = node
            .children
            .get(index)
            .and_then(|child| self.nodes.get(*child))
        else {
            return;
        };
        if child.resolved.layout.position == Position::Absolute || child.resolved.layout.hidden {
            return;
        }
        let layout = self
            .taffy
            .layout(node.taffy)
            .expect("every node has a live layout node");
        let top = layout.border.top + layout.padding.top;
        let child_top = self
            .taffy
            .layout(child.taffy)
            .expect("every node has a live layout node")
            .location
            .y;
        let current = node.scroll_offset;
        let offset = child_top - top - y;
        if offset.is_finite() {
            self.set_scroll_offset(
                id,
                Point {
                    x: current.x,
                    y: offset,
                },
            );
        }
    }

    /// Pins every following node's scroll offset to the end of its content.
    ///
    /// Only the nodes that carry a follow signal are visited. A node whose
    /// follow signal reads `true` takes the furthest offset its
    /// flowing content reaches. The pin reads the rectangles the passes of
    /// this frame just wrote, where the heights a widget reports when it
    /// paints are one frame old, so a frame that grows the content never
    /// paints a following view short of its end. A hidden node is skipped,
    /// because its rectangles are empty and say nothing about its content.
    ///
    /// The pin itself runs no layout: an offset is a translation applied
    /// after layout. A hover the moved content picks up may ask for one. The
    /// settle pass after the pin that runs before it applies that patch in
    /// the same frame, and a hover the pin after the settle pass picks up
    /// waits for a later frame's pass.
    fn resolve_follow(&mut self) {
        let nodes = &self.nodes;
        self.following.retain(|id| nodes.contains_key(*id));
        for at in 0..self.following.len() {
            let id = self.following[at];
            let node = &self.nodes[id];
            let active = !node.resolved.layout.hidden
                && node
                    .follow
                    .as_ref()
                    .is_some_and(|signal| signal.read(|active| *active));
            if !active {
                continue;
            }
            let current = node.scroll_offset;
            let Some(max) = self.max_scroll_offset(id) else {
                continue;
            };
            self.set_scroll_offset(
                id,
                Point {
                    x: current.x,
                    y: max,
                },
            );
        }
    }

    /// Returns the furthest offset the flowing content of `id` reaches, or
    /// `None` when `id` is stale.
    ///
    /// The extent is measured from the content box, like the heights a
    /// scroll view reports when it paints are, so the follow pin, a reveal
    /// and the wheel clamp agree on where the end is. Absolutely positioned
    /// children do not count, because a scroll offset does not move them.
    ///
    /// # Panics
    ///
    /// Panics when a node in the subtree has no live layout node, which is
    /// an internal invariant for ids the caller took from the tree.
    pub(super) fn max_scroll_offset(&self, id: WidgetId) -> Option<f32> {
        let node = self.nodes.get(id)?;
        let layout = self
            .taffy
            .layout(node.taffy)
            .expect("every node has a live layout node");
        let top = layout.border.top + layout.padding.top;
        let mut bottom = 0.0f32;
        for &child in &node.children {
            let child = &self.nodes[child];
            if child.resolved.layout.position == Position::Absolute {
                continue;
            }
            let child_layout = self
                .taffy
                .layout(child.taffy)
                .expect("every node has a live layout node");
            bottom = bottom.max(child_layout.location.y + child_layout.size.height);
        }
        Some((bottom - top - node.content.height()).max(0.0))
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
                    // padding and border before it measures, and offers the
                    // space inside them. The sizes it treats as fixed are the
                    // node's outer ones, so they lose the same insets here and
                    // the widget is offered the content box on every axis. Its
                    // result is used only for an axis the style leaves open,
                    // but the engine calls it on the final pass regardless.
                    let parent_width = inputs.parent_size.width;
                    let inset = (style.padding.resolve_or_zero(parent_width, |_, _| 0.0)
                        + style.border.resolve_or_zero(parent_width, |_, _| 0.0))
                    .sum_axes();
                    taffy::compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known, available| {
                            let measured = node.and_then(|node| {
                                node.widget.measure(MeasureInput {
                                    known_width: known
                                        .width
                                        .map(|width| (width - inset.width).max(0.0)),
                                    known_height: known
                                        .height
                                        .map(|height| (height - inset.height).max(0.0)),
                                    available_width: from_engine(available.width),
                                    available_height: from_engine(available.height),
                                    scale: self.scale,
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
        let rects_changed =
            self.update_subtree(root, Point::default(), clip, false, Point::default());

        self.layout_dirty = false;
        for node in self.nodes.values_mut() {
            if node.dirty == Dirty::Layout {
                node.dirty = Dirty::Paint;
            }
        }
        Some(rects_changed)
    }

    /// Writes the absolute rectangle and clip of `id` and its subtree, given
    /// the subtree's origin in paint coordinates, its clip in window
    /// coordinates and its paint shift, and returns whether any node in the
    /// subtree changed its rectangle or clip in the window.
    ///
    /// `hidden` reports whether an ancestor is hidden, which makes this node
    /// and everything below it empty. A flowing child adds the scroll offset
    /// of its parent to the shift it inherits, and an absolutely positioned
    /// child keeps it.
    ///
    /// A node is marked for painting when its rectangle or content box in
    /// paint coordinates changed, or when its clip became empty or stopped
    /// being empty, so its operations are regenerated and its old and new
    /// bounds reach the damage of the frame. A node that only moved in the
    /// window or changed its clip, which is how a scroll moves its flowing
    /// children, keeps its operations, because they hold neither: its painted
    /// bounds are queued as damage at the old and the new shift and clip, and
    /// the display list pass later wraps the moved children in an offset
    /// scope so renderers draw them at the moved position without repainting
    /// them.
    fn update_subtree(
        &mut self,
        id: WidgetId,
        origin: Point,
        clip: Rect,
        hidden: bool,
        shift: Point,
    ) -> bool {
        let node = self.nodes.get(id).expect("the id is a live node");
        let taffy_node = node.taffy;
        let hidden = hidden || node.resolved.layout.hidden;
        let overflow = node.resolved.layout.overflow;

        let layout = *self
            .taffy
            .layout(taffy_node)
            .expect("every node has a live layout node");
        let clip = if hidden { Rect::default() } else { clip };
        let (paint_rect, paint_content, rect, content) = if hidden {
            Default::default()
        } else {
            let paint_rect = Rect::new(
                origin.x + layout.location.x,
                origin.y + layout.location.y,
                origin.x + layout.location.x + layout.size.width,
                origin.y + layout.location.y + layout.size.height,
            );
            let paint_content = inset(inset(paint_rect, layout.border), layout.padding);
            let back = Point {
                x: -shift.x,
                y: -shift.y,
            };
            (
                paint_rect,
                paint_content,
                paint_rect.translated(back),
                paint_content.translated(back),
            )
        };

        let node = self.nodes.get_mut(id).expect("the id is a live node");
        let changed = node.rect != rect || node.content != content || node.clip != clip;
        let repaint = node.paint_rect != paint_rect
            || node.paint_content != paint_content
            || node.clip.is_empty() != clip.is_empty();
        if !repaint
            && changed
            && let Some(painted) = node.painted
        {
            let moved = PaintedBounds {
                clip,
                shift,
                ..painted
            };
            node.painted = Some(moved);
            self.pending_damage.push(painted);
            self.pending_damage.push(moved);
        }
        let node = self.nodes.get_mut(id).expect("the id is a live node");
        node.rect = rect;
        node.content = content;
        node.clip = clip;
        node.paint_shift = shift;
        node.paint_rect = paint_rect;
        node.paint_content = paint_content;
        let frame = ChildFrame::new(
            paint_rect,
            rect,
            clip,
            overflow,
            layout.border,
            node.scroll_offset,
            shift,
        );
        if repaint {
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
            let child_shift = frame.shift(self.nodes[child].resolved.layout.position);
            subtree_changed |=
                self.update_subtree(child, frame.origin, frame.clip, hidden, child_shift);
            index += 1;
        }
        subtree_changed
    }

    /// Returns the area inside the border of `id` in its paint coordinates,
    /// which is what a node that clips its children clips them to.
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
        inset(node.paint_rect, layout.border)
    }

    /// Returns the origin, the clip and the shifts the walk gives the
    /// children of `id`, from the geometry the last walk left on it.
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
            node.paint_rect,
            node.rect,
            node.clip,
            node.resolved.layout.overflow,
            layout.border,
            node.scroll_offset,
            node.paint_shift,
        )
    }

    /// Rebuilds the hovered chain from the last pointer position.
    ///
    /// The layout pass and a scroll offset change both move nodes under a
    /// pointer that stays still, and no pointer event announces the move, so
    /// both rebuild the chain the same way.
    ///
    /// A rebuild never takes the hover away from a node whose hover patch
    /// changes its layout. Such a patch can move the node off the pointer,
    /// by narrowing it for example, and dropping the hover would put the
    /// node back under the pointer, so the hover would flip with every frame
    /// that lays out, and which state a frame showed would depend on how
    /// many passes ran before it. The node keeps the hover until a pointer
    /// event moves it, which makes the hover a function of the layout and
    /// the pointer events alone.
    fn refresh_hover(&mut self) {
        let target = self.hover_target(self.pointer);
        if target == self.hovered {
            return;
        }
        let mut next = self.hovered;
        while let Some(id) = next {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            let stays = target.is_some_and(|target| self.is_within(target, id));
            if !stays && self.hover_moves_layout(id) {
                return;
            }
            next = node.parent;
        }
        self.set_hover_target(target);
    }

    /// Returns whether the hover patch of `id` changes the layout of its
    /// base style.
    fn hover_moves_layout(&self, id: WidgetId) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        let Some(patch) = node
            .variants
            .as_ref()
            .and_then(|variants| variants.hovered.as_ref())
        else {
            return false;
        };
        let mut style = node.base.clone();
        patch.apply(&mut style);
        style.layout != node.base.layout
    }
}

/// Where the walk places the children of one node, what it clips them to and
/// what paint shift they inherit.
struct ChildFrame {
    /// The origin every child is placed from, in paint coordinates.
    ///
    /// A scroll offset moves the flowing children in the window and adds to
    /// their shift by the same amount, so in paint coordinates they start
    /// where the absolutely positioned ones do.
    origin: Point,
    /// The clip every child inherits, in window coordinates.
    clip: Rect,
    /// The paint shift a child in the flow inherits: the node's own shift
    /// plus its scroll offset, because the child paints inside the offset
    /// scope the node's scroll produces.
    flow_shift: Point,
    /// The paint shift an absolutely positioned child inherits, which is the
    /// node's own shift: a scroll offset does not move it.
    absolute_shift: Point,
}

impl ChildFrame {
    /// Builds the frame of a node from its rectangle in paint and in window
    /// coordinates, its clip, its overflow, its border widths, its scroll
    /// offset and its paint shift.
    ///
    /// A node that clips its children clips them to the area inside its
    /// border, so content that scrolls or overflows never covers the border.
    fn new(
        paint_rect: Rect,
        rect: Rect,
        clip: Rect,
        overflow: Overflow,
        border: taffy::Rect<f32>,
        scroll: Point,
        shift: Point,
    ) -> ChildFrame {
        ChildFrame {
            origin: paint_rect.origin(),
            clip: if overflow.clips() {
                clip.intersect(inset(rect, border))
            } else {
                clip
            },
            flow_shift: Point {
                x: shift.x + scroll.x,
                y: shift.y + scroll.y,
            },
            absolute_shift: shift,
        }
    }

    /// Returns the paint shift of a child with `position`.
    fn shift(&self, position: Position) -> Point {
        match position {
            Position::Absolute => self.absolute_shift,
            Position::Relative => self.flow_shift,
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
