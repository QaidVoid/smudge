//! Layout containers: the row, the column, the stack and the spacer.
//!
//! All four are [`Panel`]s, so they take every style builder the core
//! defines. The containers only decide the defaults: which way their children
//! flow, and for the stack, how its children are placed.

use smudge_core::element::Element;
use smudge_core::style::{Display, Edges, Length, Position, Style, Track};
use smudge_core::widget::{Panel, panel};

/// Creates a container that lays its children out left to right.
///
/// The gap between adjacent children starts at zero; [`Element::gap`] sets
/// it.
///
/// ```
/// use smudge_core::style::px;
/// use smudge_widgets::{button, row};
///
/// let bar = row()
///     .gap(smudge_core::geometry::Size {
///         width: 8.0,
///         height: 0.0,
///     })
///     .child(button("Undo"))
///     .child(button("Redo").padding(px(8.0)));
/// ```
#[track_caller]
pub fn row() -> Element<Panel> {
    panel().row()
}

/// Creates a container that lays its children out top to bottom.
///
/// The gap between adjacent children starts at zero; [`Element::gap`] sets
/// it.
#[track_caller]
pub fn column() -> Element<Panel> {
    panel().column()
}

/// Creates a container that lays its children out in the cells of a grid.
///
/// The arguments name the column tracks. Children flow into the cells in
/// order, left to right and top to bottom, and wrap onto a row of their own
/// when the columns run out. A row that no [`Element::grid_rows`] template
/// names is as tall as its tallest child, and a child takes one cell unless
/// [`Element::column_span`] or [`Element::row_span`] widens it. The gap
/// between tracks starts at zero; [`Element::gap`] sets it.
///
/// ```
/// use smudge_core::style::Track;
/// use smudge_widgets::{button, grid};
///
/// let form = grid([Track::Px(90.0), Track::Px(160.0)])
///     .gap(smudge_core::geometry::Size {
///         width: 12.0,
///         height: 8.0,
///     })
///     .child(button("Cancel"))
///     .child(button("Save"));
/// ```
#[track_caller]
pub fn grid(columns: impl IntoIterator<Item = Track>) -> Element<Panel> {
    panel()
        .display(Display::Grid)
        .grid_columns(columns.into_iter().collect::<Vec<Track>>())
}

/// Creates a container whose children overlap.
///
/// Every child added to the stack is taken out of the flow and stretched to
/// the stack's own bounds, so the children paint on top of each other in the
/// order they were added and the last one covers the ones below it. Because
/// the children do not take part in the flow, they do not size the stack: a
/// stack sizes itself through its own style, such as a fixed
/// [`Element::size`], or through children that stay in the flow.
///
/// ```
/// use smudge_core::geometry::Color;
/// use smudge_core::style::px;
/// use smudge_widgets::stack;
///
/// let layered = stack()
///     .size(smudge_core::geometry::Size {
///         width: 100.0,
///         height: 60.0,
///     })
///     .child(smudge_core::widget::panel().background(Color::WHITE))
///     .child(smudge_core::widget::panel().background(Color::BLACK));
/// ```
#[track_caller]
pub fn stack() -> Element<Panel> {
    panel().style_children(|style: &mut Style| {
        style.layout.position = Position::Absolute;
        style.layout.inset = Edges::all(Length::px(0.0));
    })
}

/// Creates an empty box that takes the leftover space on the main axis.
///
/// Between two children of a [`row`](crate::row) it pushes them apart; inside
/// a [`column`](crate::column) it fills the space below the content.
#[track_caller]
pub fn spacer() -> Element<Panel> {
    panel().grow(1.0)
}
