//! A library crate built on Smudge that only needs to build.

use smudge::prelude::*;

/// A labelled row the library offers to applications.
pub fn titled(title: &str) -> Element<Panel> {
    row().child(label(title.to_string()))
}
