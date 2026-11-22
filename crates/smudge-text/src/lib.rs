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
//! Windows, Android and Apple platforms, and not on the web. An app that
//! registers fonts of its own with [`TextSystem::register_font`] can enable
//! the `app-fonts` feature on `smudge`, `smudge-widgets` or this crate
//! instead, which makes those fonts its source.
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

mod a11y;
mod chrome;

/// Whether the build carries a font source of its own: the bundled faces,
/// or the system fonts on a target where the font collection can discover
/// them. Elsewhere, the web included, it finds none.
const BUILT_IN_FONTS: bool = cfg!(any(
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
));

// The build fails when no font source reaches this crate, because text
// would draw nothing: every glyph would lay out as missing. The app's own
// fonts count when it says it registers them, because the build cannot see
// a registration.
const _: () = assert!(
    BUILT_IN_FONTS || cfg!(feature = "app-fonts"),
    "smudge-text has no font source: enable the `bundled-fonts` feature \
     (the embedded fonts, on by default) on smudge, smudge-widgets or \
     smudge-text, the `system-fonts` feature on smudge or smudge-text \
     on Linux, FreeBSD, Windows, Android or an Apple platform, or the \
     `app-fonts` feature on smudge, smudge-widgets or smudge-text when the \
     app registers fonts of its own"
);

mod fonts;
mod glyphs;
mod input;
mod label;
mod layout;
mod line_breaks;
mod rich_text;
mod selection;
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
pub use rich_text::{RichText, RichTextExt, Span, SpanStyle, rich_text};
pub use selection::{SelectableExt, selection_scope};
pub use style::{TextStyle, current_text_style, provide_text_style};
pub use system::{FontError, TextSystem};
pub use text_area::{TextArea, TextAreaExt, text_area};
pub use text_input::{PressHold, TextInput, TextInputExt, text_input};

/// Creates a signal the library keeps for itself, such as a widget's own
/// state.
///
/// It passes no caller on, unlike the public constructors that call it, so
/// hot reload files the signal under this crate rather than under the
/// app's code, where it would take the place of one of the app's signals.
pub(crate) fn signal<T: 'static>(value: T) -> smudge_core::reactive::RwSignal<T> {
    smudge_core::reactive::RwSignal::new(value)
}
