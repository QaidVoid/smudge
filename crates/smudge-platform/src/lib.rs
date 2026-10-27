//! Window creation, event translation and frame presentation for desktop and
//! web targets.
//!
//! This crate is the only place that talks to the operating system's windowing
//! layer. It owns the run loop and hands each frame to the chosen renderer.

#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod clipboard;
mod error;
mod events;
#[cfg(feature = "gpu")]
mod gpu;
mod present;
mod renderer;
mod window;

pub use error::PlatformError;
pub use present::{PresentStats, PresentStatsHandle};
pub use renderer::{RENDERER_ENVIRONMENT, RendererChoice};
pub use window::{FrameHook, WindowOptions, run, run_with};
