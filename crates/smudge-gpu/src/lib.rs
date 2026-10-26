//! Vello-based GPU renderer for the Smudge GUI library.
//!
//! The renderer replays a display list on a GPU through [vello], always
//! redrawing the whole surface. It presents the same operations as the CPU
//! renderer, so both backends can draw one display list interchangeably.

mod renderer;

pub use renderer::GpuRenderer;
