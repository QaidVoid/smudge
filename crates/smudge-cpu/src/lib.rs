//! CPU rasterizer that keeps a persistent pixmap and repaints only the damaged
//! regions of each frame.
//!
//! The rasterizer replays a display list into a premultiplied RGBA surface that
//! survives between frames, so untouched pixels cost nothing.

#![forbid(unsafe_code)]
