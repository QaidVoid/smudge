//! The labels the app prints, which an edit here patches through the
//! library crate's own replay.

#[inline(never)]
pub fn label() -> &'static str {
    "ui-1"
}

/// Reads the deeper library, so the chain from an edit there to the app
/// runs through this crate. The dependency is renamed in this crate's
/// manifest, which keeps `core2` the crate name its artifacts carry.
#[inline(never)]
pub fn deep() -> u32 {
    engine::seed()
}

/// The tag the macro baked in when this crate was compiled, which an
/// edit in the macro's helper changes through the macro's own replay.
#[inline(never)]
pub fn branded() -> &'static str {
    mac::tag!()
}

/// Only exists so the `fancy` feature makes its own build, which a
/// feature switch must record as its own unit.
#[cfg(feature = "fancy")]
pub fn fancy_only() {}
