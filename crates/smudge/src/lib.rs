//! A native Rust GUI library with a retained widget tree, fine-grained
//! reactive signals and damage-tracked repainting, built to stay fast on
//! machines without a usable GPU.
//!
//! # Quick start
//!
//! A counter: a button and a label bound to one signal. Each press writes the
//! signal, the label's text closure reruns, and the next frame repaints only
//! the label.
//!
//! ```no_run
//! use smudge::prelude::*;
//!
//! fn main() -> Result<(), PlatformError> {
//!     let count = RwSignal::new(0_u32);
//!     let press = count.clone();
//!     let shown = count.clone();
//!
//!     run(
//!         WindowOptions {
//!             title: String::from("Counter"),
//!             size: Size {
//!                 width: 280.0,
//!                 height: 140.0,
//!             },
//!             clear_color: Color::rgb(16, 17, 21),
//!             repaint: Default::default(),
//!             show_damage: false,
//!             renderer: RendererChoice::default(),
//!             stats: None,
//!             on_frame: None,
//!             on_event: None,
//!             frame_limit: None,
//!         },
//!         move || {
//!             column()
//!                 .padding(px(24.0))
//!                 .gap(Size {
//!                     width: 0.0,
//!                     height: 12.0,
//!                 })
//!                 .align_items(Align::Center)
//!                 .child(
//!                     button("Count up")
//!                         .primary()
//!                         .on_press(move || press.update(|count| *count += 1)),
//!                 )
//!                 .child(label(move || format!("Clicked {} times", shown.get())))
//!         },
//!     )
//! }
//! ```
//!
//! The runnable versions of this and two larger applications live in
//! `crates/smudge/examples`. The same application also runs headless:
//! [`core::App`] takes the build closure, and its `frame` method advances it
//! one frame at a time, which is what tests and benchmarks drive.
//!
//! # The frame model
//!
//! Application state lives in signals ([`core::reactive::RwSignal`]). Styling
//! a widget with a closure, or handing a closure to [`text::label`], binds
//! that part of the widget to the signals the closure reads. A write to a
//! signal touches no widget directly. At the next `App::frame`, the rerun
//! bindings queue their updates, layout reruns for the nodes the updates
//! marked, the changed widgets regenerate their paint operations, and the
//! pipeline subtracts what stayed the same. What is left is the frame's
//! damage, the physical-pixel rectangles the frame changed, and the renderer
//! repaints exactly that onto the previous frame's pixels. A frame that
//! changes one small box rasterizes and presents one small box;
//! [`core::RepaintMode::Full`] turns the subtraction off when a
//! whole-surface redraw is wanted instead.
//!
//! # Platform support
//!
//! - **Desktop:** the CPU renderer presents through the window's buffer. With
//!   the `gpu` feature, a vello renderer draws the same display list on the
//!   GPU, and [`window::RendererChoice::Auto`], the default, uses the GPU
//!   only when the adapter is a real GPU, falling back to the CPU renderer on
//!   a software rasterizer.
//! - **Web:** the CPU renderer runs in a browser on `wasm32-unknown-unknown`;
//!   `crates/smudge-platform/examples/web` holds a buildable web example.
//! - **Accessibility:** the AccessKit projection runs on desktop only. The
//!   web target has no accessibility backend in this version.

pub mod prelude;

pub use smudge_core as core;
pub use smudge_text as text;
pub use smudge_widgets as widgets;

#[cfg(feature = "window")]
pub use smudge_platform as window;

#[cfg(feature = "gpu")]
pub use smudge_gpu as gpu;
