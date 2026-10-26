//! The trait every renderer implements.
//!
//! A renderer receives a finished [`FrameOutput`] and is responsible for the
//! pixels it produces. The core knows nothing beyond the trait, so the CPU
//! and GPU backends stay interchangeable wherever only this contract matters.

use crate::app::FrameOutput;

/// Draws one frame of a [`DisplayList`](crate::paint::DisplayList) onto a
/// surface the renderer owns.
///
/// The method takes `self` mutably because renderers keep state between
/// frames, such as the CPU renderer's persistent surface or the GPU renderer's
/// texture. A renderer that redraws everything each frame may ignore
/// [`FrameOutput::damage`], but it must still honor [`FrameOutput::full`] and
/// [`FrameOutput::clear_color`] so that both backends show the same frame from
/// the same output.
///
/// # Panics
///
/// A renderer may panic when the frame was produced for a surface of a
/// different size than the renderer holds, which is a broken invariant of the
/// caller rather than input from outside the library.
pub trait Renderer {
    /// Repaints `frame` onto the renderer's surface.
    fn render(&mut self, frame: &FrameOutput<'_>);
}
