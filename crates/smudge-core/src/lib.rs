//! Geometry, the element builder, the widget tree, style, layout, paint,
//! damage tracking and the frame pipeline.
//!
//! Everything in this crate is independent of a window, a GPU or a rasterizer,
//! so it builds for any target the standard library supports.

#![cfg_attr(all(feature = "hot-reload", target_os = "linux"), deny(unsafe_code))]
#![cfg_attr(
    not(all(feature = "hot-reload", target_os = "linux")),
    forbid(unsafe_code)
)]

pub mod app;
pub mod channel;
pub mod clipboard;
mod damage;
pub mod element;
pub mod geometry;
/// Hot reload of the closures the tree stores, for the facade to re-export.
pub mod hot;
pub mod image;
pub mod input;
pub mod paint;
pub mod reactive;
pub mod renderer;
pub mod semantics;
pub mod style;
pub mod theme;
pub mod timer;
pub mod tree;
pub mod variants;
pub mod widget;
pub mod window;

pub use app::{App, AppConfig, FrameOutput, FrameStats, RepaintMode};
