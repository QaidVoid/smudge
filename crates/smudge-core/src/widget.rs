//! The trait every widget implements, and the widgets this crate provides.
//!
//! The tree stores widgets as `Box<dyn Widget>`, so the trait is object safe.
//! A widget never holds a reference to the tree: bindings push updates through
//! a queue, and the frame applies them.

use std::any::Any;

use crate::element::Element;
use crate::geometry::{Corners, Rect, Size};
use crate::paint::PaintOp;
use crate::style::{Edges, Style};
use crate::theme::Theme;

/// The space a measure call offers on one axis, or the intrinsic size it asks
/// for instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvailableSpace {
    /// Space offered in logical pixels.
    Definite(f32),
    /// Asks for the smallest size the content fits in, such as the widest
    /// word of a text.
    MinContent,
    /// Asks for the size the content takes without wrapping.
    MaxContent,
}

/// What one measure call offers a widget.
#[derive(Clone, Copy, Debug)]
pub struct MeasureInput {
    /// The width the layout engine treats as fixed for this call.
    ///
    /// It can be absent even when the node's style fixes a width: the
    /// engine's final layout pass passes no fixed size at all.
    pub known_width: Option<f32>,
    /// The height the layout engine treats as fixed for this call.
    ///
    /// It can be absent even when the node's style fixes a height: the
    /// engine's final layout pass passes no fixed size at all.
    pub known_height: Option<f32>,
    /// The space offered on the x axis, inside the node's padding and border.
    pub available_width: AvailableSpace,
    /// The space offered on the y axis, inside the node's padding and border.
    pub available_height: AvailableSpace,
}

/// A piece of user interface owned by one tree node.
///
/// A widget holds its own state and is the only place that state lives. The
/// tree owns the widget and the [`Style`] that positions and paints it.
///
/// Implementing this trait requires the type to be `'static`, because the tree
/// downcasts to the concrete type when a binding updates a widget property.
/// The `TypeName` bound is met by every type without any code, and supplies
/// the name of the widget's type that diagnostics print.
pub trait Widget: Any + type_name::TypeName {
    /// Returns the size of this widget's content, or `None` to let the layout
    /// engine size the node from its style.
    ///
    /// `input.known_width` and `input.known_height` hold the sizes the engine
    /// treats as fixed for this call, and either can be absent even when the
    /// node's style fixes a size. `input.available_width` and
    /// `input.available_height` are the space left on each axis inside the
    /// node's padding and border. An axis can also ask for an intrinsic size
    /// instead of offering space: [`AvailableSpace::MinContent`] asks for the
    /// smallest size the content fits in, such as the widest word of a text,
    /// and [`AvailableSpace::MaxContent`] asks for the size the content takes
    /// without wrapping. The returned size is the content size, so the engine
    /// adds padding and border around it.
    ///
    /// The engine may call this several times in one layout pass.
    ///
    /// The default returns `None`, which suits a widget that only paints. A
    /// widget that returns a size must return the same size for the same
    /// inputs, because the layout engine caches it until the node is marked
    /// for layout again.
    fn measure(&mut self, _input: MeasureInput) -> Option<Size> {
        None
    }

    /// Appends this widget's paint operations, in absolute logical coordinates.
    ///
    /// `cx` carries the node's rectangle after layout and its resolved style.
    /// Paint this node only: its children paint themselves.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>);

    /// Returns the rectangle the input method should anchor to while this
    /// widget holds the keyboard focus, in window coordinates.
    ///
    /// The app asks only the focused widget and reports [`None`] when nothing
    /// is focused or the widget has no area to report. The default reports
    /// [`None`], which suits a widget that takes no text.
    fn ime_area(&self) -> Option<Rect> {
        None
    }
}

/// The name of a widget's concrete type, which diagnostics print.
///
/// The trait is implemented for every type and cannot be named outside this
/// crate, so a widget can neither implement nor override it.
pub(crate) mod type_name {
    /// Reports the name of the implementing type.
    pub trait TypeName {
        /// Returns the full name of the implementing type, such as
        /// `smudge_core::widget::Panel`.
        fn type_name(&self) -> &'static str;
    }

    impl<T> TypeName for T {
        fn type_name(&self) -> &'static str {
            std::any::type_name::<T>()
        }
    }
}

/// What a widget paints from.
pub struct PaintCx<'a> {
    /// The node's absolute rectangle in logical pixels, as the last layout pass
    /// left it.
    pub rect: Rect,
    /// The node's resolved style.
    pub style: &'a Style,
    /// The theme that style values which name a token resolve against.
    pub theme: &'a Theme,
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
        // The radius is resolved only for an operation that uses it, so a node
        // that paints nothing never consults the theme and a theme switch
        // leaves it alone.
        if let Some(background) = &paint.background {
            out.push(PaintOp::FillRect {
                rect: cx.rect,
                radius: Corners::all(paint.corner_radius.resolve(cx.theme)),
                color: background.resolve(cx.theme),
            });
        }
        if let Some(border) = &paint.border_color {
            let width = widest_side(cx.style.layout.border_width);
            if width > 0.0 {
                out.push(PaintOp::StrokeRect {
                    rect: cx.rect,
                    radius: Corners::all(paint.corner_radius.resolve(cx.theme)),
                    width,
                    color: border.resolve(cx.theme),
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
