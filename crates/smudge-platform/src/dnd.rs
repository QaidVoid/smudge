//! File drags that cross into the window.
//!
//! On X11, Windows and macOS the windowing library reports the drops
//! itself, and nothing here is built. On a Wayland session the windowing
//! library has no drag and drop, so the client in [`wayland`] speaks the
//! data device protocol on the display the windowing library owns, which
//! is the connection the compositor routes a drag to. On the web nothing
//! supports them.
//!
//! A Wayland drag names its files only at the drop, so a hover that comes
//! from there reports no paths yet and the drop carries them. An X11 drag
//! carries its paths already while it hovers.

#[cfg(all(
    unix,
    not(any(
        target_os = "redox",
        target_family = "wasm",
        target_os = "android",
        target_os = "ios",
        target_os = "macos"
    ))
))]
mod wayland;

pub(crate) use wayland::{WaylandDnd, open_for_window};
