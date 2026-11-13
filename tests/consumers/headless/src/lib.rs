//! A headless application crate that only needs to build without a window.

use smudge::prelude::*;

/// The application's root element.
pub fn root() -> Element<Panel> {
    column().child(label("Hello, Smudge"))
}
