//! The renderer that keeps a surface between frames and repaints only the
//! damaged rectangles of each one.

use smudge_core::app::FrameOutput;
use smudge_core::geometry::PixelRect;
use smudge_core::renderer::Renderer;
use tiny_skia::Pixmap;

use crate::draw::Canvas;
use crate::glyph_cache::GlyphCache;

/// A renderer that keeps a premultiplied RGBA surface between frames and
/// repaints only the damaged rectangles of each frame.
///
/// Every frame leaves the surface equal to a full repaint of the display list.
/// Each damaged rectangle is cleared and redrawn in place, and every other
/// pixel is left untouched. A pixel's value depends only on its position and
/// the operations covering it, so redrawing a rectangle gives the same bytes a
/// full repaint would.
///
/// Glyphs are drawn from a cache of alpha masks that survives between frames
/// with the surface, so a glyph is rasterized once no matter how often it is
/// repainted.
///
/// Drawing needs no buffer of its own, so a frame whose glyphs are all in the
/// cache allocates nothing. A glyph seen for the first time allocates its mask.
///
/// # Examples
///
/// ```
/// use smudge_core::app::FrameOutput;
/// use smudge_core::geometry::{Color, PixelRect};
/// use smudge_core::paint::DisplayList;
/// use smudge_cpu::CpuRenderer;
///
/// let mut renderer = CpuRenderer::new(4, 4);
/// let list = DisplayList::new();
/// let damage = [PixelRect {
///     x: 0,
///     y: 0,
///     width: 4,
///     height: 4,
/// }];
///
/// renderer.render(&FrameOutput {
///     list: &list,
///     damage: &damage,
///     full: true,
///     clear_color: Color::WHITE,
///     scale: 1.0,
/// });
///
/// assert_eq!(renderer.pixels().len(), 4 * 4 * 4);
/// assert_eq!(&renderer.pixels()[..4], &[255, 255, 255, 255]);
/// ```
pub struct CpuRenderer {
    /// The surface that survives between frames, or `None` when the surface
    /// has no pixel.
    pixmap: Option<Pixmap>,
    /// The glyph masks that survive between frames.
    glyphs: GlyphCache,
}

impl CpuRenderer {
    /// Creates a renderer for a surface of `width` by `height` physical pixels.
    ///
    /// The surface starts empty. A surface with a zero extent holds no pixels,
    /// and [`CpuRenderer::width`], [`CpuRenderer::height`],
    /// [`CpuRenderer::pixels`] and [`CpuRenderer::encode_png`] then report an
    /// empty surface.
    pub fn new(width: u32, height: u32) -> CpuRenderer {
        CpuRenderer {
            pixmap: Pixmap::new(width, height),
            glyphs: GlyphCache::new(),
        }
    }

    /// Replaces the surface with one of `width` by `height` physical pixels.
    ///
    /// The contents are undefined until the next frame that damages the whole
    /// surface, so callers resize only when they also force a full repaint.
    /// The glyph cache survives a resize, because a mask depends on the glyph
    /// and its size but not on the surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.pixmap = Pixmap::new(width, height);
    }

    /// Repaints the damaged rectangles of `frame` into the surface.
    ///
    /// Each rectangle of [`FrameOutput::damage`] is rendered separately, so the
    /// pixels outside every rectangle keep the value the previous frame left
    /// them with. The damage already covers everything that changed, so
    /// rendering it produces the same surface a full repaint of the frame would.
    ///
    /// Only the damage is read. [`FrameOutput::full`] does not clear the
    /// surface by itself, and a full frame from the app already damages the
    /// whole surface.
    ///
    /// # Panics
    ///
    /// Panics when a damaged rectangle reaches outside the renderer's surface,
    /// which means the frame was produced for a surface of a different size.
    /// That is a broken invariant of the caller, not input from outside the
    /// library.
    pub fn render(&mut self, frame: &FrameOutput<'_>) {
        let width = self.width();
        let height = self.height();
        for rect in frame.damage {
            assert!(
                inside_surface(*rect, width, height),
                "damage {rect:?} reaches outside the {width}x{height} surface, \
                 so the frame was produced for a different surface size"
            );
        }
        let Some(pixmap) = self.pixmap.as_mut() else {
            return;
        };

        let mut canvas = Canvas {
            pixels: pixmap.data_mut(),
            width,
            height,
            scale: frame.scale,
            glyphs: &mut self.glyphs,
        };
        for &rect in frame.damage {
            canvas.clear(rect, frame.clear_color);
            canvas.replay(frame.list.items(), rect);
        }
    }

    /// Returns the surface width in physical pixels.
    pub fn width(&self) -> u32 {
        self.pixmap.as_ref().map_or(0, Pixmap::width)
    }

    /// Returns the surface height in physical pixels.
    pub fn height(&self) -> u32 {
        self.pixmap.as_ref().map_or(0, Pixmap::height)
    }

    /// Returns the surface as premultiplied RGBA8 pixels, row-major with no
    /// padding between rows.
    ///
    /// The slice is empty when the surface has no pixel.
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.as_ref().map_or(&[], Pixmap::data)
    }

    /// Encodes the surface as a PNG image.
    ///
    /// The result is empty when the surface has no pixel, because an image
    /// cannot have a zero extent.
    ///
    /// # Panics
    ///
    /// Panics when the PNG encoder fails. Encoding into memory cannot fail, so
    /// this reports a broken invariant of the encoder rather than anything a
    /// caller can provoke.
    pub fn encode_png(&self) -> Vec<u8> {
        let Some(pixmap) = &self.pixmap else {
            return Vec::new();
        };
        pixmap
            .encode_png()
            .expect("encoding a surface into memory cannot fail")
    }
}

/// Returns `true` when every pixel of `rect` lies inside a surface of `width`
/// by `height` pixels.
fn inside_surface(rect: PixelRect, width: u32, height: u32) -> bool {
    let right = u64::from(rect.x) + u64::from(rect.width);
    let bottom = u64::from(rect.y) + u64::from(rect.height);
    right <= u64::from(width) && bottom <= u64::from(height)
}

impl Renderer for CpuRenderer {
    fn render(&mut self, frame: &FrameOutput<'_>) {
        CpuRenderer::render(self, frame);
    }
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
