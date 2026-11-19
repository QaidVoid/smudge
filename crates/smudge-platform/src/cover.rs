//! Disjoint rectangles that cover the same pixels as a set that may overlap,
//! so the damage tint marks each repainted pixel once.

use smudge_core::geometry::PixelRect;

/// Turns rectangles that may overlap into disjoint ones with the same pixels,
/// keeping its buffers from one call to the next.
#[derive(Debug, Default)]
pub(crate) struct Cover {
    /// The top and bottom edges of the rectangles, sorted.
    edges: Vec<u32>,
    /// The horizontal spans of one band, as starts and ends.
    spans: Vec<(u32, u32)>,
    /// The disjoint rectangles of the last call.
    rects: Vec<PixelRect>,
}

impl Cover {
    /// Returns disjoint rectangles that cover exactly the pixels of `rects`.
    ///
    /// The rows are cut into bands at every top and bottom edge. In each
    /// band, the spans of the rectangles that cross it are merged where they
    /// overlap or touch, and each merged span becomes one rectangle.
    pub(crate) fn of(&mut self, rects: &[PixelRect]) -> &[PixelRect] {
        self.rects.clear();
        self.edges.clear();
        for rect in rects
            .iter()
            .filter(|rect| rect.width > 0 && rect.height > 0)
        {
            self.edges.push(rect.y);
            self.edges.push(rect.y + rect.height);
        }
        self.edges.sort_unstable();
        self.edges.dedup();
        for band in self.edges.windows(2) {
            let (top, bottom) = (band[0], band[1]);
            self.spans.clear();
            self.spans.extend(
                rects
                    .iter()
                    .filter(|rect| rect.width > 0 && rect.y <= top && top < rect.y + rect.height)
                    .map(|rect| (rect.x, rect.x + rect.width)),
            );
            self.spans.sort_unstable();
            let mut spans = self.spans.iter().copied();
            let Some(mut open) = spans.next() else {
                continue;
            };
            for (start, end) in spans.chain([(u32::MAX, u32::MAX)]) {
                if start <= open.1 {
                    open.1 = open.1.max(end);
                    continue;
                }
                self.rects.push(PixelRect {
                    x: open.0,
                    y: top,
                    width: open.1 - open.0,
                    height: bottom - top,
                });
                open = (start, end);
            }
        }
        &self.rects
    }
}

#[cfg(test)]
#[path = "cover_tests.rs"]
mod tests;
