//! The text style that widgets inherit through reactive context.
//!
//! There is no cascade: a widget that wants something other than the ambient
//! style states it explicitly at mount. The style is provided under the
//! current reactive owner with [`provide_text_style`] and read by descendants
//! with [`current_text_style`].

use parley::FontWeight;
use smudge_core::reactive::{provide_context, use_context};
use smudge_core::theme::{ColorToken, ColorValue};

/// How a text widget lays out and paints its text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// The font family to resolve, named or generic.
    pub family: parley::FontFamily<'static>,
    /// The font size in logical pixels.
    pub size: f32,
    /// The weight the family is matched with.
    pub weight: FontWeight,
    /// The line height as a multiple of the font size.
    pub line_height: f32,
    /// The color the glyphs paint with, which may name a theme token and
    /// resolves when the widget paints.
    pub color: ColorValue,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            family: parley::FontFamily::named(crate::fonts::SANS_FAMILY),
            size: 14.0,
            weight: FontWeight::NORMAL,
            line_height: 1.3,
            color: ColorValue::Token(ColorToken::TEXT),
        }
    }
}

/// Provides the text style that widgets created under the current owner read.
///
/// A later call on an ancestor does not reach widgets that already read the
/// style, because text styles are read once at mount.
pub fn provide_text_style(style: TextStyle) {
    provide_context(style);
}

/// Returns the text style of the closest ancestor that called
/// [`provide_text_style`], or [`TextStyle::default`] when none did.
pub fn current_text_style() -> TextStyle {
    use_context::<TextStyle>().unwrap_or_default()
}
