//! Lines, curves and shapes that a widget fills, strokes, or both.
//!
//! A [`Path`] is a list of segments in logical pixels: moves, lines,
//! quadratic and cubic curves, and closes, with arcs, circles and ellipses
//! built from cubic curves. A [`PathPaint`] says how to draw one: a fill
//! color and its [`FillRule`], a [`Stroke`] with its width, joins, caps and
//! dash, or both, the fill first. [`PaintOp::Path`](crate::paint::PaintOp::Path)
//! carries it to the renderers, which draw it the same way to within the
//! antialiasing of each.
//!
//! ```
//! use smudge_core::geometry::{Color, Point};
//! use smudge_core::path::{LineJoin, Path, PathPaint, Stroke};
//!
//! let arrow = Path::new()
//!     .move_to(Point { x: 10.0, y: 10.0 })
//!     .line_to(Point { x: 60.0, y: 30.0 })
//!     .line_to(Point { x: 10.0, y: 50.0 });
//! let paint = PathPaint::stroked(
//!     arrow,
//!     Stroke::new(3.0, Color::rgb(220, 120, 40)).with_join(LineJoin::Round),
//! );
//! assert!(paint.bounds().width() > 50.0);
//! ```

use crate::geometry::{Color, Point, Rect};

/// How far from the origin, in logical pixels, every point of a path must
/// lie for the path to draw. A path that reaches further draws nothing, so
/// the renderers never meet coordinates their arithmetic cannot hold.
pub const PATH_LIMIT: f32 = 1.0e6;

/// The most pieces a dash pattern cuts a path into. A pattern that would cut
/// more, measured along the lines through the path's points, draws the
/// stroke solid, so every renderer draws it the same way and none spends a
/// frame on millions of dashes.
pub const DASH_LIMIT: f32 = 1.0e5;

/// One segment of a [`Path`].
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Segment {
    /// Starts a new subpath at the point.
    MoveTo(Point),
    /// A straight line to the point.
    LineTo(Point),
    /// A quadratic curve through the control point to the end point.
    QuadTo(
        /// The control point.
        Point,
        /// The end point.
        Point,
    ),
    /// A cubic curve through two control points to the end point.
    CubicTo(
        /// The first control point.
        Point,
        /// The second control point.
        Point,
        /// The end point.
        Point,
    ),
    /// A straight line back to the start of the subpath, which closes it.
    Close,
}

/// A sequence of segments in logical pixels, built with the methods that
/// append to it.
///
/// Every subpath starts with a move. A line or a curve appended to an empty
/// path starts a subpath at the origin first, and one appended after a
/// close starts it where the closed subpath started.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    /// The segments, in order.
    segments: Vec<Segment>,
}

impl Path {
    /// Creates an empty path.
    pub fn new() -> Path {
        Path::default()
    }

    /// Returns the path with a new subpath started at `to`.
    pub fn move_to(mut self, to: Point) -> Path {
        self.segments.push(Segment::MoveTo(to));
        self
    }

    /// Returns the path with a straight line to `to` appended.
    pub fn line_to(mut self, to: Point) -> Path {
        self.open_subpath();
        self.segments.push(Segment::LineTo(to));
        self
    }

    /// Returns the path with a quadratic curve through `control` to `to`
    /// appended.
    pub fn quad_to(mut self, control: Point, to: Point) -> Path {
        self.open_subpath();
        self.segments.push(Segment::QuadTo(control, to));
        self
    }

    /// Returns the path with a cubic curve through `first` and `second` to
    /// `to` appended.
    pub fn cubic_to(mut self, first: Point, second: Point, to: Point) -> Path {
        self.open_subpath();
        self.segments.push(Segment::CubicTo(first, second, to));
        self
    }

    /// Returns the path with its current subpath closed. An empty path, or
    /// one whose last subpath is already closed, stays as it is.
    pub fn close(mut self) -> Path {
        if !matches!(self.segments.last(), None | Some(Segment::Close)) {
            self.segments.push(Segment::Close);
        }
        self
    }

    /// Starts a subpath with a move when none is open: at the origin on an
    /// empty path, and where the last subpath started after a close.
    fn open_subpath(&mut self) {
        match self.segments.last() {
            None => self.segments.push(Segment::MoveTo(Point::default())),
            Some(Segment::Close) => {
                let start = self.last_start();
                self.segments.push(Segment::MoveTo(start));
            }
            Some(_) => {}
        }
    }

    /// Returns where the last subpath started, or the origin with none.
    fn last_start(&self) -> Point {
        self.segments
            .iter()
            .rev()
            .find_map(|segment| match segment {
                Segment::MoveTo(to) => Some(*to),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Returns the length of the lines through every point of the path,
    /// control points included, closing lines too, which is never shorter
    /// than the path itself.
    fn outline_length(&self) -> f32 {
        let mut length = 0.0;
        let (mut at, mut start) = (Point::default(), Point::default());
        let mut walk = |from: &mut Point, to: Point| {
            length += ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
            *from = to;
        };
        for segment in &self.segments {
            match *segment {
                Segment::MoveTo(to) => {
                    at = to;
                    start = to;
                }
                Segment::LineTo(to) => walk(&mut at, to),
                Segment::QuadTo(control, to) => {
                    walk(&mut at, control);
                    walk(&mut at, to);
                }
                Segment::CubicTo(first, second, to) => {
                    walk(&mut at, first);
                    walk(&mut at, second);
                    walk(&mut at, to);
                }
                Segment::Close => walk(&mut at, start),
            }
        }
        length
    }

    /// Returns the path with an elliptical arc appended: around `center`,
    /// with the horizontal and vertical radii of `radii`, from the angle
    /// `start` through `sweep`, both in radians, measured from the positive
    /// x axis toward the positive y axis, which points down.
    ///
    /// The arc starts a new subpath at its first point when the path is
    /// empty or its last segment closed, and joins it with a line
    /// otherwise. A sweep of a full turn or more draws the whole ellipse
    /// once.
    pub fn arc(mut self, center: Point, radii: Point, start: f32, sweep: f32) -> Path {
        let at = |angle: f32| Point {
            x: center.x + radii.x * angle.cos(),
            y: center.y + radii.y * angle.sin(),
        };
        let first = at(start);
        let open = matches!(
            self.segments.last(),
            Some(segment) if *segment != Segment::Close
        );
        self.segments.push(if open {
            Segment::LineTo(first)
        } else {
            Segment::MoveTo(first)
        });
        let sweep = sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
        if !sweep.is_finite() || sweep == 0.0 {
            return self;
        }
        // Each piece spans at most a quarter turn, where a cubic stays within
        // a few ten-thousandths of the radius of the true arc.
        let pieces = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0);
        let step = sweep / pieces;
        let handle = 4.0 / 3.0 * (step / 4.0).tan();
        let mut angle = start;
        for _ in 0..pieces as u32 {
            let next = angle + step;
            let (from, to) = (at(angle), at(next));
            let first = Point {
                x: from.x - handle * radii.x * angle.sin(),
                y: from.y + handle * radii.y * angle.cos(),
            };
            let second = Point {
                x: to.x + handle * radii.x * next.sin(),
                y: to.y - handle * radii.y * next.cos(),
            };
            self.segments.push(Segment::CubicTo(first, second, to));
            angle = next;
        }
        self
    }

    /// Creates a path of one straight line from `from` to `to`.
    pub fn line(from: Point, to: Point) -> Path {
        Path::new().move_to(from).line_to(to)
    }

    /// Creates an open path through `points` in order.
    pub fn polyline(points: impl IntoIterator<Item = Point>) -> Path {
        let mut path = Path::new();
        for point in points {
            path = if path.segments.is_empty() {
                path.move_to(point)
            } else {
                path.line_to(point)
            };
        }
        path
    }

    /// Creates a closed path through `points` in order.
    pub fn polygon(points: impl IntoIterator<Item = Point>) -> Path {
        let path = Path::polyline(points);
        if path.segments.is_empty() {
            path
        } else {
            path.close()
        }
    }

    /// Creates a closed circle around `center` with `radius`.
    pub fn circle(center: Point, radius: f32) -> Path {
        Path::ellipse(
            center,
            Point {
                x: radius,
                y: radius,
            },
        )
    }

    /// Creates a closed ellipse around `center` with the horizontal and
    /// vertical radii of `radii`.
    pub fn ellipse(center: Point, radii: Point) -> Path {
        Path::new()
            .arc(center, radii, 0.0, std::f32::consts::TAU)
            .close()
    }

    /// Returns the segments, in order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Returns the path moved by `by`.
    pub fn translated(&self, by: Point) -> Path {
        let moved = |point: Point| Point {
            x: point.x + by.x,
            y: point.y + by.y,
        };
        Path {
            segments: self
                .segments
                .iter()
                .map(|segment| match *segment {
                    Segment::MoveTo(to) => Segment::MoveTo(moved(to)),
                    Segment::LineTo(to) => Segment::LineTo(moved(to)),
                    Segment::QuadTo(control, to) => Segment::QuadTo(moved(control), moved(to)),
                    Segment::CubicTo(first, second, to) => {
                        Segment::CubicTo(moved(first), moved(second), moved(to))
                    }
                    Segment::Close => Segment::Close,
                })
                .collect(),
        }
    }

    /// Returns every point of the path, control points included.
    fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.segments.iter().flat_map(|segment| {
            let (points, count) = match *segment {
                Segment::MoveTo(to) | Segment::LineTo(to) => ([to, to, to], 1),
                Segment::QuadTo(control, to) => ([control, to, to], 2),
                Segment::CubicTo(first, second, to) => ([first, second, to], 3),
                Segment::Close => ([Point::default(); 3], 0),
            };
            points.into_iter().take(count)
        })
    }

    /// Returns whether the path draws: it has a segment, and every point is
    /// finite and within [`PATH_LIMIT`] of the origin.
    pub fn is_drawable(&self) -> bool {
        !self.segments.is_empty()
            && self.points().all(|point| {
                point.x.is_finite()
                    && point.y.is_finite()
                    && point.x.abs() <= PATH_LIMIT
                    && point.y.abs() <= PATH_LIMIT
            })
    }

    /// Returns the smallest rectangle that holds every point of the path,
    /// control points included, which holds every curve too, or an empty
    /// rectangle at the origin when the path does not draw.
    pub fn bounds(&self) -> Rect {
        if !self.is_drawable() {
            return Rect::new(0.0, 0.0, 0.0, 0.0);
        }
        let mut points = self.points();
        let Some(first) = points.next() else {
            return Rect::new(0.0, 0.0, 0.0, 0.0);
        };
        points.fold(
            Rect::new(first.x, first.y, first.x, first.y),
            |bounds, point| {
                Rect::new(
                    bounds.x0.min(point.x),
                    bounds.y0.min(point.y),
                    bounds.x1.max(point.x),
                    bounds.y1.max(point.y),
                )
            },
        )
    }
}

/// Which points a fill covers where a path crosses itself or holds another
/// subpath.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FillRule {
    /// A point is inside when the path winds around it in one direction more
    /// often than in the other, so a subpath drawn the other way cuts a
    /// hole.
    #[default]
    NonZero,
    /// A point is inside when a ray from it crosses the path an odd number
    /// of times, so any subpath inside another cuts a hole.
    EvenOdd,
}

/// How a stroke turns a corner.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineJoin {
    /// The outer edges meet in a point, cut off as a bevel when the point
    /// would reach past the stroke's miter limit.
    #[default]
    Miter,
    /// A circular arc rounds the corner.
    Round,
    /// A straight edge cuts the corner off.
    Bevel,
}

/// How a stroke ends an open subpath and each dash.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineCap {
    /// The stroke ends square at the end point.
    #[default]
    Butt,
    /// A half circle rounds the end.
    Round,
    /// The stroke ends square, half its width past the end point.
    Square,
}

/// A dash pattern: the lengths of the dashes and the gaps between them, in
/// logical pixels, starting with a dash, and how far into the pattern the
/// stroke starts.
///
/// An odd number of lengths repeats once more, so `[4, 2, 1]` dashes as
/// `[4, 2, 1, 4, 2, 1]`. A pattern with a negative or non-finite length,
/// whose lengths add up to nothing, or that would cut its path into more
/// than [`DASH_LIMIT`] pieces, draws the stroke solid.
#[derive(Clone, Debug, PartialEq)]
pub struct Dash {
    /// The lengths of the dashes and gaps, alternating, dash first.
    pub lengths: Vec<f32>,
    /// How far into the pattern the stroke starts.
    pub offset: f32,
}

impl Dash {
    /// Creates a pattern of `lengths` that starts at its beginning.
    pub fn new(lengths: impl Into<Vec<f32>>) -> Dash {
        Dash {
            lengths: lengths.into(),
            offset: 0.0,
        }
    }

    /// Returns the pattern's lengths along `path`, with an odd count
    /// repeated, or `None` when the pattern draws the stroke solid.
    pub fn pattern(&self, path: &Path) -> Option<impl Iterator<Item = f32> + '_> {
        let valid = self
            .lengths
            .iter()
            .all(|length| length.is_finite() && *length >= 0.0);
        let total: f32 = self.lengths.iter().sum();
        if !valid || !total.is_finite() || total <= 0.0 {
            return None;
        }
        let pieces = path.outline_length() / total * self.lengths.len() as f32;
        if !pieces.is_finite() || pieces > DASH_LIMIT {
            return None;
        }
        let count = self.lengths.len() * if self.lengths.len() % 2 == 1 { 2 } else { 1 };
        Some(self.lengths.iter().copied().cycle().take(count))
    }

    /// Returns where the stroke starts in the pattern, or zero when the
    /// offset is not finite.
    pub fn start(&self) -> f32 {
        if self.offset.is_finite() {
            self.offset
        } else {
            0.0
        }
    }
}

/// How a path's outline is drawn.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Stroke {
    /// The width of the line, in logical pixels. A width that is not
    /// finite and positive draws no stroke.
    pub width: f32,
    /// The color of the line.
    pub color: Color,
    /// How the line turns a corner.
    pub join: LineJoin,
    /// How the line ends.
    pub cap: LineCap,
    /// How far a miter join may reach past the corner, as a multiple of
    /// half the width, before it becomes a bevel. Values below one count as
    /// one.
    pub miter_limit: f32,
    /// The dash pattern, or `None` for a solid line.
    pub dash: Option<Dash>,
}

impl Stroke {
    /// Creates a solid stroke `width` logical pixels wide in `color`, with
    /// miter joins limited to four times half the width and butt caps.
    pub fn new(width: f32, color: Color) -> Stroke {
        Stroke {
            width,
            color,
            join: LineJoin::Miter,
            cap: LineCap::Butt,
            miter_limit: 4.0,
            dash: None,
        }
    }

    /// Returns the stroke with `join` corners.
    pub fn with_join(mut self, join: LineJoin) -> Stroke {
        self.join = join;
        self
    }

    /// Returns the stroke with `cap` ends.
    pub fn with_cap(mut self, cap: LineCap) -> Stroke {
        self.cap = cap;
        self
    }

    /// Returns the stroke with miter joins limited to `limit` times half the
    /// width.
    pub fn with_miter_limit(mut self, limit: f32) -> Stroke {
        self.miter_limit = limit;
        self
    }

    /// Returns the stroke dashed by `dash`.
    pub fn with_dash(mut self, dash: Dash) -> Stroke {
        self.dash = Some(dash);
        self
    }

    /// Returns whether the stroke draws anything.
    pub fn draws(&self) -> bool {
        self.width.is_finite() && self.width > 0.0 && self.width <= PATH_LIMIT
    }

    /// Returns the miter limit the renderers draw with: the stroke's, or one
    /// when it is smaller or not finite.
    pub fn effective_miter_limit(&self) -> f32 {
        if self.miter_limit.is_finite() {
            self.miter_limit.clamp(1.0, PATH_LIMIT)
        } else {
            1.0
        }
    }

    /// Returns how far past the path the stroke can reach, in logical
    /// pixels: half its width, times the miter limit for miter joins and the
    /// square root of two for square caps, whichever reaches further, and
    /// at least half a pixel, which a hairline thinner than a pixel covers.
    fn reach(&self) -> f32 {
        let join = match self.join {
            LineJoin::Miter => self.effective_miter_limit(),
            _ => 1.0,
        };
        let cap = match self.cap {
            LineCap::Square => std::f32::consts::SQRT_2,
            _ => 1.0,
        };
        (self.width / 2.0 * join.max(cap)).max(0.5)
    }
}

/// A path and how to draw it: filled, stroked, or both, the fill first.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PathPaint {
    /// The path.
    pub path: Path,
    /// The fill color, or `None` for no fill.
    pub fill: Option<Color>,
    /// Which points the fill covers.
    pub fill_rule: FillRule,
    /// The stroke, or `None` for no outline.
    pub stroke: Option<Stroke>,
}

impl PathPaint {
    /// Creates a paint that fills `path` with `color` by the nonzero rule.
    pub fn filled(path: Path, color: Color) -> PathPaint {
        PathPaint {
            path,
            fill: Some(color),
            fill_rule: FillRule::NonZero,
            stroke: None,
        }
    }

    /// Creates a paint that strokes `path` with `stroke`.
    pub fn stroked(path: Path, stroke: Stroke) -> PathPaint {
        PathPaint {
            path,
            fill: None,
            fill_rule: FillRule::NonZero,
            stroke: Some(stroke),
        }
    }

    /// Returns the paint with `rule` deciding what the fill covers.
    pub fn with_fill_rule(mut self, rule: FillRule) -> PathPaint {
        self.fill_rule = rule;
        self
    }

    /// Returns the paint with `stroke` drawn over the fill.
    pub fn with_stroke(mut self, stroke: Stroke) -> PathPaint {
        self.stroke = Some(stroke);
        self
    }

    /// Returns the paint with `color` filling the path under the stroke.
    pub fn with_fill(mut self, color: Color) -> PathPaint {
        self.fill = Some(color);
        self
    }

    /// Returns the rectangle that holds every pixel the paint can draw: the
    /// path's bounds, grown by how far the stroke reaches past the path. A
    /// path that does not draw has an empty rectangle at the origin.
    pub fn bounds(&self) -> Rect {
        if !self.path.is_drawable() {
            return Rect::new(0.0, 0.0, 0.0, 0.0);
        }
        let bounds = self.path.bounds();
        let reach = self
            .stroke
            .as_ref()
            .filter(|stroke| stroke.draws())
            .map_or(0.0, Stroke::reach);
        Rect::new(
            bounds.x0 - reach,
            bounds.y0 - reach,
            bounds.x1 + reach,
            bounds.y1 + reach,
        )
    }
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
