//! The system clipboard, backed by arboard.
//!
//! The window installs one on every app it opens, so cut, copy and paste
//! reach the operating system's clipboard without the app doing anything.
//! Where the system clipboard cannot be opened, the window falls back to the
//! app's in-memory clipboard, which keeps the keys working inside the app.

use smudge_core::clipboard::Clipboard;

/// The operating system's clipboard, for the text an app copies and pastes.
pub(crate) struct SystemClipboard {
    /// The arboard clipboard, which talks to the platform.
    inner: arboard::Clipboard,
}

impl SystemClipboard {
    /// Opens the system clipboard.
    ///
    /// # Errors
    ///
    /// Returns the platform's error when the clipboard cannot be opened, for
    /// example on a headless display.
    pub(crate) fn new() -> Result<SystemClipboard, arboard::Error> {
        arboard::Clipboard::new().map(|inner| SystemClipboard { inner })
    }
}

impl Clipboard for SystemClipboard {
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
