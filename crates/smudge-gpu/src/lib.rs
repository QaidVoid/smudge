//! Vello-based GPU renderer for the Smudge GUI library.
//!
//! The renderer replays a display list on a GPU through [vello], always
//! redrawing the whole surface. It presents the same operations as the CPU
//! renderer, so both backends can draw one display list interchangeably.

mod encoder;
mod renderer;
mod surface;

pub use renderer::GpuRenderer;
pub use surface::{GpuError, GpuSurface};

/// The wgpu that vello renders through, re-exported so a caller can build its
/// instances, adapters and surfaces against the same version the renderers
/// use.
pub use vello::wgpu;
