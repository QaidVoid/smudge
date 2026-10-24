//! Damage accumulation and merging.
//!
//! The paint pass reports what changed as logical rectangles. A renderer
//! repaints each physical rectangle it is handed separately, so many small
//! rectangles cost per-rectangle overhead while one large rectangle costs area.
//! [`DamageAccumulator`] converts the logical rectangles to physical pixels,
//! merges the pairs that are cheap to replace with their union, and falls back
//! to a single full-surface rectangle once the damaged area grows large enough
//! that separate rectangles stop paying for themselves.
//!
//! The cost of merging grows quickly with the number of input rectangles,
//! because the merge compares every pair. On a scene of 600 boxes that all
//! change on every frame, it takes about 1.5 percent of the frame. Damage that
//! stays scattered past a fixed rectangle count skips the reduction that
//! would shrink it and collapses to the full surface instead, because that
//! reduction is cubic and soon costs more than a full repaint.

use crate::geometry::{PixelRect, Rect};

/// How much larger a merged rectangle may be than the two it replaces before
/// merging costs more area than it saves.
///
/// The benchmark suite tunes this value.
const MERGE_SLACK: f64 = 1.3;

/// The largest number of rectangles the output may hold.
///
/// The benchmark suite tunes this value.
const MAX_RECTS: usize = 16;

/// The fraction of the surface above which the output collapses to a single
/// full-surface rectangle.
///
/// The benchmark suite tunes this value.
const FULL_FRAME_RATIO: f64 = 0.6;

/// The number of rectangles above which the output collapses to one
/// full-surface rectangle instead of being reduced to [`MAX_RECTS`].
///
/// The reduction merges one pair at a time and searches every remaining pair
/// for each merge, which is cubic in the number of rectangles. Measured in a
/// release build on a frame that changed scattered boxes in a grid: the
/// damage step spent about 46 ms on 500 rectangles, 291 ms on 1000 and 2.3 s
/// on 2000, nearly all of it in the reduction. A full repaint of a
/// 1920 by 1080 surface holding 600 rounded boxes costs about 3.7 ms on the
/// same machine, so past this limit a full repaint is the cheaper choice,
/// while at 64 rectangles the reduction still costs well under a
/// millisecond.
const REDUCE_LIMIT: usize = 64;

/// A logical rectangle a node painted, with the clip it was drawn under.
///
/// The two are kept apart because a renderer applies a clip in physical
/// pixels: it draws a pixel when any part of it lies inside the clip. Damage
/// that intersected the two in logical space first would miss the pixels a
/// clip edge only partly covers.
#[derive(Copy, Clone, Debug)]
struct ClippedRect {
    /// The union of the node's paint operation bounds, before clipping.
    bounds: Rect,
    /// The rectangle the node was clipped to.
    clip: Rect,
}

/// Collects logical damage rectangles and turns them into a small set of
/// physical pixel rectangles that cover every damaged pixel.
///
/// The buffers are reused across frames, so a steady frame does not allocate
/// once they have grown to the size it needs.
#[derive(Debug, Default)]
pub(crate) struct DamageAccumulator {
    /// Logical rectangles added since the last clear, each with the clip it
    /// was drawn under, in the order they were added.
    clipped: Vec<ClippedRect>,
    /// Whether the whole surface must be repainted.
    full: bool,
    /// The physical rectangles produced by the last [`DamageAccumulator::finish`].
    out: Vec<PixelRect>,
    /// The pixel area the rectangles covered after conversion and before the
    /// merge step, recorded by the last [`DamageAccumulator::finish`].
    pre_merge_pixels: u64,
}

impl DamageAccumulator {
    /// Creates an accumulator with nothing to repaint.
    pub(crate) fn new() -> DamageAccumulator {
        DamageAccumulator::default()
    }

    /// Adds the part of `bounds` that a renderer can draw inside `clip`.
    ///
    /// A renderer draws a pixel when any part of it lies inside the clip, so
    /// the intersection is taken after both rectangles are rounded outward to
    /// physical pixels. A node that a clip cuts off entirely, whose bounds
    /// only touch the clip edge, still damages the pixels that edge passes
    /// through.
    ///
    /// A pair where either rectangle covers no area is ignored. A rectangle
    /// whose coordinates are not finite is dropped when the damage is finished,
    /// because it maps to no pixel.
    pub(crate) fn add_clipped(&mut self, bounds: Rect, clip: Rect) {
        if bounds.is_empty() || clip.is_empty() {
            return;
        }
        self.clipped.push(ClippedRect { bounds, clip });
    }

    /// Marks the whole surface as damaged.
    ///
    /// This overrides the rectangles added around it: the finished damage is
    /// one rectangle covering the surface.
    pub(crate) fn add_full(&mut self) {
        self.full = true;
    }

    /// Returns `true` when nothing needs repainting.
    ///
    /// Only the rectangles added so far are considered, so a rectangle that
    /// turns out to lie outside the surface still counts as damage here.
    pub(crate) fn is_empty(&self) -> bool {
        !self.full && self.clipped.is_empty()
    }

    /// Converts the collected rectangles to physical pixels, merges them, and
    /// clamps them to the surface.
    ///
    /// Every returned rectangle is non-empty and inside the surface, and
    /// together they cover every pixel the added rectangles touch. An empty
    /// slice means nothing needs repainting.
    ///
    /// The output holds at most a fixed number of rectangles. Pairs whose union
    /// is barely larger than the two of them are merged. Overlapping
    /// rectangles are not always merged: two that cross, such as a wide bar
    /// and a tall bar, stay separate because their union is mostly empty, and
    /// a renderer then paints the pixels they share twice, with the same
    /// result. Once the output covers more than a fixed fraction of the
    /// surface, or the whole surface was marked with
    /// [`DamageAccumulator::add_full`], the output is one rectangle covering
    /// the surface. The same collapse happens when more rectangles survive the
    /// merge than a fixed limit, because the reduction that would shrink them
    /// costs more than repainting the surface.
    ///
    /// The returned slice is valid until the next call to
    /// [`DamageAccumulator::clear`], and the internal buffers are reused, so
    /// calling this on consecutive frames does not allocate.
    pub(crate) fn finish(
        &mut self,
        scale: f32,
        surface_width: u32,
        surface_height: u32,
    ) -> &[PixelRect] {
        self.out.clear();
        self.pre_merge_pixels = 0;
        let Some(surface) = surface_rect(surface_width, surface_height) else {
            return &self.out;
        };
        if self.full {
            self.pre_merge_pixels = surface.area();
            self.out.push(surface);
            return &self.out;
        }
        for rect in &self.clipped {
            let bounds = PixelRect::from_logical(rect.bounds, scale, surface_width, surface_height);
            let clip = PixelRect::from_logical(rect.clip, scale, surface_width, surface_height);
            if let (Some(bounds), Some(clip)) = (bounds, clip)
                && let Some(intersection) = bounds.intersect(clip)
            {
                self.out.push(intersection);
            }
        }
        self.pre_merge_pixels = self.out.iter().map(|rect| rect.area()).sum();
        self.merge(surface);
        &self.out
    }

    /// Behaves like [`DamageAccumulator::finish`], and also returns the pixel
    /// area the rectangles covered after conversion and before the merge step
    /// collapsed them, counting an overlap once per rectangle.
    ///
    /// The frame pipeline records this with the time the call took, so a run
    /// can see what the merge step does to the damage.
    pub(crate) fn finish_measured(
        &mut self,
        scale: f32,
        surface_width: u32,
        surface_height: u32,
    ) -> (&[PixelRect], u64) {
        self.finish(scale, surface_width, surface_height);
        let pixels = self.pre_merge_pixels;
        (self.out.as_slice(), pixels)
    }

    /// Removes every collected rectangle, keeping the allocated capacity.
    pub(crate) fn clear(&mut self) {
        self.clipped.clear();
        self.out.clear();
        self.full = false;
    }

    /// Merges the physical rectangles in `self.out` in place and collapses them
    /// to the whole surface when they cover enough of it.
    fn merge(&mut self, surface: PixelRect) {
        loop {
            let before = self.out.len();
            let mut i = 0;
            while i < self.out.len() {
                let mut j = i + 1;
                while j < self.out.len() {
                    if should_merge(self.out[i], self.out[j]) {
                        self.out[i] = self.out[i].union(self.out[j]);
                        self.out.swap_remove(j);
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
            if self.out.len() == before {
                break;
            }
        }

        if self.out.len() > REDUCE_LIMIT {
            self.out.clear();
            self.out.push(surface);
            return;
        }

        while self.out.len() > MAX_RECTS {
            let (i, j) = cheapest_pair(&self.out);
            self.out[i] = self.out[i].union(self.out[j]);
            self.out.swap_remove(j);
        }

        let total: u128 = self.out.iter().map(|rect| u128::from(rect.area())).sum();
        if total as f64 > FULL_FRAME_RATIO * surface.area() as f64 {
            self.out.clear();
            self.out.push(surface);
        }
    }
}

/// Returns the rectangle covering the whole surface, or `None` when the surface
/// has no pixel.
fn surface_rect(width: u32, height: u32) -> Option<PixelRect> {
    if width == 0 || height == 0 {
        return None;
    }
    Some(PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    })
}

/// Returns `true` when two rectangles are cheap enough to replace with their
/// union.
///
/// The comparison is on areas alone. It merges a pair whose union adds little
/// area, including two long thin rectangles that nearly touch and most
/// overlapping pairs. It keeps apart a pair whose union would be mostly empty,
/// including two bars that cross.
fn should_merge(a: PixelRect, b: PixelRect) -> bool {
    let combined = a.area() as f64 + b.area() as f64;
    a.union(b).area() as f64 <= MERGE_SLACK * combined
}

/// Returns the indices of the pair of rectangles whose union adds the least
/// area beyond the area of the pair.
///
/// # Panics
///
/// Panics when `rects` holds fewer than two rectangles.
fn cheapest_pair(rects: &[PixelRect]) -> (usize, usize) {
    debug_assert!(rects.len() >= 2, "a pair needs two rectangles");
    let mut best = (0, 1);
    let mut best_added = u64::MAX;
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let pair = rects[i].area().saturating_add(rects[j].area());
            let added = rects[i].union(rects[j]).area().saturating_sub(pair);
            if added < best_added {
                best_added = added;
                best = (i, j);
            }
        }
    }
    best
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
