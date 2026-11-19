//! The open window's native handles, for a library that parents a native
//! dialog, such as a file picker, to the window.
//!
//! [`with_window_handles`] lends the handles of the window the calling code
//! runs in to a closure. The configure step of [`run_with`](crate::run_with)
//! and [`run_view_with`](crate::run_view_with), event handlers and timers all
//! run on the window's thread after the window opened, so each can borrow
//! them. The handles implement the [`raw_window_handle`] traits in the
//! version this crate's windowing library uses, which this module re-exports
//! so a dialog library can be matched to it.
//!
//! The handles are lent, never kept, so they cannot hold the window open
//! past the run. A dialog library copies the raw handle when it takes its
//! parent, which is all a dialog needs. Use the handles on the window's
//! thread, or hand them to a dialog library that documents how it uses them
//! on its own threads.
//!
//! On the web there is no native window to parent a dialog to, so
//! [`with_window_handles`] answers [`HandleError::NotSupported`] there.
//!
//! # Examples
//!
//! ```no_run
//! use smudge_platform::handle::raw_window_handle::HasWindowHandle;
//! use smudge_platform::handle::with_window_handles;
//!
//! fn pick_file() {
//!     // A dialog library takes `handles` as the dialog's parent.
//!     let _ = with_window_handles(|handles| handles.window_handle().is_ok());
//! }
//! ```

use std::cell::RefCell;
use std::sync::{Arc, Weak};

pub use winit::raw_window_handle;
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::Window;

thread_local! {
    /// The window the run loop on this thread opened, held weakly so the
    /// window closes when the loop drops it.
    static OPEN: RefCell<Weak<Window>> = const { RefCell::new(Weak::new()) };
}

/// Records `window` as the window code on this thread borrows the handles of.
pub(crate) fn opened(window: &Arc<Window>) {
    OPEN.with(|open| *open.borrow_mut() = Arc::downgrade(window));
}

/// The native window and display handles of the open window, lent by
/// [`with_window_handles`].
///
/// Pass a reference to it wherever a library takes a parent window through
/// [`HasWindowHandle`] and [`HasDisplayHandle`].
pub struct WindowHandles {
    /// The window, held only while the handles are lent.
    window: Arc<Window>,
}

impl HasWindowHandle for WindowHandles {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for WindowHandles {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.display_handle()
    }
}

/// Runs `lend` with the handles of the window the calling code runs in, and
/// returns what it returns.
///
/// # Errors
///
/// Returns [`HandleError::Unavailable`] when no window is open on this
/// thread, as before the run starts, after it ends, on another thread, or in
/// a headless test. Returns [`HandleError::NotSupported`] on the web, which
/// has no native window.
pub fn with_window_handles<R>(lend: impl FnOnce(&WindowHandles) -> R) -> Result<R, HandleError> {
    if cfg!(target_arch = "wasm32") {
        return Err(HandleError::NotSupported);
    }
    let window = OPEN
        .with(|open| open.borrow().upgrade())
        .ok_or(HandleError::Unavailable)?;
    Ok(lend(&WindowHandles { window }))
}

#[cfg(test)]
#[path = "handle_tests.rs"]
mod tests;
