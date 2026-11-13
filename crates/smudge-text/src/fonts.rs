//! The font faces bundled with the library.
//!
//! The files come from the official Noto Sans release and are licensed under
//! the SIL Open Font License 1.1, which `assets/fonts/OFL.txt` carries. They
//! are embedded so that layout produces the same result on every machine and
//! on wasm, which has no fonts of its own.
//!
//! Each file's `post` table is rewritten to version 3, which drops the glyph
//! names and keeps every other table, glyph and metric unchanged. Nothing
//! reads the names when shaping or drawing, and they took about 42 KB per
//! face.

/// The family name of the bundled sans serif faces.
pub(crate) const SANS_FAMILY: &str = "Noto Sans";

#[cfg(feature = "bundled-fonts")]
pub(crate) static SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");

#[cfg(feature = "bundled-fonts")]
pub(crate) static SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");

#[cfg(feature = "bundled-fonts")]
pub(crate) static MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSansMono-Regular.ttf");
