//! Window creation, event translation and frame presentation for desktop and
//! web targets.
//!
//! This crate is the only place that talks to the operating system's windowing
//! layer. It owns the run loop and hands each frame to the chosen renderer.
