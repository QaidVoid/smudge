//! The trait every widget implements, and the widgets this crate provides.
//!
//! The tree stores widgets as `Box<dyn Widget>`, so the trait is object safe.
//! A widget never holds a reference to the tree: bindings push updates through
//! a queue, and the frame applies them.

use std::any::Any;

use crate::element::Element;
use crate::geometry::{Rect, Size};
use crate::paint::PaintOp;
use crate::style::{Edges, Style};

pub use taffy::{AvailableSpace, Size as MeasureSize};

/// A piece of user interface owned by one tree node.
///
/// A widget holds its own state and is the only place that state lives. The
/// tree owns the widget and the [`Style`] that positions and paints it.
///
/// Implementing this trait requires the type to be `'static`, because the tree
/// downcasts to the concrete type when a binding updates a widget property.
pub trait Widget: Any {
    /// Returns the size this widget needs, or `None` to let the layout engine
    /// size the node from its style.
    ///
    /// The parameters are the sizes the parent has already fixed and the space
    /// left on each axis. The default returns `None`, which suits a widget that
    /// only paints. A widget that returns a size must return the same size for
    /// the same inputs, because the layout engine caches it until the node is
    /// marked for layout again.
    fn measure(
        &mut self,
        _known: MeasureSize<Option<f32>>,
        _available: MeasureSize<AvailableSpace>,
    ) -> Option<Size> {
        None
    }

    /// Appends this widget's paint operations, in absolute logical coordinates.
    ///
    /// `cx` carries the node's rectangle after layout and its resolved style.
    /// Paint this node only: its children paint themselves.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>);
}

/// What a widget paints from.
pub struct PaintCx<'a> {
    /// The node's absolute rectangle in logical pixels, as the last layout pass
    /// left it.
    pub rect: Rect,
    /// The node's resolved style.
    pub style: &'a Style,
}

/// A container that paints a background and a border and does nothing else.
///
/// A border whose four sides differ in width is painted at the widest side,
/// because a single stroke carries a single width.
pub struct Panel;

/// Creates a [`Panel`] with the default style.
pub fn panel() -> Element<Panel> {
    Element::new(Panel)
}

impl Widget for Panel {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let paint = &cx.style.paint;
        if let Some(color) = paint.background {
            out.push(PaintOp::FillRect {
                rect: cx.rect,
                radius: paint.corner_radius,
                color,
            });
        }
        if let Some(color) = paint.border_color {
            let width = widest_side(cx.style.layout.border_width);
            if width > 0.0 {
                out.push(PaintOp::StrokeRect {
                    rect: cx.rect,
                    radius: paint.corner_radius,
                    width,
                    color,
                });
            }
        }
    }
}

/// Returns the widest of the four border sides, because a stroke has one width.
fn widest_side(border: Edges<f32>) -> f32 {
    border
        .top
        .max(border.right)
        .max(border.bottom)
        .max(border.left)
}

#[cfg(test)]
#[path = "widget_tests.rs"]
mod tests;
