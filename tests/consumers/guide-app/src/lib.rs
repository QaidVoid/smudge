//! An application crate that only needs to build: the configurations of the
//! small-binary guide are checked by compiling it with their features.

use smudge::prelude::*;

/// The application's root element.
pub fn root() -> Element<Panel> {
    column().child(label("Hello, Smudge"))
}
