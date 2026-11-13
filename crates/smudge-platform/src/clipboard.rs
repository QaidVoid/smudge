//! The system clipboard.
//!
//! The window installs one on every app it opens, so cut, copy and paste
//! reach the operating system's clipboard without the app doing anything.
//! On a Wayland session whose compositor offers a data-control protocol, the
//! clipboard talks to the compositor directly. Everywhere else, including a
//! Wayland session without one where Xwayland runs, it goes through arboard.
//! Where neither can be opened, the window falls back to the app's in-memory
//! clipboard, which keeps the keys working inside the app.

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

use smudge_core::clipboard::Clipboard;

/// Opens the system clipboard, or returns `None` when there is none to open,
/// for example on a headless display.
pub(crate) fn open() -> Option<Box<dyn Clipboard>> {
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
    if let Some(clipboard) = wayland::WaylandClipboard::new() {
        return Some(Box::new(clipboard));
    }
    let inner = arboard::Clipboard::new().ok()?;
    Some(Box::new(ArboardClipboard { inner }))
}

/// The operating system's clipboard, reached through arboard.
struct ArboardClipboard {
    /// The arboard clipboard, which talks to the platform.
    inner: arboard::Clipboard,
}

impl Clipboard for ArboardClipboard {
    fn get(&mut self) -> Option<String> {
        // A read can fail while another application owns the clipboard, or
        // when what is on it is not text, which is nothing to paste.
        self.inner.get_text().ok()
    }

    fn set(&mut self, text: String) {
        // A write can fail while another application owns the clipboard,
        // which drops this write rather than the user's text.
        let _ = self.inner.set_text(text);
    }
}
