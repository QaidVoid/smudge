//! Style data for one node, split into layout properties and paint properties.
//!
//! The split is the mechanism that lets a paint-only change skip layout.
//! [`LayoutProps`] converts to the layout engine's style and is compared before
//! and after a change to decide whether the node needs layout again, while
//! [`PaintProps`] never reaches the layout engine.
//!
//! There is no cascade and no inheritance: every node carries a complete
//! [`Style`] of its own.

use crate::geometry::{Color, Corners, Size};

/// A length along one axis.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Length {
    /// A fixed length in logical pixels.
    Px(f32),
    /// A percentage of the containing block's size, where `50.0` is half.
    Percent(f32),
    /// A length the layout algorithm computes from the content and the space
    /// available.
    Auto,
}

impl Length {
    /// A fixed length in logical pixels.
    pub const fn px(value: f32) -> Length {
        Length::Px(value)
    }

    /// A percentage of the containing block's size, where `50.0` is half.
    pub const fn pct(percent: f32) -> Length {
        Length::Percent(percent)
    }

    /// A length the layout algorithm computes.
    pub const fn auto() -> Length {
        Length::Auto
    }
}

/// Four edge values, one per side of a box.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Edges<T> {
    /// Value for the top edge.
    pub top: T,
    /// Value for the right edge.
    pub right: T,
    /// Value for the bottom edge.
    pub bottom: T,
    /// Value for the left edge.
    pub left: T,
}

impl<T: Copy> Edges<T> {
    /// The same value on every edge.
    pub const fn all(value: T) -> Edges<T> {
        Edges {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// `x` on the left and right edges, `y` on the top and bottom edges.
    pub const fn xy(x: T, y: T) -> Edges<T> {
        Edges {
            top: y,
            right: x,
            bottom: y,
            left: x,
        }
    }
}

/// The main axis of a flex layout.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Direction {
    /// Children are placed left to right and wrap onto new lines downwards.
    Row,
    /// Children are placed top to bottom and wrap onto new columns.
    Column,
}

/// Cross axis alignment of a node's children.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Align {
    /// Pack children against the start of the cross axis.
    Start,
    /// Pack children against the end of the cross axis.
    End,
    /// Center children on the cross axis.
    Center,
    /// Stretch children to fill the cross axis.
    Stretch,
}

/// Main axis distribution of a node's children.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Justify {
    /// Pack children against the start of the main axis.
    Start,
    /// Pack children against the end of the main axis.
    End,
    /// Center children on the main axis.
    Center,
    /// Place the first and last child at the ends of the main axis and spread
    /// the remaining space between them.
    SpaceBetween,
}

/// Whether a node takes part in normal flow.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Position {
    /// The node sits in normal flow. Insets move it from where the flow put it
    /// without moving its siblings, and the node becomes the containing block
    /// for its absolutely positioned children. This is the default, so an
    /// absolutely positioned child is placed against its own parent.
    Relative,
    /// The node leaves normal flow and is placed by its insets against the
    /// nearest positioned ancestor. Siblings are laid out as if it were not
    /// there.
    Absolute,
}

/// Which layout algorithm arranges a node's children.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Display {
    /// Children are laid out as a flex row or column.
    Flex,
    /// Children are laid out one after another down the block axis.
    Block,
    /// The node and its subtree are not laid out and paint nothing.
    None,
}

/// What happens to children that do not fit inside their parent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Overflow {
    /// Children paint outside the parent's bounds.
    Visible,
    /// Children are clipped to the parent's bounds.
    Clip,
}

/// The properties that take part in layout.
///
/// Every field here reaches the layout engine, so a change to any of them
/// requires a new layout pass. A change to [`PaintProps`] alone does not.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutProps {
    /// Which layout algorithm arranges this node's children.
    pub display: Display,
    /// The main axis of the flex layout.
    pub direction: Direction,
    /// Whether children that do not fit move to a new line instead of
    /// shrinking.
    pub wrap: bool,
    /// Share of the leftover main axis space this node takes when it is a flex
    /// child. Zero keeps the node at its basis.
    pub grow: f32,
    /// How readily this node gives up main axis space when flex children
    /// overflow. Zero never shrinks.
    pub shrink: f32,
    /// The main axis size this node starts from before growing or shrinking.
    pub basis: Length,
    /// Size on the horizontal axis.
    pub width: Length,
    /// Size on the vertical axis.
    pub height: Length,
    /// Lower bound for [`LayoutProps::width`].
    pub min_width: Length,
    /// Lower bound for [`LayoutProps::height`].
    pub min_height: Length,
    /// Upper bound for [`LayoutProps::width`].
    pub max_width: Length,
    /// Upper bound for [`LayoutProps::height`].
    pub max_height: Length,
    /// Space between the node's edge and its children.
    pub padding: Edges<Length>,
    /// Space around the node, outside its border.
    pub margin: Edges<Length>,
    /// Space between adjacent children.
    pub gap: Size,
    /// Cross axis alignment for this node's children. `None` leaves the layout
    /// engine's default, which stretches children across the cross axis.
    pub align_items: Option<Align>,
    /// Main axis distribution of this node's children. `None` leaves the
    /// layout engine's default, which packs them against the start.
    pub justify_content: Option<Justify>,
    /// Whether the node stays in normal flow.
    pub position: Position,
    /// Offsets from the containing block's edges. They position an absolutely
    /// positioned node and shift a relatively positioned one from its flow
    /// position.
    pub inset: Edges<Length>,
    /// Border thickness inside the node's edge. The border takes space away
    /// from the content and is painted in [`PaintProps::border_color`].
    pub border_width: Edges<f32>,
    /// Whether content that does not fit is clipped. This also affects the
    /// automatic minimum size of a flex item, so it is a layout property.
    pub overflow: Overflow,
}

impl Default for LayoutProps {
    /// A flex row that is sized by its content, does not grow, shrinks freely
    /// and takes no space beyond its children. These are the layout engine's
    /// defaults, with the exception of [`Position::Relative`], which this crate
    /// uses so that every node is a containing block for its own children.
    fn default() -> LayoutProps {
        LayoutProps {
            display: Display::Flex,
            direction: Direction::Row,
            wrap: false,
            grow: 0.0,
            shrink: 1.0,
            basis: Length::Auto,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            padding: Edges::all(Length::px(0.0)),
            margin: Edges::all(Length::px(0.0)),
            gap: Size::default(),
            align_items: None,
            justify_content: None,
            position: Position::Relative,
            inset: Edges::all(Length::auto()),
            border_width: Edges::all(0.0),
            overflow: Overflow::Visible,
        }
    }
}

impl LayoutProps {
    /// Converts these properties to the layout engine's style.
    ///
    /// Percentages are divided by 100, so [`Length::pct`] with `50.0` becomes
    /// the engine's fraction `0.5`. [`Length::Auto`] in a property that has no
    /// automatic value, which is padding and gap, becomes zero.
    ///
    /// ```
    /// use smudge_core::style::{LayoutProps, Length};
    ///
    /// let props = LayoutProps {
    ///     width: Length::pct(50.0),
    ///     ..LayoutProps::default()
    /// };
    /// assert_eq!(
    ///     props.to_taffy().size.width.expand(),
    ///     taffy::ExpandedDimension::Percent(0.5)
    /// );
    /// ```
    pub fn to_taffy(&self) -> taffy::Style {
        let overflow = match self.overflow {
            Overflow::Visible => taffy::Overflow::Visible,
            Overflow::Clip => taffy::Overflow::Clip,
        };
        let mut style: taffy::Style = taffy::Style::DEFAULT;
        style.display = match self.display {
            Display::Flex => taffy::Display::Flex,
            Display::Block => taffy::Display::Block,
            Display::None => taffy::Display::None,
        };
        style.flex_direction = match self.direction {
            Direction::Row => taffy::FlexDirection::Row,
            Direction::Column => taffy::FlexDirection::Column,
        };
        style.flex_wrap = if self.wrap {
            taffy::FlexWrap::Wrap
        } else {
            taffy::FlexWrap::NoWrap
        };
        style.flex_grow = self.grow;
        style.flex_shrink = self.shrink;
        style.flex_basis = to_dimension(self.basis);
        style.size = taffy::Size {
            width: to_dimension(self.width),
            height: to_dimension(self.height),
        };
        style.min_size = taffy::Size {
            width: to_length_percentage_auto(self.min_width),
            height: to_length_percentage_auto(self.min_height),
        };
        style.max_size = taffy::Size {
            width: to_length_percentage_auto(self.max_width),
            height: to_length_percentage_auto(self.max_height),
        };
        style.padding = to_taffy_edges(self.padding, to_length_percentage);
        style.margin = to_taffy_edges(self.margin, to_length_percentage_auto);
        style.gap = taffy::Size {
            width: taffy::LengthPercentage::length(self.gap.width),
            height: taffy::LengthPercentage::length(self.gap.height),
        };
        style.align_items = match self.align_items {
            None => taffy::AlignItems::NORMAL,
            Some(Align::Start) => taffy::AlignItems::START,
            Some(Align::End) => taffy::AlignItems::END,
            Some(Align::Center) => taffy::AlignItems::CENTER,
            Some(Align::Stretch) => taffy::AlignItems::STRETCH,
        };
        style.justify_content = match self.justify_content {
            None => taffy::JustifyContent::NORMAL,
            Some(Justify::Start) => taffy::JustifyContent::START,
            Some(Justify::End) => taffy::JustifyContent::END,
            Some(Justify::Center) => taffy::JustifyContent::CENTER,
            Some(Justify::SpaceBetween) => taffy::JustifyContent::SPACE_BETWEEN,
        };
        style.position = match self.position {
            Position::Relative => taffy::Position::Relative,
            Position::Absolute => taffy::Position::Absolute,
        };
        style.inset = to_taffy_edges(self.inset, to_length_percentage_auto);
        style.border = to_taffy_edges(self.border_width, taffy::LengthPercentage::length);
        style.overflow = taffy::Point {
            x: overflow,
            y: overflow,
        };
        style
    }
}

/// The properties that are painted but never laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintProps {
    /// Fill painted behind the node's content, or `None` for no fill.
    pub background: Option<Color>,
    /// Color of the border drawn inside the node's edge, or `None` for no
    /// border. Its thickness is [`LayoutProps::border_width`].
    pub border_color: Option<Color>,
    /// Corner radii of the background and the border.
    pub corner_radius: Corners,
}

/// The complete style of one node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// Properties that take part in layout.
    pub layout: LayoutProps,
    /// Properties that only affect painting.
    pub paint: PaintProps,
}

/// Converts a length to a size, which accepts a percentage, a length or `auto`.
fn to_dimension(length: Length) -> taffy::Dimension {
    match length {
        Length::Px(value) => taffy::Dimension::length(value),
        Length::Percent(percent) => taffy::Dimension::percent(percent / 100.0),
        Length::Auto => taffy::Dimension::auto(),
    }
}

/// Converts a length to a spacing value, which accepts a percentage or a
/// length but not `auto`. An automatic length becomes zero.
fn to_length_percentage(length: Length) -> taffy::LengthPercentage {
    match length {
        Length::Px(value) => taffy::LengthPercentage::length(value),
        Length::Percent(percent) => taffy::LengthPercentage::percent(percent / 100.0),
        Length::Auto => taffy::LengthPercentage::length(0.0),
    }
}

/// Converts a length to a spacing value that also accepts `auto`.
fn to_length_percentage_auto(length: Length) -> taffy::LengthPercentageAuto {
    match length {
        Length::Px(value) => taffy::LengthPercentageAuto::length(value),
        Length::Percent(percent) => taffy::LengthPercentageAuto::percent(percent / 100.0),
        Length::Auto => taffy::LengthPercentageAuto::auto(),
    }
}

/// Converts four edge values of one length type to the layout engine's edge
/// struct.
fn to_taffy_edges<T: Copy, U>(edges: Edges<T>, convert: impl Fn(T) -> U) -> taffy::Rect<U> {
    taffy::Rect {
        left: convert(edges.left),
        right: convert(edges.right),
        top: convert(edges.top),
        bottom: convert(edges.bottom),
    }
}

#[cfg(test)]
#[path = "style_tests.rs"]
mod tests;
