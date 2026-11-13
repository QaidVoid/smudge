//! The renderer that draws straight into a window buffer of `0x00RRGGBB`
//! pixels and keeps no surface of its own.

use smudge_core::app::FrameOutput;
use smudge_core::geometry::PixelRect;

use crate::renderer::Painter;

/// A renderer that draws the damaged rectangles of each frame straight into a
/// buffer the caller owns, one `u32` per pixel in the `0x00RRGGBB` format
/// that window systems present.
///
/// It keeps no copy of the frame. Each damaged rectangle is painted in place
/// as premultiplied RGBA, exactly as [`crate::CpuRenderer`] paints it, and
/// then rewritten to `0x00RRGGBB` with the alpha channel dropped. A rectangle
/// reads only the pixels it has just painted, so its result equals the
/// matching pixels of a full repaint whatever the buffer held before. Pixels
/// outside the damage keep what the buffer held, so the caller passes the
/// damage that brings that buffer up to date: for a buffer that showed the
/// previous frame, the frame's own damage.
///
/// Glyph masks and the per frame plan of drawable operations survive between
/// frames, as they do in [`crate::CpuRenderer`].
///
/// # Examples
///
/// ```
/// use smudge_core::app::FrameOutput;
/// use smudge_core::geometry::{Color, PixelRect};
/// use smudge_core::paint::DisplayList;
/// use smudge_cpu::XrgbRenderer;
///
/// let mut renderer = XrgbRenderer::new(4, 4);
/// let mut buffer = vec![0u32; 4 * 4];
/// let list = DisplayList::new();
/// let damage = [PixelRect {
///     x: 0,
///     y: 0,
///     width: 4,
///     height: 4,
/// }];
///
/// renderer.render_into(
///     &FrameOutput {
///         list: &list,
///         damage: &damage,
///         full: true,
///         clear_color: Color::WHITE,
///         scale: 1.0,
///     },
///     &mut buffer,
/// );
///
/// assert!(buffer.iter().all(|&pixel| pixel == 0x00ff_ffff));
/// ```
pub struct XrgbRenderer {
    /// The buffer width in physical pixels.
    width: u32,
    /// The buffer height in physical pixels.
    height: u32,
    /// The state that survives between frames.
    painter: Painter,
}

impl XrgbRenderer {
    /// Creates a renderer for buffers of `width` by `height` physical pixels.
    pub fn new(width: u32, height: u32) -> XrgbRenderer {
        XrgbRenderer {
            width,
            height,
            painter: Painter::new(),
        }
    }

    /// Changes the size of the buffers the renderer draws into.
    ///
    /// The glyph cache survives a resize, because a mask depends on the glyph
    /// and its size but not on the buffer.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Returns the buffer width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the buffer height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Repaints the damaged rectangles of `frame` into `buffer`.
    ///
    /// `buffer` holds one `0x00RRGGBB` entry per pixel, row-major with no
    /// padding between rows. Every pixel inside a rectangle of
    /// [`FrameOutput::damage`] ends up with the color a full repaint of the
    /// frame gives it, without alpha. A non-opaque clear color therefore
    /// loses its alpha. Every other pixel is left alone.
    ///
    /// # Panics
    ///
    /// Panics when `buffer` does not hold one entry per pixel of the
    /// renderer's size, or when a damaged rectangle reaches outside it. Either
    /// means the frame or the buffer was produced for a surface of a different
    /// size, which is a broken invariant of the caller, not input from outside
    /// the library.
    pub fn render_into(&mut self, frame: &FrameOutput<'_>, buffer: &mut [u32]) {
        let (width, height) = (self.width, self.height);
        assert_eq!(
            buffer.len() as u64,
            u64::from(width) * u64::from(height),
            "the buffer must hold one entry per pixel of the {width}x{height} surface"
        );
        let pixels: &mut [u8] = bytemuck::cast_slice_mut(buffer);
        self.painter
            .paint(pixels, width, height, frame, |pixels, rect| {
                to_xrgb(pixels, width, rect);
            });
    }
}

/// Rewrites the premultiplied RGBA8 pixels of `rect` in place as
/// `0x00RRGGBB` words, dropping the alpha channel.
///
/// The conversion works on whole words, which a measurement of a 1270 by 40
/// pixel band showed 3.4 times as fast as rewriting each pixel's four bytes:
/// 5.1 against 17.2 microseconds.
fn to_xrgb(pixels: &mut [u8], width: u32, rect: PixelRect) {
    // The bytes are the caller's buffer of words seen as bytes, so they keep
    // the alignment of a word and the cast back cannot fail.
    let words: &mut [u32] = bytemuck::cast_slice_mut(pixels);
    for y in rect.y..rect.y + rect.height {
        let start = y as usize * width as usize + rect.x as usize;
        for word in &mut words[start..start + rect.width as usize] {
            let [red, green, blue, _] = word.to_ne_bytes();
            *word = u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue);
        }
    }
}

#[cfg(test)]
#[path = "xrgb_tests.rs"]
mod tests;
