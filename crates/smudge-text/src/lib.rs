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
//! ```
//! use smudge_text::{TextLayout, TextSystem, TextStyle};
//!
//! TextSystem::bundled_only().install();
//!
//! let style = TextStyle::default();
//! let layout = TextLayout::new("Hello, Smudge", &style, None);
//! assert!(layout.size().width > 0.0);
//! ```

mod fonts;
mod glyphs;
mod input;
mod label;
mod layout;
mod style;
mod system;
mod text_input;

pub use input::Editor;
pub use label::{Label, LabelExt, label};
pub use layout::TextLayout;
pub use style::{TextStyle, current_text_style, provide_text_style};
pub use system::TextSystem;
pub use text_input::{TextInput, TextInputExt, text_input};
