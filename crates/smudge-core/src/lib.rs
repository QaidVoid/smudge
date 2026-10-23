//! Geometry, the element builder, the widget tree, style, layout, paint,
//! damage tracking and the frame pipeline.
//!
//! Everything in this crate is independent of a window, a GPU or a rasterizer,
//! so it builds for any target the standard library supports.

#![forbid(unsafe_code)]

pub mod damage;
pub mod element;
pub mod geometry;
pub mod paint;
pub mod reactive;
pub mod style;
pub mod tree;
pub mod widget;
