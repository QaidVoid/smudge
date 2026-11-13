//! Window creation, event translation and frame presentation for desktop and
//! web targets.
//!
//! This crate is the only place that talks to the operating system's windowing
//! layer. It owns the run loop and hands each frame to the chosen renderer.
//!
//! Its only unsafe code is the hot reload receiver, compiled with the
//! `hot-reload` feature on Linux, which loads the patch libraries a patch
//! builder sends. Every other build forbids unsafe code.

#![cfg_attr(
    not(all(feature = "hot-reload", target_os = "linux")),
    forbid(unsafe_code)
)]
#![cfg_attr(all(feature = "hot-reload", target_os = "linux"), deny(unsafe_code))]

#[cfg(not(target_arch = "wasm32"))]
mod clipboard;
mod error;
mod events;
#[cfg(feature = "gpu")]
mod gpu;
// Loading a patch library and calling the patched view are unsafe by nature,
// and the receiver is the one module that does either.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[allow(unsafe_code)]
mod hot;
mod present;
mod renderer;
mod window;

pub use error::PlatformError;
pub use present::{PresentStats, PresentStatsHandle};
pub use renderer::{RENDERER_ENVIRONMENT, RendererChoice};
pub use window::{EventHook, FrameHook, WindowOptions, run, run_view, run_with};

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
pub use hot::{HOT_BUILD_ENVIRONMENT, HOT_SOCKET_ENVIRONMENT};

/// The GPU adapter a window presents through, as plain strings.
///
/// The platform reports one through [`PresentStatsHandle::adapter`] when it
/// sets the GPU presentation path up.
#[cfg(feature = "gpu")]
pub use smudge_gpu::AdapterSummary;
