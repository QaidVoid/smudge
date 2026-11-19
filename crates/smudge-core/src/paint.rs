//! Paint operations and the display list that renderers replay.
//!
//! Widgets emit [`PaintOp`]s in paint coordinates: logical window
//! coordinates with the scroll offsets of the enclosing scroll views left
//! out, so that scrolling moves existing operations instead of rebuilding
//! them (see [`crate::widget::Widget::paint`]). Outside a scrolled subtree
//! the two coincide. Y grows downwards and the origin is the top left of the
//! window. The frame collects the operations into one [`DisplayList`] in
//! paint order, and both renderers replay that list.
//!
//! A display list may carry [`DisplayItem::PushOffset`] scopes. Items inside a
//! scope are drawn translated by the offset, so a scrolled subtree can keep
//! its operations in its own coordinates while the window scrolls. A renderer
//! that replays a list must honor them.
//!
//! A renderer skips items whose bounds miss the damaged area, so
//! [`PaintOp::bounds`] must cover every pixel an operation can change.

use std::rc::Rc;
use std::sync::Arc;

use crate::geometry::{Color, Corners, Point, Rect};
use crate::style::Edges;

/// The bytes of one font file, shared by every glyph run that uses it.
///
/// Glyph runs carry the font as plain bytes so that paint operations never
/// depend on a particular shaping library. `id` identifies the data uniquely
/// for the lifetime of the process, which renderers use as a glyph cache key
/// without hashing the bytes. `index` selects one face inside a font file that
/// holds several.
#[derive(Clone)]
pub struct FontBlob {
    /// The unique identifier of the font data within this process.
    pub id: u64,
    /// The bytes of the font file.
    pub data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    /// The face to use when the file holds a collection of fonts.
    pub index: u32,
}

impl PartialEq for FontBlob {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.index == other.index
    }
}

impl std::fmt::Debug for FontBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontBlob")
            .field("id", &self.id)
            .field("index", &self.index)
            .field("len", &self.data.as_ref().as_ref().len())
            .finish()
    }
}

/// One glyph of a [`GlyphRun`], positioned in the run's paint coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// The glyph identifier inside the run's font, as shaping produced it.
    pub id: u32,
    /// Horizontal position of the glyph's baseline origin.
    pub x: f32,
    /// Vertical position of the glyph's baseline origin.
    pub y: f32,
}

/// A shaped run of glyphs that all use one font, size and color.
///
/// Glyph positions are in the run's paint coordinates, which leave out the
/// scroll offsets of enclosing views (see [`crate::widget::Widget::paint`]),
/// with each glyph's `x` and `y` at its baseline origin. A renderer
/// rasterizes each glyph separately and blits it in its run's color, so
/// nothing else about the text, such as clusters or line breaking, travels
/// with the run.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    /// The font the glyph identifiers refer to.
    pub font: FontBlob,
    /// The font size in logical pixels.
    pub size: f32,
    /// The straight-alpha sRGB color the glyphs paint with.
    pub color: Color,
    /// The glyphs of the run, in paint order.
    pub glyphs: Vec<PositionedGlyph>,
    /// How far each glyph's outline is pushed outward, in logical pixels, to
    /// draw it heavier than its face, or zero to draw it as the face has it.
    ///
    /// Text carries it when it asks for a weight more than 200 above the
    /// weight of the face it got, and that face has no weight axis. A
    /// renderer widens each outline by this much on every side, with mitered
    /// corners reaching no more than [`EMBOLDEN_MITER_LIMIT`] times this
    /// distance past it, within the limits [`GlyphRun::synthesis`] sets.
    pub embolden: f32,
    /// The slant of each glyph, in degrees, or zero to draw it upright.
    ///
    /// Text carries it when it asks for an italic or oblique style that no
    /// face of its family has. A positive angle leans the top of each glyph
    /// toward the right, shearing it about its baseline origin, within the
    /// limits [`GlyphRun::synthesis`] sets.
    pub skew: f32,
    /// The instance of a variable font the run draws at: one normalized
    /// coordinate per variation axis of the face, in the face's axis order,
    /// as the bits of a 2.14 fixed-point number from -1.0 to 1.0. Empty, or
    /// all zero, draws the face's default instance, as does a face with no
    /// variation axes.
    ///
    /// Text fills it in when it asks for a weight or an italic that a
    /// variable face reaches through its axes.
    pub coords: Vec<i16>,
    /// The rectangle in the run's paint coordinates that covers the ink of
    /// every glyph, emboldened and slanted, plus one logical pixel on every
    /// side for anti-aliasing.
    ///
    /// A renderer may rely on it as the area the run can change: ink outside
    /// it is not guaranteed to be drawn.
    pub ink_bounds: Rect,
}

/// How far a mitered corner of an emboldened outline may reach past the
/// outline, as a multiple of [`GlyphRun::embolden`]. A sharper corner is
/// cut off.
pub const EMBOLDEN_MITER_LIMIT: f32 = 4.0;

/// The largest embolden a renderer draws, as a share of the font size.
pub const MAX_EMBOLDEN_SHARE: f32 = 1.0 / 12.0;

/// The largest embolden a renderer draws at any size, in logical pixels.
pub const MAX_EMBOLDEN: f32 = 32.0;

/// The steepest slant a renderer draws, in degrees either way.
pub const MAX_SKEW: f32 = 45.0;

impl GlyphRun {
    /// Returns the embolden and the slant a renderer draws the run with:
    /// [`GlyphRun::embolden`] held to at most [`MAX_EMBOLDEN_SHARE`] of the
    /// size and at most [`MAX_EMBOLDEN`], and [`GlyphRun::skew`] held to
    /// [`MAX_SKEW`] either way. An embolden that is negative or not finite,
    /// or a slant that is not finite, is zero.
    ///
    /// Text asks for more only above 1536 logical pixels, where its embolden
    /// is drawn at the cap. The limits bound what a run built by hand can
    /// cost a renderer, and keep the renderers drawing it alike.
    pub fn synthesis(&self) -> (f32, f32) {
        let largest = if self.size.is_finite() && self.size > 0.0 {
            (self.size * MAX_EMBOLDEN_SHARE).min(MAX_EMBOLDEN)
        } else {
            0.0
        };
        let embolden = if self.embolden.is_finite() {
            self.embolden.clamp(0.0, largest)
        } else {
            0.0
        };
        let skew = if self.skew.is_finite() {
            self.skew.clamp(-MAX_SKEW, MAX_SKEW)
        } else {
            0.0
        };
        (embolden, skew)
    }
}

/// A single drawing operation in paint coordinates, which leave out the
/// scroll offsets of enclosing views (see [`crate::widget::Widget::paint`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaintOp {
    /// A filled rectangle with optional rounded corners.
    FillRect {
        /// Area the fill covers.
        rect: Rect,
        /// Corner radii of the filled shape.
        radius: Corners,
        /// Straight-alpha sRGB fill color.
        color: Color,
    },
    /// A border drawn inside `rect`, so it never extends past the rectangle.
    StrokeRect {
        /// Outer edge of the border.
        rect: Rect,
        /// Corner radii of the border.
        radius: Corners,
        /// Border thickness in logical pixels, measured inwards from the edge
        /// of `rect`.
        width: f32,
        /// Straight-alpha sRGB border color.
        color: Color,
    },
    /// A border drawn inside `rect` with a width of its own on each side, so
    /// it never extends past the rectangle.
    ///
    /// The inner edge lies each side's width inside `rect`. Each inner
    /// corner is a quarter ellipse centered where the outer corner's circle
    /// is centered: its horizontal radius is the outer radius less the width
    /// of the side beside it, its vertical radius the outer radius less the
    /// width of the side above or below it, and it is square when either
    /// comes to zero or less. Four equal sides give quarter circles. Where a
    /// side is so wide that a corner's curve reaches past the inner side
    /// across from it, the curve stops there. The border never draws outside
    /// `rect`'s rounded outline. A side whose width is zero, negative or not
    /// finite draws nothing along it.
    StrokeSides {
        /// Outer edge of the border.
        rect: Rect,
        /// Corner radii of the border's outer edge.
        radius: Corners,
        /// The thickness of each side in logical pixels, measured inwards
        /// from that side of `rect`.
        widths: Edges<f32>,
        /// Straight-alpha sRGB border color.
        color: Color,
    },
    /// A shaped run of glyphs, painted in one font, size and color.
    ///
    /// The run is shared in an `Rc` so that rebuilding the display list for a
    /// frame in which text did not change copies no glyph vectors.
    Glyphs(Rc<GlyphRun>),
    /// A premultiplied RGBA image stretched over a rounded rectangle.
    Image {
        /// Area the image stretches over.
        rect: Rect,
        /// Corner radii of the rounded rectangle the image clips to.
        radius: Corners,
        /// The pixels to draw, shared with the element that shows them.
        image: crate::image::Image,
    },
}

impl PaintOp {
    /// Returns the rectangle in the operation's paint coordinates that this
    /// operation can change.
    ///
    /// The bounds cover the whole operation, including a stroke, which is drawn
    /// inside its rectangle. Anti-aliasing can shade pixels the edge passes
    /// through, which is why damage converts these bounds to physical pixels by
    /// rounding outwards rather than to the nearest pixel.
    pub fn bounds(&self) -> Rect {
        match self {
            PaintOp::FillRect { rect, .. }
            | PaintOp::StrokeRect { rect, .. }
            | PaintOp::StrokeSides { rect, .. } => *rect,
            PaintOp::Glyphs(run) => run.ink_bounds,
            PaintOp::Image { rect, .. } => *rect,
        }
    }
}

/// One entry in a [`DisplayList`].
///
/// More kinds of items may come in later releases, so a renderer matches it
/// with a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DisplayItem {
    /// A drawing operation.
    Op(PaintOp),
    /// Clips every following item to this rectangle until the matching
    /// [`DisplayItem::PopClip`].
    PushClip(Rect),
    /// Restores the clip that was active before the matching
    /// [`DisplayItem::PushClip`].
    PopClip,
    /// Draws every following item translated by this offset until the
    /// matching [`DisplayItem::PopOffset`].
    ///
    /// Offsets nest with clips and with each other: an item's window position
    /// is its own position plus the sum of the open offsets. Clips pushed
    /// inside an offset scope hold their rectangle in the scope's coordinates,
    /// so translating the scope moves the clip with the items it cuts.
    PushOffset(Point),
    /// Restores the offset that was active before the matching
    /// [`DisplayItem::PushOffset`].
    PopOffset,
    /// Draws every following item, until the matching
    /// [`DisplayItem::PopGroup`], as one group, which is then blended over
    /// what lies below at this opacity, between 0.0 and 1.0.
    ///
    /// The items of a group cover each other as usual, and only the result
    /// shows through at the group's opacity. Groups nest with clips, offsets
    /// and each other.
    PushGroup(f32),
    /// Blends the group the matching [`DisplayItem::PushGroup`] opened.
    PopGroup,
}

/// A flat list of display items in paint order, ready for a renderer to replay.
///
/// The list is rebuilt from the widget tree whenever a node paints. Clips are
/// nested, so every [`DisplayItem::PushClip`] needs a later
/// [`DisplayItem::PopClip`], which [`DisplayList::finish`] checks in debug
/// builds. Offsets nest the same way with [`DisplayItem::PushOffset`] and
/// [`DisplayItem::PopOffset`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    items: Vec<DisplayItem>,
}

impl DisplayList {
    /// Creates an empty list.
    ///
    /// ```
    /// use smudge_core::geometry::{Color, Corners, Rect};
    /// use smudge_core::paint::{DisplayList, PaintOp};
    ///
    /// let mut list = DisplayList::new();
    /// list.push_op(PaintOp::FillRect {
    ///     rect: Rect::new(0.0, 0.0, 8.0, 8.0),
    ///     radius: Corners::ZERO,
    ///     color: Color::BLACK,
    /// });
    /// list.finish();
    /// assert_eq!(list.len(), 1);
    /// ```
    pub fn new() -> DisplayList {
        DisplayList::default()
    }

    /// Removes every item, leaving the list empty and ready for a new build.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Appends one drawing operation.
    pub fn push_op(&mut self, op: PaintOp) {
        self.items.push(DisplayItem::Op(op));
    }

    /// Appends several drawing operations in order.
    pub fn extend_ops(&mut self, ops: &[PaintOp]) {
        self.items.extend(ops.iter().cloned().map(DisplayItem::Op));
    }

    /// Starts a clip to `rect`, which applies to every following item until the
    /// matching [`DisplayList::pop_clip`].
    pub fn push_clip(&mut self, rect: Rect) {
        self.items.push(DisplayItem::PushClip(rect));
    }

    /// Ends the most recent clip.
    ///
    /// Callers must pair every call with an earlier
    /// [`DisplayList::push_clip`]. [`DisplayList::finish`] reports an imbalance
    /// in debug builds.
    pub fn pop_clip(&mut self) {
        self.items.push(DisplayItem::PopClip);
    }

    /// Starts an offset scope, which translates every following item by
    /// `offset` until the matching [`DisplayList::pop_offset`].
    ///
    /// Scopes nest with clips and with each other. A clip pushed inside a
    /// scope holds its rectangle in the scope's coordinates, so it moves with
    /// the scope.
    pub fn push_offset(&mut self, offset: Point) {
        self.items.push(DisplayItem::PushOffset(offset));
    }

    /// Ends the most recent offset scope.
    ///
    /// Callers must pair every call with an earlier
    /// [`DisplayList::push_offset`]. [`DisplayList::finish`] reports an
    /// imbalance in debug builds.
    pub fn pop_offset(&mut self) {
        self.items.push(DisplayItem::PopOffset);
    }

    /// Starts a group drawn at `opacity`, which blends every following item
    /// as one until the matching [`DisplayList::pop_group`].
    pub fn push_group(&mut self, opacity: f32) {
        self.items.push(DisplayItem::PushGroup(opacity));
    }

    /// Ends the most recent group.
    ///
    /// Callers must pair every call with an earlier
    /// [`DisplayList::push_group`]. [`DisplayList::finish`] reports an
    /// imbalance in debug builds.
    pub fn pop_group(&mut self) {
        self.items.push(DisplayItem::PopGroup);
    }

    /// Returns the items in paint order.
    pub fn items(&self) -> &[DisplayItem] {
        &self.items
    }

    /// Returns the number of items, counting clips.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` when the list holds no item.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Completes a build of the list.
    ///
    /// The frame calls this once after rebuilding the list and before handing
    /// it to a renderer. In release builds it does nothing, because an
    /// unbalanced list is an internal invariant violation rather than input
    /// from outside the library.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when a [`DisplayItem::PushClip`] has no matching
    /// [`DisplayItem::PopClip`], when a `PopClip` appears with no clip open,
    /// or when an offset scope or a group is unbalanced the same way.
    pub fn finish(&mut self) {
        #[cfg(debug_assertions)]
        self.debug_assert_balanced();
    }

    /// Panics unless every clip, offset scope and group was pushed before
    /// it was popped.
    #[cfg(debug_assertions)]
    fn debug_assert_balanced(&self) {
        let mut open = 0usize;
        let mut open_offsets = 0usize;
        let mut open_groups = 0usize;
        for item in &self.items {
            match item {
                DisplayItem::PushClip(_) => open += 1,
                DisplayItem::PopClip => {
                    debug_assert!(open > 0, "PopClip with no open clip");
                    open = open.saturating_sub(1);
                }
                DisplayItem::PushOffset(_) => open_offsets += 1,
                DisplayItem::PopOffset => {
                    debug_assert!(open_offsets > 0, "PopOffset with no open offset");
                    open_offsets = open_offsets.saturating_sub(1);
                }
                DisplayItem::PushGroup(_) => open_groups += 1,
                DisplayItem::PopGroup => {
                    debug_assert!(open_groups > 0, "PopGroup with no open group");
                    open_groups = open_groups.saturating_sub(1);
                }
                DisplayItem::Op(_) => {}
            }
        }
        debug_assert_eq!(open, 0, "PushClip with no matching PopClip");
        debug_assert_eq!(open_offsets, 0, "PushOffset with no matching PopOffset");
        debug_assert_eq!(open_groups, 0, "PushGroup with no matching PopGroup");
    }
}

#[cfg(test)]
#[path = "paint_tests.rs"]
mod tests;
