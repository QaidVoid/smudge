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
//! The cost of merging grows quickly with the number of input rectangles: the
//! pass that collapses a large set to a small one compares every pair again
//! for each merge it makes. A frame that damages several hundred rectangles at
//! once is therefore slow, which is a known cost that the benchmark suite has
//! to weigh against the damage area it saves.

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

/// Collects logical damage rectangles and turns them into a small set of
/// physical pixel rectangles that cover every damaged pixel.
///
/// The buffers are reused across frames, so a steady frame does not allocate
/// once they have grown to the size it needs.
///
/// # Examples
///
/// ```
/// use smudge_core::damage::DamageAccumulator;
/// use smudge_core::geometry::{PixelRect, Rect};
///
/// let mut damage = DamageAccumulator::new();
/// damage.add(Rect::new(2.0, 2.0, 4.0, 4.0));
/// damage.add(Rect::new(3.0, 3.0, 5.0, 5.0));
///
/// assert_eq!(
///     damage.finish(1.0, 16, 16),
///     &[PixelRect {
///         x: 2,
///         y: 2,
///         width: 3,
///         height: 3,
///     }]
/// );
/// ```
#[derive(Debug, Default)]
pub struct DamageAccumulator {
    /// Logical rectangles added since the last clear, in the order they were
    /// added.
    rects: Vec<Rect>,
    /// Whether the whole surface must be repainted.
    full: bool,
    /// The physical rectangles produced by the last [`DamageAccumulator::finish`].
    out: Vec<PixelRect>,
}

impl DamageAccumulator {
    /// Creates an accumulator with nothing to repaint.
    pub fn new() -> DamageAccumulator {
        DamageAccumulator::default()
    }

    /// Adds a logical rectangle whose pixels must be repainted.
    ///
    /// A rectangle that covers no area is not damage and is ignored. A
    /// rectangle whose coordinates are not finite is dropped when the damage is
    /// finished, because it maps to no pixel.
    pub fn add(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        self.rects.push(rect);
    }

    /// Marks the whole surface as damaged.
    ///
    /// This overrides the rectangles added around it: the finished damage is
    /// one rectangle covering the surface.
    pub fn add_full(&mut self) {
        self.full = true;
    }

    /// Returns `true` when nothing needs repainting.
    ///
    /// Only the rectangles added so far are considered, so a rectangle that
    /// turns out to lie outside the surface still counts as damage here.
    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
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
    /// the surface.
    ///
    /// The returned slice is valid until the next call to
    /// [`DamageAccumulator::clear`], and the internal buffers are reused, so
    /// calling this on consecutive frames does not allocate.
    pub fn finish(&mut self, scale: f32, surface_width: u32, surface_height: u32) -> &[PixelRect] {
        self.out.clear();
        let Some(surface) = surface_rect(surface_width, surface_height) else {
            return &self.out;
        };
        if self.full {
            self.out.push(surface);
            return &self.out;
        }
        for rect in &self.rects {
            if let Some(pixel_rect) =
                PixelRect::from_logical(*rect, scale, surface_width, surface_height)
            {
                self.out.push(pixel_rect);
            }
        }
        self.merge(surface);
        &self.out
    }

    /// Removes every collected rectangle, keeping the allocated capacity.
    pub fn clear(&mut self) {
        self.rects.clear();
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
