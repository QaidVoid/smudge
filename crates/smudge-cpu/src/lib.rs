//! CPU rasterizer that keeps a persistent pixmap and repaints only the damaged
//! regions of each frame.
//!
//! The rasterizer replays a display list into a premultiplied RGBA surface that
//! survives between frames, so untouched pixels cost nothing. Each damaged
//! rectangle is cleared and redrawn in place, with drawing clipped to it.
//! Glyphs are drawn from a cache of alpha masks that rasterizes each glyph
//! once per size and subpixel phase.
//!
//! Redrawing a rectangle on its own produces the same pixels as a full
//! repaint, because each pixel's coverage is computed from its absolute
//! position and the shape covering it, not from the area being drawn.

#![forbid(unsafe_code)]

mod draw;
mod glyph_cache;
mod renderer;

pub use renderer::CpuRenderer;
