//! Paint operations and the display list that renderers replay.
//!
//! Widgets emit [`PaintOp`]s in absolute logical window coordinates, with the
//! origin at the top left of the window and y growing downwards. The frame
//! collects them into one [`DisplayList`] in paint order, and both renderers
//! replay that list.
//!
//! A renderer skips items whose bounds miss the damaged area, so
//! [`PaintOp::bounds`] must cover every pixel an operation can change.

use crate::geometry::{Color, Corners, Rect};

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
