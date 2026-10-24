//! Paint operations and the display list that renderers replay.
//!
//! Widgets emit [`PaintOp`]s in absolute logical window coordinates, with the
//! origin at the top left of the window and y growing downwards. The frame
//! collects them into one [`DisplayList`] in paint order, and both renderers
//! replay that list.
//!
//! A renderer skips items whose bounds miss the damaged area, so
//! [`PaintOp::bounds`] must cover every pixel an operation can change.

use std::rc::Rc;
use std::sync::Arc;

use crate::geometry::{Color, Corners, Rect};

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

/// One glyph of a [`GlyphRun`], positioned in absolute logical window
/// coordinates.
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
/// Glyph positions are absolute logical window coordinates, with each glyph's
/// `x` and `y` at its baseline origin. A renderer rasterizes each glyph
/// separately and blits it in its run's color, so nothing else about the text,
/// such as clusters or line breaking, travels with the run.
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
    /// The rectangle in absolute logical coordinates that covers the ink of
    /// every glyph, plus one logical pixel on every side for anti-aliasing.
    ///
    /// A renderer may rely on it as the area the run can change: ink outside
    /// it is not guaranteed to be drawn.
    pub ink_bounds: Rect,
}

/// A single drawing operation in absolute logical window coordinates.
#[derive(Clone, Debug, PartialEq)]
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
    /// A shaped run of glyphs, painted in one font, size and color.
    ///
    /// The run is shared in an `Rc` so that rebuilding the display list for a
    /// frame in which text did not change copies no glyph vectors.
    Glyphs(Rc<GlyphRun>),
}

impl PaintOp {
    /// Returns the rectangle in absolute logical window coordinates that this
    /// operation can change.
    ///
    /// The bounds cover the whole operation, including a stroke, which is drawn
    /// inside its rectangle. Anti-aliasing can shade pixels the edge passes
    /// through, which is why damage converts these bounds to physical pixels by
    /// rounding outwards rather than to the nearest pixel.
    pub fn bounds(&self) -> Rect {
        match self {
            PaintOp::FillRect { rect, .. } | PaintOp::StrokeRect { rect, .. } => *rect,
            PaintOp::Glyphs(run) => run.ink_bounds,
        }
    }
}

/// One entry in a [`DisplayList`].
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    /// A drawing operation.
    Op(PaintOp),
    /// Clips every following item to this rectangle until the matching
    /// [`DisplayItem::PopClip`].
    PushClip(Rect),
    /// Restores the clip that was active before the matching
    /// [`DisplayItem::PushClip`].
    PopClip,
}

/// A flat list of display items in paint order, ready for a renderer to replay.
///
/// The list is rebuilt from the widget tree whenever a node paints. Clips are
/// nested, so every [`DisplayItem::PushClip`] needs a later
/// [`DisplayItem::PopClip`], which [`DisplayList::finish`] checks in debug
/// builds.
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
    /// [`DisplayItem::PopClip`], or when a `PopClip` appears with no clip open.
    pub fn finish(&mut self) {
        #[cfg(debug_assertions)]
        self.debug_assert_balanced();
    }

    /// Panics unless every clip was pushed before it was popped.
    #[cfg(debug_assertions)]
    fn debug_assert_balanced(&self) {
        let mut open = 0usize;
        for item in &self.items {
            match item {
                DisplayItem::PushClip(_) => open += 1,
                DisplayItem::PopClip => {
                    debug_assert!(open > 0, "PopClip with no open clip");
                    open = open.saturating_sub(1);
                }
                DisplayItem::Op(_) => {}
            }
        }
        debug_assert_eq!(open, 0, "PushClip with no matching PopClip");
    }
}

#[cfg(test)]
#[path = "paint_tests.rs"]
mod tests;
