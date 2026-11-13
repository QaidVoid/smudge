//! Text layout for Smudge, backed by parley.
//!
//! This crate owns parley's font and layout contexts in a per-thread
//! [`TextSystem`], lays out text through [`TextLayout`], and shares a
//! [`TextStyle`] with widgets through reactive context.
//!
//! The fonts in this crate's `assets/fonts` directory are embedded by
//! default, so layout is deterministic and never depends on the fonts
//! installed on the host. System font discovery is opt-in through the
//! `system-fonts` feature and is unavailable on wasm.
//!
//! Without the `complex-scripts` feature, line and word breaks follow the
//! Unicode rules alone. Chinese and Japanese still wrap between characters,
//! but word selection treats each Han character as a word, and Thai, Lao,
//! Khmer and Burmese, which put no spaces between words, wrap only at
//! spaces. The feature compiles in the break dictionaries for those scripts,
//! at about 3 MB per binary and about 5 percent more layout time for other
//! text.
//!
//! One font source must reach this crate. A build in which none does fails
//! to compile, because every glyph would lay out as missing. Enable the
//! `bundled-fonts` feature, the default, on `smudge`, `smudge-widgets` or
//! this crate, or the `system-fonts` feature on `smudge` or this crate. The
//! system fonts count only where they can be found: on Linux, FreeBSD,
//! Windows, Android and Apple platforms, and not on the web.
//!
//! ```
//! use smudge_text::{TextLayout, TextSystem, TextStyle};
//!
//! TextSystem::bundled_only().install();
//!
//! let style = TextStyle::default();
//! let layout = TextLayout::new("Hello, Smudge", &style, None);
//! assert!(layout.size().width > 0.0);
//! ```

mod chrome;

// The build fails when no font source reaches this crate, because text
// would draw nothing: every glyph would lay out as missing. The system fonts
// count only on the targets where the font collection can discover them.
// Elsewhere, the web included, it finds none.
#[cfg(not(any(
    feature = "bundled-fonts",
    all(
        feature = "system-fonts",
        any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows",
            target_os = "android",
            target_vendor = "apple",
        ),
    ),
)))]
compile_error!(
    "smudge-text has no font source: enable the `bundled-fonts` feature \
     (the embedded fonts, on by default) on smudge, smudge-widgets or \
     smudge-text, or the `system-fonts` feature on smudge or smudge-text \
     on Linux, FreeBSD, Windows, Android or an Apple platform"
);

mod fonts;
mod glyphs;
mod input;
mod label;
mod layout;
mod line_breaks;
mod style;
mod system;
mod text_area;
mod text_input;

pub use input::{Editor, EditorParagraph};
pub use label::{Label, LabelExt, label};
pub use layout::TextLayout;
pub use parley::FontFamily;
pub use parley::FontWeight;
pub use parley::GenericFamily;
pub use style::{TextStyle, current_text_style, provide_text_style};
pub use system::TextSystem;
pub use text_area::{TextArea, TextAreaExt, text_area};
pub use text_input::{TextInput, TextInputExt, text_input};
