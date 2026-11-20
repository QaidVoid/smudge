use crate::geometry::{Color, Point};

use super::{Dash, LineCap, LineJoin, PATH_LIMIT, Path, PathPaint, Segment, Stroke};

/// Returns the point of the cubic from `from` through `first` and `second`
/// to `to` at `t`.
fn cubic_at(from: Point, first: Point, second: Point, to: Point, t: f32) -> Point {
    let u = 1.0 - t;
    let mix = |a: f32, b: f32, c: f32, d: f32| {
        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
    };
    Point {
        x: mix(from.x, first.x, second.x, to.x),
        y: mix(from.y, first.y, second.y, to.y),
    }
}

#[test]
fn a_circle_stays_on_its_radius() {
    let center = Point { x: 40.0, y: 30.0 };
    let path = Path::circle(center, 20.0);
    let mut from = None;
    let mut checked = 0;
    for segment in path.segments() {
        match *segment {
            Segment::MoveTo(to) => from = Some(to),
            Segment::CubicTo(first, second, to) => {
                let start = from.expect("the arc starts with a move");
                for step in 0..=8 {
                    let point = cubic_at(start, first, second, to, step as f32 / 8.0);
                    let distance = (point.x - center.x).hypot(point.y - center.y);
                    assert!((distance - 20.0).abs() < 0.02, "{distance}");
                    checked += 1;
                }
                from = Some(to);
            }
            Segment::Close => {}
            _ => panic!("a circle is a move, cubics and a close"),
        }
    }
    assert_eq!(checked, 4 * 9, "four quarter turns");
}

#[test]
fn an_arc_after_a_segment_joins_it_with_a_line() {
    let path = Path::new().move_to(Point { x: 0.0, y: 0.0 }).arc(
        Point { x: 10.0, y: 0.0 },
        Point { x: 5.0, y: 5.0 },
        0.0,
        1.0,
    );
    assert_eq!(
        path.segments()[1],
        Segment::LineTo(Point { x: 15.0, y: 0.0 })
    );
}

#[test]
fn the_bounds_hold_the_stroke_reach_of_joins_and_caps() {
    let path = Path::polyline([Point { x: 10.0, y: 10.0 }, Point { x: 50.0, y: 10.0 }]);
    let wide = |join, cap| {
        PathPaint::stroked(
            path.clone(),
            Stroke::new(4.0, Color::BLACK).with_join(join).with_cap(cap),
        )
        .bounds()
    };
    assert_eq!(wide(LineJoin::Round, LineCap::Round).x0, 8.0);
    let square = wide(LineJoin::Bevel, LineCap::Square).x0;
    assert!((square - (10.0 - 2.0 * std::f32::consts::SQRT_2)).abs() < 1e-5);
    assert_eq!(
        wide(LineJoin::Miter, LineCap::Butt).y0,
        10.0 - 2.0 * 4.0,
        "a miter reaches its limit"
    );
    assert_eq!(
        PathPaint::filled(path, Color::BLACK).bounds().y0,
        10.0,
        "a fill reaches no further than the path"
    );
}

#[test]
fn a_path_out_of_reach_or_not_finite_does_not_draw() {
    let point = |x: f32| Point { x, y: 0.0 };
    assert!(Path::line(point(0.0), point(10.0)).is_drawable());
    assert!(!Path::new().is_drawable());
    assert!(!Path::line(point(0.0), point(f32::NAN)).is_drawable());
    let far = Path::line(point(0.0), point(PATH_LIMIT * 2.0));
    assert!(!far.is_drawable());
    assert_eq!(
        PathPaint::filled(far, Color::BLACK).bounds().width(),
        0.0,
        "so its damage is empty"
    );
}

#[test]
fn an_odd_dash_repeats_and_a_broken_or_too_fine_one_draws_solid() {
    let line = Path::line(Point::default(), Point { x: 100.0, y: 0.0 });
    let pattern = |dash: Dash| dash.pattern(&line).map(Iterator::collect::<Vec<_>>);
    assert_eq!(
        pattern(Dash::new([4.0, 2.0, 1.0])),
        Some(vec![4.0, 2.0, 1.0, 4.0, 2.0, 1.0])
    );
    assert_eq!(pattern(Dash::new([0.0, 0.0])), None);
    assert_eq!(pattern(Dash::new([3.0, -1.0])), None);
    assert_eq!(pattern(Dash::new([3.0, f32::INFINITY])), None);
    assert_eq!(
        pattern(Dash::new([1.0e-6, 1.0e-6])),
        None,
        "a pattern that would cut the line into millions of dashes"
    );
    assert!(pattern(Dash::new([1.0e-3, 1.0e-3])).is_some());
}

#[test]
fn a_line_or_curve_without_a_move_starts_where_a_renderer_would() {
    let lone = Path::new().line_to(Point { x: 5.0, y: 5.0 });
    assert_eq!(lone.segments()[0], Segment::MoveTo(Point::default()));

    let start = Point { x: 10.0, y: 10.0 };
    let after_close = Path::new()
        .move_to(start)
        .line_to(Point { x: 20.0, y: 10.0 })
        .close()
        .quad_to(Point { x: 30.0, y: 0.0 }, Point { x: 40.0, y: 10.0 });
    assert_eq!(after_close.segments()[3], Segment::MoveTo(start));

    assert!(Path::new().close().segments().is_empty());
    assert!(!Path::new().close().is_drawable());
}

#[test]
fn a_hairline_reaches_half_a_pixel_past_its_path() {
    let line = Path::line(Point { x: 10.25, y: 0.0 }, Point { x: 10.25, y: 20.0 });
    let bounds = PathPaint::stroked(line, Stroke::new(0.2, Color::BLACK)).bounds();
    assert!(
        bounds.x0 <= 9.75 && bounds.x1 >= 10.75,
        "{bounds:?} holds the pixel-wide line a thin stroke draws"
    );
}
