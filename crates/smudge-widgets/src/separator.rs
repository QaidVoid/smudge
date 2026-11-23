//! The separator: the line that divides sections of a form.

use smudge_core::element::Element;
use smudge_core::style::{Align, px};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{Panel, panel};

/// Creates a separator: a one logical pixel line in the theme's
/// [`BORDER`](ColorToken::BORDER) color that divides the sections of a form.
///
/// In a [`column`](crate::column) the line runs horizontally, in a
/// [`row`](crate::row) it runs vertically: the separator always takes one
/// logical pixel of its parent's main axis and stretches across the whole
/// cross axis, whichever way the parent lays its children out. It shows
/// nothing to a screen reader, because it divides nothing but space.
///
/// The separator takes no press and no keys, and a gap the parent sets
/// keeps it apart from the sections around it.
///
/// ```
/// use smudge_core::geometry::Size;
/// use smudge_text::label;
/// use smudge_widgets::{column, separator};
///
/// let form = column()
///     .gap(Size {
///         width: 0.0,
///         height: 8.0,
///     })
///     .child(label("Name"))
///     .child(separator())
///     .child(label("Address"));
/// ```
#[track_caller]
pub fn separator() -> Element<Panel> {
    panel()
        .basis(px(1.0))
        .min_width(px(1.0))
        .min_height(px(1.0))
        .align_self(Align::Stretch)
        .background(ColorToken::BORDER)
}
