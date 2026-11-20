//! A widget that draws what its closure paints: paths, rectangles and text,
//! in its own coordinates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Corners, Point, Rect, Size};
use smudge_core::paint::PaintOp;
use smudge_core::path::{Path, PathPaint, Stroke};
use smudge_core::reactive::{Memo, RwSignal, prelude::*};
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Widget};
use smudge_text::{TextLayout, TextStyle, current_text_style};

/// Where a canvas draws: the top left of its content box, in paint
/// coordinates, the content box's size, and the theme it draws under.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    /// The top left of the content box.
    origin: Point,
    /// The size of the content box.
    size: Size,
    /// The theme of the window the canvas draws in.
    theme: Rc<Theme>,
}

/// What a canvas's closure draws with.
///
/// Its coordinates are the canvas's own, in logical pixels, with the origin
/// at the top left of the content box, inside the border and padding, and
/// [`Painter::size`] the size of that box. Each call appends a drawing in
/// order, so a later one draws over an earlier one.
pub struct Painter {
    /// Where the canvas draws.
    place: Place,
    /// The text style the canvas was built under, which text starts from.
    style: TextStyle,
    /// The operations drawn so far, in paint coordinates.
    ops: Vec<PaintOp>,
    /// Whether the closure asked for the theme.
    themed: Cell<bool>,
}

impl Painter {
    /// Returns the size of the canvas's content box, in logical pixels.
    pub fn size(&self) -> Size {
        self.place.size
    }

    /// Returns the theme of the window the canvas draws in, so its colors
    /// can follow the theme's tokens. The canvas draws again when the theme
    /// changes.
    pub fn theme(&self) -> &Theme {
        self.themed.set(true);
        &self.place.theme
    }

    /// Returns `rect`, in canvas coordinates, in paint coordinates.
    fn placed(&self, rect: Rect) -> Rect {
        let origin = self.place.origin;
        Rect::new(
            rect.x0 + origin.x,
            rect.y0 + origin.y,
            rect.x1 + origin.x,
            rect.y1 + origin.y,
        )
    }

    /// Draws `paint`: its path filled, stroked, or both.
    pub fn draw(&mut self, paint: PathPaint) {
        let mut paint = paint;
        paint.path = paint.path.translated(self.place.origin);
        self.ops.push(PaintOp::Path(Rc::new(paint)));
    }

    /// Fills `path` with `color`.
    pub fn fill(&mut self, path: &Path, color: Color) {
        self.draw(PathPaint::filled(path.clone(), color));
    }

    /// Strokes `path` with `stroke`.
    pub fn stroke(&mut self, path: &Path, stroke: Stroke) {
        self.draw(PathPaint::stroked(path.clone(), stroke));
    }

    /// Fills `rect` with `color`, its corners rounded by `radius`.
    pub fn fill_rect(&mut self, rect: Rect, radius: Corners, color: Color) {
        let rect = self.placed(rect);
        self.ops.push(PaintOp::FillRect {
            rect,
            radius,
            color,
        });
    }

    /// Strokes the inside of `rect` with a line `width` wide in `color`,
    /// its corners rounded by `radius`.
    pub fn stroke_rect(&mut self, rect: Rect, radius: Corners, width: f32, color: Color) {
        let rect = self.placed(rect);
        self.ops.push(PaintOp::StrokeRect {
            rect,
            radius,
            width,
            color,
        });
    }

    /// Draws `text` on one line in the canvas's font at `size` logical
    /// pixels in `color`, with the top left of its box at `at`, and returns
    /// the box's size.
    ///
    /// # Panics
    ///
    /// Panics when no text system is installed on the thread, as laying text
    /// out anywhere does.
    pub fn text(&mut self, text: &str, at: Point, size: f32, color: Color) -> Size {
        let style = TextStyle {
            size,
            ..self.style.clone()
        };
        let layout = TextLayout::new(text, &style, None);
        let origin = Point {
            x: at.x + self.place.origin.x,
            y: at.y + self.place.origin.y,
        };
        self.ops.extend(
            layout
                .glyph_runs_at(origin, color)
                .into_iter()
                .map(|run| PaintOp::Glyphs(Rc::new(run))),
        );
        layout.size()
    }
}

/// The widget [`canvas`] builds.
pub struct Canvas {
    /// Where the canvas drew last, or `None` before its first paint, which
    /// the closure draws at.
    place: RwSignal<Option<Place>>,
    /// What the closure draws at `place`, run again when `place` or a
    /// signal the closure read changes.
    drawing: Memo<Option<Drawing>>,
    /// What the canvas painted last, which a change from the binding only
    /// repaints when it differs from.
    painted: RefCell<Option<Drawing>>,
}

/// What a canvas's closure drew.
#[derive(Clone, PartialEq)]
struct Drawing {
    /// The operations, in paint coordinates.
    ops: Rc<Vec<PaintOp>>,
    /// Whether the closure asked for the theme, so it draws again when the
    /// theme changes.
    themed: bool,
}

/// Runs `draw` at `place` and returns what it drew.
fn record(draw: &dyn Fn(&mut Painter), style: &TextStyle, place: Place) -> Drawing {
    let mut painter = Painter {
        place,
        style: style.clone(),
        ops: Vec::new(),
        themed: Cell::new(false),
    };
    draw(&mut painter);
    Drawing {
        ops: Rc::new(painter.ops),
        themed: painter.themed.get(),
    }
}

/// The key of the binding that repaints a canvas when what it read changed.
struct Redraw;

impl Canvas {
    /// Moves the place the closure draws at to `origin` and `size` under
    /// `theme`, keeping the copy of the theme it holds while that is the
    /// same.
    fn place_at(&self, origin: Point, size: Size, theme: &Theme) {
        let kept = self.place.with(|known| {
            known
                .as_ref()
                .filter(|known| *known.theme == *theme)
                .map(|known| Rc::clone(&known.theme))
        });
        let theme = kept.unwrap_or_else(|| Rc::new(theme.clone()));
        self.place.set(Some(Place {
            origin,
            size,
            theme,
        }));
    }
}

impl Widget for Canvas {
    /// Paints nothing outside the canvas's border: what the closure drew
    /// goes inside it.
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    /// Paints what the closure drew at the canvas's place, clipped to the
    /// box inside its border, moving the place first when the canvas moved,
    /// resized or the theme changed, which draws it there.
    fn paint_inside(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let (origin, size) = (
            cx.content.origin(),
            Size {
                width: cx.content.width(),
                height: cx.content.height(),
            },
        );
        let moved = self.place.with(|known| {
            known
                .as_ref()
                .is_none_or(|known| known.origin != origin || known.size != size)
        });
        if moved {
            self.place_at(origin, size, cx.theme);
        }
        let mut drawing = self.drawing.get();
        // A drawing that asked for the theme draws again under a new one,
        // whether it asked before or only now.
        let stale = drawing.as_ref().is_some_and(|drawing| drawing.themed)
            && self.place.with(|known| {
                known
                    .as_ref()
                    .is_some_and(|known| *known.theme != *cx.theme)
            });
        if stale {
            self.place_at(origin, size, cx.theme);
            drawing = self.drawing.get();
        }
        if let Some(drawing) = &drawing {
            out.extend(drawing.ops.iter().cloned());
            // The closure read a copy of the theme, so resolving a token of
            // the window's own records that a theme switch repaints the
            // canvas.
            if drawing.themed {
                cx.theme.color(ColorToken::TEXT);
            }
        }
        *self.painted.borrow_mut() = drawing;
    }
}

/// Creates a canvas that draws what `draw` paints, in its own coordinates.
///
/// `draw` runs with a [`Painter`] whose origin is the top left of the
/// canvas's content box, and draws again whenever a signal it read
/// changes, the canvas moves or resizes, or the theme changes after the
/// closure asked for it with [`Painter::theme`]. What it draws past its
/// box is cut off at the canvas's border. The canvas takes no size of its
/// own, so give it one, or let its parent stretch it.
///
/// ```
/// use smudge_core::geometry::{Color, Point};
/// use smudge_core::path::{Path, Stroke};
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_widgets::canvas;
///
/// let angle = RwSignal::new(0.0_f32);
/// let dial = canvas(move |painter| {
///     let size = painter.size();
///     let center = Point {
///         x: size.width / 2.0,
///         y: size.height / 2.0,
///     };
///     let radius = size.width.min(size.height) / 2.0 - 2.0;
///     painter.stroke(&Path::circle(center, radius), Stroke::new(2.0, Color::BLACK));
///     let tip = Point {
///         x: center.x + radius * angle.get().cos(),
///         y: center.y + radius * angle.get().sin(),
///     };
///     painter.stroke(&Path::line(center, tip), Stroke::new(3.0, Color::BLACK));
/// })
/// .width(px(64.0))
/// .height(px(64.0));
/// ```
#[track_caller]
pub fn canvas(draw: impl Fn(&mut Painter) + 'static) -> Element<Canvas> {
    let style = current_text_style();
    let place = crate::signal(None::<Place>);
    let known = place.clone();
    // The closure runs only here, so a change of what it read and a move of
    // the canvas each draw once: the binding reads what the memo holds, and
    // paint, which moves the place, reads it again without running it.
    let drawing = Memo::new(move || known.get().map(|place| record(&draw, &style, place)));
    let watched = drawing.clone();
    Element::new(Canvas {
        place,
        drawing,
        painted: RefCell::new(None),
    })
    .bind(
        Redraw,
        move || watched.get(),
        |canvas, drawing| {
            let painted = canvas.painted.borrow();
            let same = match (&*painted, &drawing) {
                (Some(painted), Some(drawing)) => Rc::ptr_eq(&painted.ops, &drawing.ops),
                (None, None) => true,
                _ => false,
            };
            if same { Dirty::Clean } else { Dirty::Paint }
        },
    )
}
