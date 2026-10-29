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

use parley::FontData;
use parley::PositionedLayoutItem;
use skrifa::GlyphId;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use smudge_core::geometry::{Color, Point, Rect};
use smudge_core::paint::{FontBlob, GlyphRun, PositionedGlyph};

use crate::layout::Cut;
use crate::layout::Ellipsis;
use crate::layout::TextLayout;
use crate::layout::Truncation;

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
    /// `origin` is where the top left corner of the layout box goes, in the
    /// caller's paint coordinates, which leave out the scroll offsets of
    /// enclosing views, so the first line's baseline lies one ascent below
    /// it. The layout itself carries no color, so `color` is the
    /// color every returned run paints with, resolved by the caller at paint
    /// time. Each run carries the font as bytes, the size its style used, the
    /// given color, and glyphs positioned in those same coordinates. A run
    /// whose text is empty produces no runs.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`crate::TextSystem::with`], in
    /// particular when called from inside another `with` call on the same
    /// thread.
    pub fn glyph_runs_at(&self, origin: Point, color: Color) -> Vec<GlyphRun> {
        glyph_runs_with(&self.layout, origin, color, self.truncation.as_ref())
    }
}

/// Converts the glyph runs of any laid out text into paint operations' runs.
///
/// The conversion is shared with the text input, which paints a parley
/// [`Layout`](parley::Layout) it drives through the editor rather than a
/// [`TextLayout`] it laid out itself, and truncates nothing.
pub(crate) fn glyph_runs<B: parley::Brush>(
    layout: &parley::Layout<B>,
    origin: Point,
    color: Color,
) -> Vec<GlyphRun> {
    glyph_runs_with(layout, origin, color, None)
}

/// Converts the glyph runs of any laid out text into paint operations' runs,
/// cut to `truncation`.
///
/// `truncation` limits the conversion to a truncation's kept lines and cuts
/// the last of them: the runs before the cut survive whole, the cut run
/// keeps its leading glyphs, and the ellipsis follows them in a run of its
/// own, since its font can differ from the cut run's.
pub(crate) fn glyph_runs_with<B: parley::Brush>(
    layout: &parley::Layout<B>,
    origin: Point,
    color: Color,
    truncation: Option<&Truncation>,
) -> Vec<GlyphRun> {
    let kept_lines = truncation.map_or(usize::MAX, |truncation| truncation.kept_lines);
    let mut runs = Vec::new();
    for (line_index, line) in layout.lines().enumerate().take(kept_lines) {
        let cut = match truncation {
            Some(truncation) if line_index + 1 == truncation.kept_lines => truncation.cut.as_ref(),
            _ => None,
        };
        let mut run_index = 0usize;
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let kept = match cut {
                Some(cut) if run_index > cut.run => break,
                Some(cut) if run_index == cut.run => cut.glyphs,
                _ => usize::MAX,
            };
            run_index += 1;
            if kept == 0 {
                continue;
            }
            let glyphs = run
                .positioned_glyphs()
                .take(kept)
                .map(|glyph| (glyph.id, origin.x + glyph.x, origin.y + glyph.y));
            runs.push(paint_run(
                run.run().font(),
                run.run().font_size(),
                glyphs,
                color,
            ));
        }
        if let Some(Cut {
            x,
            baseline,
            ellipsis: Some(ellipsis),
            ..
        }) = cut
        {
            let y = origin.y + baseline;
            let dots = (0..ellipsis.dots).map(|dot| {
                let dot_x = origin.x + x + dot as f32 * ellipsis.advance;
                (ellipsis.glyph, dot_x, y)
            });
            runs.push(paint_run(&ellipsis.font, ellipsis.size, dots, color));
        }
    }
    runs
}

/// Builds one [`GlyphRun`] out of the glyph identifiers and absolute pen
/// positions `glyphs` yields, painted with `font` at `size`.
///
/// Every glyph's ink is measured with skrifa against that font and size.
fn paint_run(
    font: &FontData,
    size: f32,
    glyphs: impl Iterator<Item = (u32, f32, f32)>,
    color: Color,
) -> GlyphRun {
    // skrifa reads the font once per run, not once per glyph.
    let metrics = skrifa::FontRef::from_index(font.data.as_ref(), font.index)
        .ok()
        .map(|font| font.glyph_metrics(Size::new(size), LocationRef::default()));
    let mut painted = Vec::new();
    let mut ink = Rect::default();
    for (id, x, y) in glyphs {
        if let Some(bounds) = metrics
            .as_ref()
            .and_then(|metrics| metrics.bounds(GlyphId::new(id)))
        {
            // Font coordinates grow up, window coordinates down.
            ink = ink.union(Rect::new(
                x + bounds.x_min,
                y - bounds.y_max,
                x + bounds.x_max,
                y - bounds.y_min,
            ));
        }
        painted.push(PositionedGlyph { id, x, y });
    }
    GlyphRun {
        font: font_blob(font),
        size,
        color,
        glyphs: painted,
        ink_bounds: expand(ink, 1.0),
    }
}

/// Resolves the ellipsis glyph truncation appends, measured with the font
/// and size of the run it ends.
///
/// The horizontal ellipsis stands in for the cut text, or three full stops
/// when the font maps no ellipsis. Returns `None` when the font maps
/// neither, so truncation looks for the ellipsis elsewhere.
pub(crate) fn ellipsis_glyph(font: &FontData, font_size: f32) -> Option<Ellipsis> {
    let face = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
    let charmap = face.charmap();
    let (glyph, dots) = match charmap.map('\u{2026}') {
        Some(id) => (id.to_u32(), 1),
        None => (charmap.map('.')?.to_u32(), 3),
    };
    let metrics = face.glyph_metrics(Size::new(font_size), LocationRef::default());
    let advance = metrics.advance_width(GlyphId::new(glyph)).unwrap_or(0.0);
    Some(Ellipsis {
        font: font.clone(),
        size: font_size,
        glyph,
        dots,
        advance,
    })
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
