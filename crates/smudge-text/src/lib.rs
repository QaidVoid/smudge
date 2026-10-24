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
mod layout;
mod style;
mod system;

pub use layout::TextLayout;
pub use style::{TextStyle, current_text_style, provide_text_style};
pub use system::TextSystem;
