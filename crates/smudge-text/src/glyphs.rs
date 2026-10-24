//! Conversion of a laid out text block into the glyph runs paint operations
//! carry.
//!
//! The conversion turns parley's positioned glyphs into the library's own
//! [`GlyphRun`], which carries the font as plain bytes so that paint
//! operations never depend on parley. Each run's ink bounds are measured with
//! skrifa and cover every glyph's outline plus one logical pixel on every
//! side for anti-aliasing, which is the area renderers may rely on for damage.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::layout::TextLayout;
use parley::FontData;
use skrifa::GlyphId;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use smudge_core::geometry::{Color, Point, Rect};
use smudge_core::paint::{FontBlob, GlyphRun, PositionedGlyph};

thread_local! {
    /// The font blobs already mapped out of parley's font data, keyed by the
    /// blob identifier and face index. One entry per font keeps every run of a
    /// layout sharing the bytes, and keeps the identifier stable so renderers
    /// can cache glyphs under it.
    ///
    /// Entries hold the font bytes alive for the life of the thread. Fonts are
    /// few and outlive layouts in practice, so nothing collects them.
    static FONT_BLOBS: RefCell<HashMap<(u64, u32), FontBlob>> = RefCell::new(HashMap::new());
}

impl TextLayout {
    /// Converts the laid out text into glyph runs at `origin`, in paint order.
    ///
    /// `origin` is where the top left corner of the layout box goes, in
    /// logical window coordinates, so the first line's baseline lies one
    /// ascent below it. The layout itself carries no color, so `color` is the
    /// color every returned run paints with, resolved by the caller at paint
    /// time. Each run carries the font as bytes, the size its style used, the
    /// given color, and glyphs positioned in absolute logical window
    /// coordinates. A run whose text is empty produces no runs.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`crate::TextSystem::with`], in
    /// particular when called from inside another `with` call on the same
    /// thread.
    pub fn glyph_runs_at(&self, origin: Point, color: Color) -> Vec<GlyphRun> {
        self.glyph_runs()
            .map(|run| {
                let parley_run = run.run();
                let font = font_blob(parley_run.font());
                let size = parley_run.font_size();
                // skrifa reads the font once per run, not once per glyph.
                let metrics = skrifa::FontRef::from_index(
                    parley_run.font().data.as_ref(),
                    parley_run.font().index,
                )
                .ok()
                .map(|font| font.glyph_metrics(Size::new(size), LocationRef::default()));
                let mut glyphs = Vec::new();
                let mut ink = Rect::default();
                for glyph in run.positioned_glyphs() {
                    let position = PositionedGlyph {
                        id: glyph.id,
                        x: origin.x + glyph.x,
                        y: origin.y + glyph.y,
                    };
                    if let Some(bounds) = metrics
                        .as_ref()
                        .and_then(|metrics| metrics.bounds(GlyphId::new(glyph.id)))
                    {
                        // Font coordinates grow up, window coordinates down.
                        ink = ink.union(Rect::new(
                            position.x + bounds.x_min,
                            position.y - bounds.y_max,
                            position.x + bounds.x_max,
                            position.y - bounds.y_min,
                        ));
                    }
                    glyphs.push(position);
                }
                GlyphRun {
                    font,
                    size,
                    color,
                    glyphs,
                    ink_bounds: expand(ink, 1.0),
                }
            })
            .collect()
    }
}

/// Returns the [`FontBlob`] for parley's font data, reusing the blob the font
/// collection already holds.
fn font_blob(font: &FontData) -> FontBlob {
    FONT_BLOBS.with(|cache| {
        cache
            .borrow_mut()
            .entry((font.data.id(), font.index))
            .or_insert_with(|| {
                let (data, id) = font.data.clone().into_raw_parts();
                FontBlob {
                    id,
                    data,
                    index: font.index,
                }
            })
            .clone()
    })
}

/// Grows a non-empty rectangle by `by` pixels on every side.
fn expand(rect: Rect, by: f32) -> Rect {
    if rect.is_empty() {
        rect
    } else {
        Rect::new(rect.x0 - by, rect.y0 - by, rect.x1 + by, rect.y1 + by)
    }
}

#[cfg(test)]
#[path = "glyphs_tests.rs"]
mod tests;
