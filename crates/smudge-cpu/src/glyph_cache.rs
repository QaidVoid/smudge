//! A bounded cache of rasterized glyph alpha masks.
//!
//! Outlining a glyph and filling its path is the expensive part of drawing
//! text, so each glyph is rasterized once into an 8-bit alpha mask and blitted
//! from the cache afterwards. A mask depends on the glyph, its quantized size
//! and its subpixel x phase only, never on the area being drawn, so a damaged
//! repaint blits the same bytes a full repaint would.
//!
//! Rasterization is unhinted, at the exact quantized size, with grayscale
//! anti-aliasing only. y is snapped to whole physical pixels and x keeps a
//! quarter-pixel phase, which keeps vertical stems steady without LCD
//! filtering.

use std::collections::HashMap;

use skrifa::GlyphId;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::pen::PathElement;
use skrifa::outline::{DrawSettings, OutlinePen};
use smudge_core::paint::FontBlob;
use tiny_skia::{FillRule, Mask, PathBuilder, Transform};

/// The largest font size a glyph is rasterized at, in physical pixels.
///
/// A mask is as tall and wide as its glyph's ink, so the size bounds the
/// memory one mask can take. Text larger than this draws nothing.
pub(crate) const MAX_GLYPH_SIZE: f32 = 1024.0;

/// The widest or tallest mask that is rasterized, in pixels.
///
/// A glyph's ink can reach past its em box, so this sits above
/// [`MAX_GLYPH_SIZE`] rather than matching it. Anything wider or taller draws
/// nothing rather than allocating a mask of unbounded size.
const MAX_MASK_DIMENSION: u32 = 4096;

/// The memory the cache holds in its current generation before the older one
/// is dropped, in bytes of mask.
///
/// The previous generation can hold as much again, so the cache peaks at about
/// twice this budget.
const DEFAULT_BUDGET: usize = 16 * 1024 * 1024;

/// The key a glyph mask is cached under.
///
/// The subpixel bin is the quarter-pixel phase of the glyph's x origin, so a
/// glyph needs one mask per phase and none for y, which is snapped to whole
/// physical pixels.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MaskKey {
    /// The identifier of the font bytes, as [`FontBlob`] reports it.
    pub(crate) font_id: u64,
    /// The face inside the font file.
    pub(crate) font_index: u32,
    /// The glyph inside the face.
    pub(crate) glyph_id: u32,
    /// The font size in 1/64 physical pixels.
    pub(crate) size: i32,
    /// The quarter-pixel phase of the x origin, from 0 to 3.
    pub(crate) subpixel: u32,
}

/// One rasterized glyph: its alpha mask and where the mask sits relative to
/// the glyph's baseline origin, in physical pixels.
///
/// The bearings can be negative, which happens when the ink reaches left of
/// the origin or above the baseline.
pub(crate) struct GlyphMask {
    /// Coverage from 0 to 255, row-major with no padding.
    data: Vec<u8>,
    /// The mask width in pixels.
    width: u32,
    /// The mask height in pixels.
    height: u32,
    /// Distance from the glyph's x origin to the mask's left edge.
    left: i32,
    /// Distance from the glyph's baseline y to the mask's top edge.
    top: i32,
}

impl GlyphMask {
    /// Returns the mask width in pixels.
    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    /// Returns the mask height in pixels.
    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    /// Returns the distance from the glyph's x origin to the mask's left edge.
    pub(crate) fn left(&self) -> i32 {
        self.left
    }

    /// Returns the distance from the glyph's baseline y to the mask's top
    /// edge.
    pub(crate) fn top(&self) -> i32 {
        self.top
    }

    /// Returns the coverage bytes, row-major with no padding.
    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    /// The memory the mask occupies, in bytes.
    fn size_in_bytes(&self) -> usize {
        self.data.len()
    }

    /// A mask with no pixels, for a glyph whose outline is empty.
    fn empty() -> GlyphMask {
        GlyphMask {
            data: Vec::new(),
            width: 0,
            height: 0,
            left: 0,
            top: 0,
        }
    }
}

/// A cache of glyph masks that keeps two generations.
///
/// The current generation holds recently rasterized masks. When it grows past
/// the budget, it becomes the previous generation and a fresh current
/// generation starts, so the masks of an older frame are dropped as a whole.
/// A mask found in the previous generation moves back to the current one,
/// which keeps glyphs that every frame uses alive however many frames pass.
pub(crate) struct GlyphCache {
    current: HashMap<MaskKey, GlyphMask>,
    previous: HashMap<MaskKey, GlyphMask>,
    /// The bytes of mask the current generation holds.
    bytes: usize,
    budget: usize,
    hits: u64,
    misses: u64,
}

impl GlyphCache {
    /// Creates a cache with the default memory budget.
    pub(crate) fn new() -> GlyphCache {
        GlyphCache::with_budget(DEFAULT_BUDGET)
    }

    /// Creates a cache that keeps at most `budget` bytes of mask in its
    /// current generation.
    ///
    /// One mask larger than the whole budget is still cached, because a glyph
    /// drawn every frame is worth more than the memory it takes.
    pub(crate) fn with_budget(budget: usize) -> GlyphCache {
        GlyphCache {
            current: HashMap::new(),
            previous: HashMap::new(),
            bytes: 0,
            budget,
            hits: 0,
            misses: 0,
        }
    }

    /// Returns the mask for `key`, rasterizing it from `font` on a miss.
    ///
    /// Returns `None` when the glyph has no mask because its font cannot be
    /// read or its size is out of range. A glyph with an empty outline yields
    /// an empty mask, so repeated draws of a space hit the cache too.
    pub(crate) fn entry(&mut self, key: MaskKey, font: &FontBlob) -> Option<&GlyphMask> {
        if !self.current.contains_key(&key) {
            if let Some(mask) = self.previous.remove(&key) {
                self.bytes += mask.size_in_bytes();
                self.current.insert(key, mask);
                self.hits += 1;
            } else {
                self.misses += 1;
                let mask = rasterize(key, font)?;
                if self.bytes + mask.size_in_bytes() > self.budget {
                    self.previous = std::mem::take(&mut self.current);
                    self.bytes = 0;
                }
                self.bytes += mask.size_in_bytes();
                self.current.insert(key, mask);
            }
        } else {
            self.hits += 1;
        }
        self.current.get(&key)
    }

    /// Returns the bytes of mask the current generation holds.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Returns how many draws found a cached mask.
    #[cfg(test)]
    pub(crate) fn hits(&self) -> u64 {
        self.hits
    }

    /// Returns how many draws had to rasterize a mask.
    #[cfg(test)]
    pub(crate) fn misses(&self) -> u64 {
        self.misses
    }
}

/// Rasterizes one glyph into an alpha mask.
///
/// The outline is drawn unhinted at the key's quantized size, flipped for a
/// coordinate system whose y grows down, and shifted by the key's subpixel
/// phase. The mask spans the outline's bounds with one pixel of slack on
/// every side, so every anti-aliased edge pixel it can shade is inside the
/// mask, and the bearings report where that rectangle sits relative to the
/// glyph's baseline origin.
fn rasterize(key: MaskKey, font: &FontBlob) -> Option<GlyphMask> {
    if key.size <= 0 || key.size as f32 / 64.0 > MAX_GLYPH_SIZE {
        return None;
    }
    let data = font.data.as_ref().as_ref();
    let font = skrifa::FontRef::from_index(data, key.font_index).ok()?;
    let outline = font.outline_glyphs().get(GlyphId::new(key.glyph_id))?;

    let mut collector = OutlineCollector::default();
    let settings =
        DrawSettings::unhinted(Size::new(key.size as f32 / 64.0), LocationRef::default());
    outline.draw(settings, &mut collector).ok()?;

    let Some((x_min, y_min, x_max, y_max)) = collector.bounds() else {
        return Some(GlyphMask::empty());
    };
    let shift = key.subpixel as f32 / 4.0;
    let x0 = (x_min + shift).floor() - 1.0;
    let y0 = y_min.floor() - 1.0;
    let width = ((x_max + shift).ceil() + 1.0 - x0) as u32;
    let height = (y_max.ceil() + 1.0 - y0) as u32;
    if width == 0 || height == 0 || width > MAX_MASK_DIMENSION || height > MAX_MASK_DIMENSION {
        return None;
    }

    let mut builder = PathBuilder::new();
    for element in &collector.elements {
        match *element {
            PathElement::MoveTo { x, y } => builder.move_to(x + shift - x0, y - y0),
            PathElement::LineTo { x, y } => builder.line_to(x + shift - x0, y - y0),
            PathElement::QuadTo { cx0, cy0, x, y } => {
                builder.quad_to(cx0 + shift - x0, cy0 - y0, x + shift - x0, y - y0);
            }
            PathElement::CurveTo {
                cx0,
                cy0,
                cx1,
                cy1,
                x,
                y,
            } => builder.cubic_to(
                cx0 + shift - x0,
                cy0 - y0,
                cx1 + shift - x0,
                cy1 - y0,
                x + shift - x0,
                y - y0,
            ),
            PathElement::Close => builder.close(),
        }
    }
    let Some(path) = builder.finish() else {
        return Some(GlyphMask::empty());
    };
    let mut mask = Mask::new(width, height)?;
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(GlyphMask {
        data: mask.data().to_vec(),
        width,
        height,
        left: x0 as i32,
        top: y0 as i32,
    })
}

/// Collects the elements of one outline in physical pixels, flipped for a
/// coordinate system whose y grows down.
#[derive(Default)]
struct OutlineCollector {
    /// The path elements collected so far.
    elements: Vec<PathElement>,
}

impl OutlineCollector {
    /// Returns the bounds of every point the collected elements mention,
    /// including control points, or `None` when nothing was collected.
    fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        let fold = |bounds: &mut Option<(f32, f32, f32, f32)>, x: f32, y: f32| {
            *bounds = Some(match *bounds {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        };
        for element in &self.elements {
            match *element {
                PathElement::MoveTo { x, y } | PathElement::LineTo { x, y } => {
                    fold(&mut bounds, x, y);
                }
                PathElement::QuadTo { cx0, cy0, x, y, .. } => {
                    fold(&mut bounds, cx0, cy0);
                    fold(&mut bounds, x, y);
                }
                PathElement::CurveTo {
                    cx0,
                    cy0,
                    cx1,
                    cy1,
                    x,
                    y,
                } => {
                    fold(&mut bounds, cx0, cy0);
                    fold(&mut bounds, cx1, cy1);
                    fold(&mut bounds, x, y);
                }
                PathElement::Close => {}
            }
        }
        bounds
    }
}

impl OutlinePen for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.elements.push(PathElement::MoveTo { x, y: -y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.elements.push(PathElement::LineTo { x, y: -y });
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.elements.push(PathElement::QuadTo {
            cx0,
            cy0: -cy0,
            x,
            y: -y,
        });
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.elements.push(PathElement::CurveTo {
            cx0,
            cy0: -cy0,
            cx1,
            cy1: -cy1,
            x,
            y: -y,
        });
    }

    fn close(&mut self) {
        self.elements.push(PathElement::Close);
    }
}

#[cfg(test)]
#[path = "glyph_cache_tests.rs"]
mod tests;
