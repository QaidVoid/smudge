use super::*;
use proptest::prelude::*;

fn any_rect() -> impl Strategy<Value = Rect> {
    (
        -1_000.0f32..1_000.0,
        -1_000.0f32..1_000.0,
        -1_000.0f32..1_000.0,
        -1_000.0f32..1_000.0,
    )
        .prop_map(|(x0, y0, x1, y1)| Rect::new(x0, y0, x1, y1))
}

fn any_non_empty_rect() -> impl Strategy<Value = Rect> {
    (
        -1_000.0f32..1_000.0,
        -1_000.0f32..1_000.0,
        0.001f32..2_000.0,
        0.001f32..2_000.0,
    )
        .prop_map(|(x, y, width, height)| {
            Rect::from_origin_size(Point { x, y }, Size { width, height })
        })
}

fn corners(rect: Rect) -> [Point; 4] {
    [
        Point {
            x: rect.x0,
            y: rect.y0,
        },
        Point {
            x: rect.x1,
            y: rect.y0,
        },
        Point {
            x: rect.x0,
            y: rect.y1,
        },
        Point {
            x: rect.x1,
            y: rect.y1,
        },
    ]
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    !inner.is_empty()
        && inner.x0 >= outer.x0
        && inner.y0 >= outer.y0
        && inner.x1 <= outer.x1
        && inner.y1 <= outer.y1
}

proptest! {
    #[test]
    fn union_contains_every_corner_of_both_rects(
        a in any_non_empty_rect(),
        b in any_non_empty_rect(),
    ) {
        let union = a.union(b);
        for corner in corners(a).into_iter().chain(corners(b)) {
            prop_assert!(
                union.contains(corner),
                "union {:?} of {:?} and {:?} misses {:?}",
                union,
                a,
                b,
                corner
            );
        }
    }

    #[test]
    fn intersect_is_empty_or_inside_both_rects(a in any_rect(), b in any_rect()) {
        let intersection = a.intersect(b);
        if !intersection.is_empty() {
            prop_assert!(
                contains_rect(a, intersection),
                "intersection {:?} of {:?} and {:?} escapes the first",
                intersection,
                a,
                b
            );
            prop_assert!(
                contains_rect(b, intersection),
                "intersection {:?} of {:?} and {:?} escapes the second",
                intersection,
                a,
                b
            );
        }
    }

    #[test]
    fn from_logical_covers_every_pixel_the_rect_touches(
        rect in (-16.0f32..48.0, -16.0f32..48.0, -16.0f32..48.0, -16.0f32..48.0)
            .prop_map(|(x0, y0, x1, y1)| Rect::new(x0, y0, x1, y1)),
        scale in 0.5f32..4.0,
        surface_width in 1u32..64,
        surface_height in 1u32..64,
    ) {
        let scaled = Rect::new(
            rect.x0 * scale,
            rect.y0 * scale,
            rect.x1 * scale,
            rect.y1 * scale,
        );
        let covered = PixelRect::from_logical(rect, scale, surface_width, surface_height);
        for y in 0..surface_height {
            for x in 0..surface_width {
                let pixel = Rect::new(x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0);
                if scaled.intersect(pixel).is_empty() {
                    continue;
                }
                let touched = PixelRect { x, y, width: 1, height: 1 };
                prop_assert!(
                    covered.is_some_and(|covered| covered.contains_rect(touched)),
                    "damage {:?} misses pixel {:?} touched by {:?} at scale {}",
                    covered,
                    (x, y),
                    scaled,
                    scale
                );
            }
        }
    }

    #[test]
    fn from_logical_never_extends_beyond_the_surface(
        rect in any_rect(),
        scale in 0.5f32..4.0,
        surface_width in 1u32..512,
        surface_height in 1u32..512,
    ) {
        if let Some(pixel_rect) = PixelRect::from_logical(rect, scale, surface_width, surface_height) {
            prop_assert!(pixel_rect.width > 0 && pixel_rect.height > 0, "empty damage {:?}", pixel_rect);
            prop_assert!(
                u64::from(pixel_rect.x) + u64::from(pixel_rect.width) <= u64::from(surface_width),
                "damage {:?} overflows the surface width {}",
                pixel_rect,
                surface_width
            );
            prop_assert!(
                u64::from(pixel_rect.y) + u64::from(pixel_rect.height) <= u64::from(surface_height),
                "damage {:?} overflows the surface height {}",
                pixel_rect,
                surface_height
            );
        }
    }
}

#[test]
fn union_ignores_an_empty_rect() {
    let rect = Rect::new(1.0, 2.0, 3.0, 4.0);
    assert_eq!(rect.union(Rect::default()), rect);
    assert_eq!(Rect::default().union(rect), rect);
    assert!(Rect::default().union(Rect::default()).is_empty());
}

#[test]
fn pixel_union_ignores_an_empty_rect() {
    let rect = PixelRect {
        x: 10,
        y: 20,
        width: 5,
        height: 5,
    };
    assert_eq!(PixelRect::default().union(rect), rect);
    assert_eq!(rect.union(PixelRect::default()), rect);
}

#[test]
fn from_logical_returns_none_for_an_unusable_scale() {
    let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(PixelRect::from_logical(rect, scale, 50, 50), None);
    }
}

#[test]
fn from_logical_returns_none_for_a_rect_outside_the_surface() {
    let right_of_surface = Rect::new(100.0, 100.0, 200.0, 200.0);
    let left_of_surface = Rect::new(-200.0, -200.0, -100.0, -100.0);
    assert_eq!(PixelRect::from_logical(right_of_surface, 1.0, 50, 50), None);
    assert_eq!(PixelRect::from_logical(left_of_surface, 1.0, 50, 50), None);
}

#[test]
fn from_logical_returns_none_for_a_non_finite_coordinate() {
    let nan_x0 = Rect::new(f32::NAN, 0.0, 10.0, 10.0);
    let infinite_x1 = Rect::new(0.0, 0.0, f32::INFINITY, 10.0);
    assert_eq!(PixelRect::from_logical(nan_x0, 1.0, 50, 50), None);
    assert_eq!(PixelRect::from_logical(infinite_x1, 1.0, 50, 50), None);
}

#[test]
fn from_logical_returns_none_for_a_zero_width_rect() {
    let zero_width = Rect::new(10.0, 10.0, 10.0, 20.0);
    let zero_height = Rect::new(10.0, 10.0, 20.0, 10.0);
    assert_eq!(PixelRect::from_logical(zero_width, 1.0, 50, 50), None);
    assert_eq!(PixelRect::from_logical(zero_height, 1.0, 50, 50), None);
}

#[test]
fn from_logical_clamps_a_partly_visible_rect() {
    let rect = Rect::new(-10.0, -10.0, 5.0, 5.0);
    assert_eq!(
        PixelRect::from_logical(rect, 1.0, 20, 20),
        Some(PixelRect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        })
    );
}

#[test]
fn from_logical_rounds_outward_at_a_fractional_scale() {
    let rect = Rect::new(1.0, 1.0, 2.0, 2.0);
    assert_eq!(
        PixelRect::from_logical(rect, 0.5, 10, 10),
        Some(PixelRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        })
    );
}
