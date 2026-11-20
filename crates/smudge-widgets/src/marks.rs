//! Small marks the widgets draw as paths, in the text color of their node:
//! a check mark and chevrons.

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::element::{Element, Value};
use smudge_core::geometry::{Color, Point, Rect};
use smudge_core::paint::PaintOp;
use smudge_core::path::{LineCap, LineJoin, Path, PathPaint, Stroke};
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Widget};

/// The shape of a [`Mark`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A tick.
    Check,
    /// A chevron that points down.
    Down,
    /// A chevron that points up.
    Up,
}

/// A small mark drawn as a stroked path across its content box.
pub(crate) struct Mark {
    /// What the mark draws.
    shape: Shape,
    /// Whether the mark shows.
    shown: bool,
    /// The mark drawn last, with the box and the color it was drawn for,
    /// which a paint of the same box and color draws again.
    drawn: RefCell<Option<(Rect, Color, PaintOp)>>,
}

/// The key of a mark's binding.
struct Shown;

impl Widget for Mark {
    /// Strokes the mark while it shows, in the node's text color, or the
    /// theme's text color without one.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        if !self.shown {
            return;
        }
        let color = cx
            .style
            .paint
            .text_color
            .unwrap_or(ColorValue::Token(ColorToken::TEXT))
            .resolve(cx.theme);
        let area = cx.content;
        let mut drawn = self.drawn.borrow_mut();
        if let Some((rect, ink, op)) = &*drawn
            && *rect == area
            && *ink == color
        {
            out.push(op.clone());
            return;
        }
        let at = |x: f32, y: f32| Point {
            x: area.x0 + x * area.width(),
            y: area.y0 + y * area.height(),
        };
        let (points, width) = match self.shape {
            Shape::Check => ([at(0.15, 0.55), at(0.4, 0.78), at(0.85, 0.25)], 1.8),
            Shape::Down => ([at(0.2, 0.35), at(0.5, 0.65), at(0.8, 0.35)], 1.5),
            Shape::Up => ([at(0.2, 0.65), at(0.5, 0.35), at(0.8, 0.65)], 1.5),
        };
        let op = PaintOp::Path(Rc::new(PathPaint::stroked(
            Path::polyline(points),
            Stroke::new(width, color)
                .with_cap(LineCap::Round)
                .with_join(LineJoin::Round),
        )));
        out.push(op.clone());
        *drawn = Some((area, color, op));
    }
}

/// Builds a mark of `shape`, `size` logical pixels square, that shows while
/// `shown` reads `true`.
pub(crate) fn mark(shape: Shape, size: f32, shown: impl Into<Value<bool>>) -> Element<Mark> {
    Element::new(Mark {
        shape,
        shown: false,
        drawn: RefCell::new(None),
    })
    .bind(Shown, shown, |mark, shown| {
        mark.shown = shown;
        Dirty::Paint
    })
    .width(px(size))
    .height(px(size))
    .shrink(0.0)
}
