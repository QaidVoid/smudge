//! A laid out block of text.
//!
//! [`TextLayout`] wraps parley's layout so that callers shape one style of
//! text against a maximum width and read back the resulting size and glyph
//! runs. Parley types stay in this crate: the rest of Smudge sees only this
//! wrapper and, through the run iterator, parley's own public types.

use crate::style::TextStyle;
use crate::system::TextSystem;
use parley::PositionedLayoutItem;
use parley::StyleProperty;
use parley::style::LineHeight;
use smudge_core::geometry::Size;

/// A block of text laid out in logical pixels against one style.
///
/// The layout is computed eagerly by [`TextLayout::new`] and never changes
/// afterwards. A new layout replaces an old one rather than being edited. It
/// carries no color: layout runs without a theme, and the caller paints the
/// glyph runs in whatever color it resolves when it paints.
pub struct TextLayout {
    /// The shaped and line-broken text.
    pub(crate) layout: parley::Layout<()>,
}

impl TextLayout {
    /// Lays out `text` with `style`, wrapping lines at `max_width` logical
    /// pixels, or not at all when it is `None`.
    ///
    /// Layout runs against the current thread's [`TextSystem`].
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`], in particular
    /// when called from inside another `with` call on the same thread.
    pub fn new(text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout {
        TextSystem::with(|system| {
            // Layout units are logical pixels; scaling to a device happens in
            // the renderer, not here. Coordinates stay fractional so that a
            // renderer can quantize them itself.
            let mut builder = system
                .layouts
                .ranged_builder(&mut system.fonts, text, 1.0, false);
            builder.push_default(StyleProperty::FontFamily(style.family.clone()));
            builder.push_default(StyleProperty::FontSize(style.size));
            builder.push_default(StyleProperty::FontWeight(style.weight));
            builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
                style.line_height,
            )));

            let mut layout = builder.build(text);
            layout.break_all_lines(max_width);
            TextLayout { layout }
        })
    }

    /// Returns the width and height of the laid out text in logical pixels.
    ///
    /// The width is the longest line, without hanging whitespace. The height
    /// is the sum of the line heights.
    pub fn size(&self) -> Size {
        Size {
            width: self.layout.width(),
            height: self.layout.height(),
        }
    }

    /// Returns the number of lines the text was broken into.
    pub fn line_count(&self) -> usize {
        self.layout.len()
    }

    /// Iterates the glyph runs of every line, in visual order.
    ///
    /// A glyph run carries its font, size and positioned glyphs, and no color:
    /// the caller supplies it when converting runs with `glyph_runs_at`. The
    /// item type is parley's, and its coordinates are relative to the top
    /// left of the layout.
    pub fn glyph_runs(&self) -> impl Iterator<Item = parley::GlyphRun<'_, ()>> + Clone + '_ {
        self.layout.lines().flat_map(|line| {
            line.items().filter_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => Some(run),
                PositionedLayoutItem::InlineBox(_) => None,
            })
        })
    }
}
