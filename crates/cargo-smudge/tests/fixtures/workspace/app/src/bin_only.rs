//! A module only the binary owns, which stays a patchable source even
//! though a library lives in the same package and directory.

/// The tag the binary prints, which an edit here patches with the app.
#[inline(never)]
pub fn tag() -> &'static str {
    "bin-1"
}
