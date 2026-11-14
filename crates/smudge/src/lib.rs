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
//!     run_with(
//!         WindowOptions {
//!             title: String::from("Counter"),
//!             size: Size {
//!                 width: 280.0,
//!                 height: 140.0,
//!             },
//!             clear_color: Color::rgb(16, 17, 21),
//!             ..WindowOptions::default()
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
//!         |app| app.set_theme(Theme::dark()),
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
//!   GPU. Windows start on the CPU renderer either way, and
//!   [`window::RendererChoice::Auto`] asks for the GPU when the adapter is a
//!   real GPU, falling back to the CPU renderer on a software rasterizer.
//! - **Web:** the CPU renderer runs in a browser on `wasm32-unknown-unknown`;
//!   `crates/smudge-platform/examples/web` holds a buildable web example.
//! - **Accessibility:** the AccessKit projection runs on desktop only, with
//!   the `a11y` feature, which is on by default. The web target has no
//!   accessibility backend in this version.
//! - **Text:** Chinese, Japanese, Thai, Lao, Khmer and Burmese break by word
//!   only with the `complex-scripts` feature, which compiles in the break
//!   dictionaries at about 3 MB per binary. Text needs a font source: a
//!   build in which the text crate gets no font feature fails to compile,
//!   whichever crate the feature is enabled on.
//! - **Hot reload:** with the `hot-reload` feature, on Linux, an application
//!   started through [`window::run_view`] under a patch builder remounts its
//!   root through the patched code after every patch, and keeps the state it
//!   passed in. Everywhere else `run_view` is a plain run.

pub mod prelude;

pub use smudge_core as core;
pub use smudge_text as text;
pub use smudge_widgets as widgets;

/// Hot reload helpers for applications.
///
/// [`call`](self::hot::call) runs one step of a loop on a thread the
/// application spawned itself, so the step picks up the patched code on the
/// next iteration. [`on_hot_patch`](self::hot::on_hot_patch) registers a
/// hook the window runs after it applied a patch and remounted, so an
/// application restarts the workers it created outside the build. Both are
/// for applications started through
/// [`window::run_view`] with the `hot-reload` feature on
/// Linux; everywhere else `call` runs the body and `on_hot_patch` drops the
/// hook.
pub mod hot {
    pub use smudge_core::hot::{call, on_hot_patch};
}

/// The commands an application sends to its window from a handler, the
/// window state it reads, and the pointer cursors.
///
/// This is the core's window module under a name that does not clash with
/// [`window`], the windowing layer that applies the commands.
///
/// ```
/// use smudge::commands::{self, ResizeEdge};
/// use smudge::prelude::*;
///
/// let header = row()
///     .height(px(36.0))
///     .on_pointer_down(|_| commands::start_move())
///     .child(
///         panel()
///             .width(px(6.0))
///             .cursor(Cursor::Resize(ResizeEdge::East))
///             .hidden(commands::maximized)
///             .on_pointer_down(|_| commands::start_resize(ResizeEdge::East)),
///     )
///     .child(button("Close").on_press(commands::close));
/// ```
pub use smudge_core::window as commands;

#[cfg(feature = "window")]
pub use smudge_platform as window;

#[cfg(feature = "gpu")]
pub use smudge_gpu as gpu;
