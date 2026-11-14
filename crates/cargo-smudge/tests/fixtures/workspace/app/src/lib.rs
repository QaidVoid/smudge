//! The app package's library, present so the package builds a library
//! and a binary of the same name, which one patch chain cannot replay
//! separately.

mod shared;

/// Keeps the library's own build honest, which a patch never replays.
#[inline(never)]
pub fn placeholder() -> u32 {
    1
}

/// Reads the source both targets compile, so the library's own unit owns
/// it as much as the binary's does.
#[inline(never)]
pub fn echoed() -> &'static str {
    shared::TAG
}
