//! Style variants: patches the base style takes on while a node is hovered,
//! keyboard focused, pressed or disabled.
//!
//! Every node carries one complete base [`Style`]. An
//! element can declare a patch per interaction state, and the tree keeps the
//! base style and the resolved style apart: builders and bindings write the
//! base style, and layout and paint read the resolved style, which the tree
//! recomputes whenever the base style or the interaction bits change.
//!
//! The patches apply in one fixed order: base, then hovered, then focused,
//! then pressed, then disabled. A patch overrides exactly the fields its
//! setters wrote, so a hovered background leaves the base width alone. The
//! focused state applies only while the focus last moved by keyboard
//! navigation, so a focus ring follows Tab and not a mouse click.
//!
//! A patch holds static values only. There are no reactive values inside a
//! patch, and the setters of [`PatchBuilder`] take plain values, so a closure
//! over signals cannot be written into one. A value that changes with state
//! belongs in a binding on the base style, which the patches then override
//! while their state is active.
//!
//! ```
//! use smudge_core::geometry::Color;
//! use smudge_core::style::px;
//! use smudge_core::widget::panel;
//!
//! let button = panel()
//!     .width(px(80.0))
//!     .height(px(24.0))
//!     .background(Color::rgb(60, 60, 60))
//!     .hover(|p| p.background(Color::rgb(90, 90, 90)))
//!     .pressed(|p| p.background(Color::rgb(40, 40, 40)))
//!     .disabled_style(|p| p.background(Color::rgb(30, 30, 30)));
//! ```

use std::rc::Rc;

use crate::geometry::{Color, Corners, Size};
use crate::style::{Align, Direction, Edges, Justify, Length, Overflow, Position, Style};

use crate::tree::interaction::Interaction;

/// One recorded setter of a style patch.
type Setter = Rc<dyn Fn(&mut Style)>;

/// A patch of static style values, applied over the base style while one
/// interaction state is active.
///
/// Build one with [`PatchBuilder`], which the variant methods of
/// [`Element`](crate::element::Element) run for you. The patch overrides the
/// fields its setters wrote and leaves every other field of the base style
/// alone.
pub struct StylePatch {
    /// The setters, run in the order they were recorded.
    setters: Vec<Setter>,
}

impl StylePatch {
    /// Applies every setter of the patch to `style`, in the order they were
    /// recorded.
    pub(crate) fn apply(&self, style: &mut Style) {
        for setter in &self.setters {
            setter(style);
        }
    }
}

/// Builds a [`StylePatch`] with the same setters the element builders have,
/// for static values only.
///
/// A variant method such as [`hover`](crate::element::Element::hover) hands a
/// fresh builder to the closure the caller passes and takes the builder back
/// when the closure returns it, so a patch reads as a chain:
/// `|p| p.background(color).border(1.0)`.
#[derive(Default)]
pub struct PatchBuilder {
    /// The setters, in the order they were recorded.
    setters: Vec<Setter>,
}

impl PatchBuilder {
    /// Records a setter that writes one field of the style.
    fn push(mut self, set: impl Fn(&mut Style) + 'static) -> Self {
        self.setters.push(Rc::new(set));
        self
    }

    /// Sets the width. This takes part in layout.
    pub fn width(self, width: Length) -> Self {
        self.push(move |style| style.layout.width = width)
    }

    /// Sets the height. This takes part in layout.
    pub fn height(self, height: Length) -> Self {
        self.push(move |style| style.layout.height = height)
    }

    /// Sets the width and the height, in logical pixels. This takes part in
    /// layout.
    pub fn size(self, size: Size) -> Self {
        self.push(move |style| {
            style.layout.width = Length::px(size.width);
            style.layout.height = Length::px(size.height);
        })
    }

    /// Sets the lower bound of the width. This takes part in layout.
    pub fn min_width(self, width: Length) -> Self {
        self.push(move |style| style.layout.min_width = width)
    }

    /// Sets the lower bound of the height. This takes part in layout.
    pub fn min_height(self, height: Length) -> Self {
        self.push(move |style| style.layout.min_height = height)
    }

    /// Sets the upper bound of the width. This takes part in layout.
    pub fn max_width(self, width: Length) -> Self {
        self.push(move |style| style.layout.max_width = width)
    }

    /// Sets the upper bound of the height. This takes part in layout.
    pub fn max_height(self, height: Length) -> Self {
        self.push(move |style| style.layout.max_height = height)
    }

    /// Sets the same padding on all four sides. This takes part in layout.
    pub fn padding(self, padding: Length) -> Self {
        self.push(move |style| style.layout.padding = Edges::all(padding))
    }

    /// Sets the horizontal padding from `width` and the vertical padding from
    /// `height`. This takes part in layout.
    pub fn padding_xy(self, padding: Size) -> Self {
        self.push(move |style| {
            style.layout.padding = Edges::xy(Length::px(padding.width), Length::px(padding.height));
        })
    }

    /// Sets the same margin on all four sides. This takes part in layout.
    pub fn margin(self, margin: Length) -> Self {
        self.push(move |style| style.layout.margin = Edges::all(margin))
    }

    /// Sets the space between adjacent children. This takes part in layout.
    pub fn gap(self, gap: Size) -> Self {
        self.push(move |style| style.layout.gap = gap)
    }

    /// Sets the main axis of the children's layout. This takes part in layout.
    pub fn direction(self, direction: Direction) -> Self {
        self.push(move |style| style.layout.direction = direction)
    }

    /// Lays the children out left to right. This takes part in layout.
    pub fn row(self) -> Self {
        self.push(|style| style.layout.direction = Direction::Row)
    }

    /// Lays the children out top to bottom. This takes part in layout.
    pub fn column(self) -> Self {
        self.push(|style| style.layout.direction = Direction::Column)
    }

    /// Lets children that do not fit move to a new line instead of shrinking.
    /// This takes part in layout.
    pub fn wrap(self, wrap: bool) -> Self {
        self.push(move |style| style.layout.wrap = wrap)
    }

    /// Sets the share of the leftover space on the main axis this node takes.
    /// This takes part in layout.
    pub fn grow(self, grow: f32) -> Self {
        self.push(move |style| style.layout.grow = grow)
    }

    /// Sets how readily this node gives up space when its siblings overflow.
    /// This takes part in layout.
    pub fn shrink(self, shrink: f32) -> Self {
        self.push(move |style| style.layout.shrink = shrink)
    }

    /// Sets the main axis size the node starts from before growing or
    /// shrinking. This takes part in layout.
    pub fn basis(self, basis: Length) -> Self {
        self.push(move |style| style.layout.basis = basis)
    }

    /// Sets how the children line up across the main axis. This takes part in
    /// layout.
    pub fn align_items(self, align: Align) -> Self {
        self.push(move |style| style.layout.align_items = Some(align))
    }

    /// Sets how the children are spread along the main axis. This takes part
    /// in layout.
    pub fn justify_content(self, justify: Justify) -> Self {
        self.push(move |style| style.layout.justify_content = Some(justify))
    }

    /// Takes the node out of normal flow, so its insets place it against its
    /// containing block. This takes part in layout.
    pub fn absolute(self) -> Self {
        self.push(|style| style.layout.position = Position::Absolute)
    }

    /// Sets the offsets from the containing block's edges. This takes part in
    /// layout.
    pub fn inset(self, inset: Edges<Length>) -> Self {
        self.push(move |style| style.layout.inset = inset)
    }

    /// Sets the border width, the same on all four sides. This takes part in
    /// layout, because the border takes space away from the content.
    pub fn border(self, width: f32) -> Self {
        self.push(move |style| style.layout.border_width = Edges::all(width))
    }

    /// Clips the children to the node's bounds. This takes part in layout.
    pub fn clip(self, clip: bool) -> Self {
        self.push(move |style| {
            style.layout.overflow = if clip {
                Overflow::Clip
            } else {
                Overflow::Visible
            };
        })
    }

    /// Hides the node and its subtree, or shows them again. This takes part
    /// in layout.
    pub fn hidden(self, hidden: bool) -> Self {
        self.push(move |style| style.layout.hidden = hidden)
    }

    /// Sets the fill painted behind the node.
    pub fn background(self, color: Color) -> Self {
        self.push(move |style| style.paint.background = Some(color))
    }

    /// Sets the color of the border whose width [`PatchBuilder::border`]
    /// sets.
    pub fn border_color(self, color: Color) -> Self {
        self.push(move |style| style.paint.border_color = Some(color))
    }

    /// Sets the corner radius of the background and the border, the same on
    /// all four corners.
    pub fn corner_radius(self, radius: f32) -> Self {
        self.push(move |style| style.paint.corner_radius = Corners::all(radius))
    }

    /// Consumes the builder into the patch it recorded.
    pub(crate) fn patch(self) -> StylePatch {
        StylePatch {
            setters: self.setters,
        }
    }
}

/// The style patches of one node, one per interaction state.
///
/// An element builds this through its
/// [`hover`](crate::element::Element::hover),
/// [`focused`](crate::element::Element::focused),
/// [`pressed`](crate::element::Element::pressed) and
/// [`disabled_style`](crate::element::Element::disabled_style) methods. A node
/// without any patch never changes its style with its interaction state.
pub struct Variants {
    /// Applied while the node is in the hovered chain.
    pub hovered: Option<StylePatch>,
    /// Applied while the node holds the keyboard focus and the focus last
    /// moved by keyboard navigation.
    pub focused: Option<StylePatch>,
    /// Applied while a primary press is held on the node.
    pub pressed: Option<StylePatch>,
    /// Applied while the node is disabled.
    pub disabled: Option<StylePatch>,
}

impl Default for Variants {
    /// A set with no patches, which leaves the base style in force in every
    /// state.
    fn default() -> Variants {
        Variants {
            hovered: None,
            focused: None,
            pressed: None,
            disabled: None,
        }
    }
}

impl Variants {
    /// Returns whether no state carries a patch.
    pub(crate) fn is_empty(&self) -> bool {
        self.hovered.is_none()
            && self.focused.is_none()
            && self.pressed.is_none()
            && self.disabled.is_none()
    }
}

/// Resolves the style of a node from its base style, its patches and its
/// interaction state.
///
/// The base style comes first, then the hovered patch, then the focused one,
/// then the pressed one, then the disabled one. Each patch overrides the
/// fields its setters wrote; a later patch wins over an earlier one for the
/// same field. The base style is not modified.
pub(crate) fn resolve(base: &Style, variants: &Variants, interaction: Interaction) -> Style {
    let mut style = base.clone();
    if interaction.has(Interaction::HOVERED)
        && let Some(patch) = &variants.hovered
    {
        patch.apply(&mut style);
    }
    if interaction.has(Interaction::KEYBOARD_FOCUS)
        && let Some(patch) = &variants.focused
    {
        patch.apply(&mut style);
    }
    if interaction.has(Interaction::PRESSED)
        && let Some(patch) = &variants.pressed
    {
        patch.apply(&mut style);
    }
    if interaction.has(Interaction::DISABLED)
        && let Some(patch) = &variants.disabled
    {
        patch.apply(&mut style);
    }
    style
}

#[cfg(test)]
#[path = "variants_tests.rs"]
mod tests;
