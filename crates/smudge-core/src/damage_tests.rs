use super::*;
use crate::geometry::{Point, Size};
use proptest::prelude::*;

/// A logical rectangle that partly lies inside and partly outside a small
/// surface.
fn any_logical_rect() -> impl Strategy<Value = Rect> {
    (-8.0f32..24.0, -8.0f32..24.0, 0.0f32..24.0, 0.0f32..24.0).prop_map(|(x, y, width, height)| {
        Rect::from_origin_size(Point { x, y }, Size { width, height })
    })
}

/// A small logical rectangle placed anywhere on a large surface, so that most
/// pairs are too far apart to merge.
fn any_scattered_rect() -> impl Strategy<Value = Rect> {
    (0.0f32..500.0, 0.0f32..500.0, 1.0f32..6.0, 1.0f32..6.0).prop_map(|(x, y, width, height)| {
        Rect::from_origin_size(Point { x, y }, Size { width, height })
    })
}

fn accumulate(rects: &[Rect]) -> DamageAccumulator {
    let mut damage = DamageAccumulator::new();
    for rect in rects {
        damage.add_clipped(*rect, *rect);
    }
    damage
}

proptest! {
    #[test]
    fn finished_damage_covers_every_pixel_the_input_rects_touch(
        rects in prop::collection::vec(any_logical_rect(), 0..6),
        scale in 1.0f32..3.0,
    ) {
        const WIDTH: u32 = 16;
        const HEIGHT: u32 = 16;

        let mut damage = accumulate(&rects);
        let output = damage.finish(scale, WIDTH, HEIGHT);
        for rect in &rects {
            let Some(touched) = PixelRect::from_logical(*rect, scale, WIDTH, HEIGHT) else {
                continue;
            };
            for y in touched.y..touched.y + touched.height {
                for x in touched.x..touched.x + touched.width {
                    let pixel = PixelRect { x, y, width: 1, height: 1 };
                    prop_assert!(
                        output.iter().any(|out| out.contains_rect(pixel)),
                        "damage {:?} misses pixel {:?} touched by {:?} at scale {}",
                        output,
                        (x, y),
                        rect,
                        scale
                    );
                }
            }
        }
    }

    #[test]
    fn finished_damage_is_non_empty_and_inside_the_surface(
        rects in prop::collection::vec(any_logical_rect(), 0..12),
        scale in 0.5f32..3.0,
        surface_width in 1u32..32,
        surface_height in 1u32..32,
    ) {
        let mut damage = accumulate(&rects);
        for rect in damage.finish(scale, surface_width, surface_height) {
            prop_assert!(
                rect.width > 0 && rect.height > 0,
                "empty damage rect {:?}",
                rect
            );
            prop_assert!(
                u64::from(rect.x) + u64::from(rect.width) <= u64::from(surface_width),
                "damage {:?} overflows the surface width {}",
                rect,
                surface_width
            );
            prop_assert!(
                u64::from(rect.y) + u64::from(rect.height) <= u64::from(surface_height),
                "damage {:?} overflows the surface height {}",
                rect,
                surface_height
            );
        }
    }

    #[test]
    fn finished_damage_holds_at_most_the_rect_cap(
        rects in prop::collection::vec(any_scattered_rect(), 0..64),
    ) {
        let mut damage = accumulate(&rects);
        let output = damage.finish(1.0, 512, 512);
        prop_assert!(
            output.len() <= MAX_RECTS,
            "damage holds {} rects, above the cap of {}",
            output.len(),
            MAX_RECTS
        );
    }

    #[test]
    fn many_scattered_rects_stay_covered_under_the_cap(
        rects in prop::collection::vec(any_scattered_rect(), 65..110),
    ) {
        let mut damage = accumulate(&rects);
        let output = damage.finish(1.0, 512, 512);
        prop_assert!(
            output.len() <= MAX_RECTS,
            "damage holds {} rects, above the cap of {}",
            output.len(),
            MAX_RECTS
        );
        for rect in &rects {
            let Some(touched) = PixelRect::from_logical(*rect, 1.0, 512, 512) else {
                continue;
            };
            prop_assert!(
                output.iter().any(|out| out.contains_rect(touched)),
                "damage {:?} misses {:?}",
                output,
                touched
            );
        }
    }
}

#[test]
fn two_crossing_bars_stay_separate() {
    let mut damage = accumulate(&[
        Rect::new(0.0, 45.0, 100.0, 55.0),
        Rect::new(45.0, 0.0, 55.0, 100.0),
    ]);

    assert_eq!(
        damage.finish(1.0, 1000, 1000).len(),
        2,
        "a union that is mostly empty costs more than painting the shared pixels twice"
    );
}

#[test]
fn two_far_apart_small_rects_stay_separate() {
    let mut damage = accumulate(&[
        Rect::new(0.0, 0.0, 20.0, 20.0),
        Rect::new(60.0, 60.0, 80.0, 80.0),
    ]);
    assert_eq!(
        damage.finish(1.0, 100, 100),
        &[
            PixelRect {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            },
            PixelRect {
                x: 60,
                y: 60,
                width: 20,
                height: 20,
            },
        ]
    );
}

#[test]
fn two_overlapping_rects_merge_into_one() {
    let mut damage = accumulate(&[
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(5.0, 5.0, 15.0, 15.0),
    ]);
    assert_eq!(
        damage.finish(1.0, 100, 100),
        &[PixelRect {
            x: 0,
            y: 0,
            width: 15,
            height: 15,
        }]
    );
}

#[test]
fn damage_over_sixty_percent_of_the_surface_becomes_one_rect() {
    let mut damage = accumulate(&[Rect::new(0.0, 0.0, 100.0, 80.0)]);
    assert_eq!(
        damage.finish(1.0, 100, 100),
        &[PixelRect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        }]
    );
}

#[test]
fn scattered_damage_beyond_the_reduce_limit_becomes_one_full_rect() {
    let rects: Vec<Rect> = (0..=REDUCE_LIMIT)
        .map(|index| {
            let x = (index % 16) as f32 * 32.0;
            let y = (index / 16) as f32 * 32.0;
            Rect::new(x, y, x + 2.0, y + 2.0)
        })
        .collect();
    let mut damage = accumulate(&rects);

    assert_eq!(
        damage.finish(1.0, 512, 512),
        &[PixelRect {
            x: 0,
            y: 0,
            width: 512,
            height: 512,
        }],
        "the reduction is cubic in the rect count, so damage past the limit \
         collapses to the surface instead of being reduced"
    );
}

#[test]
fn exactly_the_reduce_limit_rects_still_reduce_to_the_cap() {
    let rects: Vec<Rect> = (0..REDUCE_LIMIT)
        .map(|index| {
            let x = (index % 8) as f32 * 128.0;
            let y = (index / 8) as f32 * 128.0;
            Rect::new(x, y, x + 2.0, y + 2.0)
        })
        .collect();
    let mut damage = accumulate(&rects);
    let output = damage.finish(1.0, 2048, 2048);

    assert_eq!(
        output.len(),
        MAX_RECTS,
        "the reduction still runs at the limit"
    );
    for rect in &rects {
        let pixels = PixelRect {
            x: rect.x0 as u32,
            y: rect.y0 as u32,
            width: 2,
            height: 2,
        };
        assert!(
            output.iter().any(|out| out.contains_rect(pixels)),
            "damage {output:?} misses {pixels:?}"
        );
    }
}

#[test]
fn a_full_frame_is_one_rect_covering_the_surface() {
    let mut damage = DamageAccumulator::new();
    damage.add_clipped(Rect::new(1.0, 1.0, 2.0, 2.0), Rect::new(1.0, 1.0, 2.0, 2.0));
    damage.add_full();
    assert_eq!(
        damage.finish(2.0, 40, 30),
        &[PixelRect {
            x: 0,
            y: 0,
            width: 40,
            height: 30,
        }]
    );
}

#[test]
fn more_than_sixteen_rects_merge_down_to_the_cap() {
    let rects: Vec<Rect> = (0..20)
        .map(|index| {
            let offset = index as f32 * 10.0;
            Rect::new(offset, offset, offset + 1.0, offset + 1.0)
        })
        .collect();
    let mut damage = accumulate(&rects);
    let output = damage.finish(1.0, 200, 200);
    assert!(
        output.len() <= MAX_RECTS,
        "damage holds {} rects",
        output.len()
    );
    for rect in &rects {
        let pixel = PixelRect {
            x: rect.x0 as u32,
            y: rect.y0 as u32,
            width: 1,
            height: 1,
        };
        assert!(
            output.iter().any(|out| out.contains_rect(pixel)),
            "damage {:?} misses pixel {:?}",
            output,
            pixel
        );
    }
}

#[test]
fn an_empty_rect_is_not_damage() {
    let mut damage = accumulate(&[Rect::default(), Rect::new(5.0, 5.0, 5.0, 20.0)]);
    assert!(damage.is_empty());
    assert!(damage.finish(1.0, 100, 100).is_empty());
}

#[test]
fn a_rect_outside_the_surface_is_not_damage() {
    let mut damage = accumulate(&[Rect::new(200.0, 200.0, 300.0, 300.0)]);
    assert!(!damage.is_empty());
    assert!(damage.finish(1.0, 100, 100).is_empty());
}

#[test]
fn damage_partly_outside_the_surface_is_clamped() {
    let mut damage = accumulate(&[Rect::new(-10.0, -10.0, 10.0, 10.0)]);
    assert_eq!(
        damage.finish(1.0, 100, 100),
        &[PixelRect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        }]
    );
}

#[test]
fn a_surface_with_no_pixels_has_no_damage() {
    let mut damage = DamageAccumulator::new();
    damage.add_clipped(
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    assert!(damage.finish(1.0, 0, 100).is_empty());
    damage.add_full();
    assert!(damage.finish(1.0, 100, 0).is_empty());
}

#[test]
fn a_rect_with_a_non_finite_coordinate_is_dropped() {
    let mut damage = accumulate(&[Rect::new(0.0, 0.0, f32::INFINITY, 10.0)]);
    assert!(damage.finish(1.0, 100, 100).is_empty());
}

#[test]
fn clear_removes_collected_damage() {
    let mut damage = DamageAccumulator::new();
    damage.add_full();
    damage.clear();
    assert!(damage.is_empty());
    assert!(damage.finish(1.0, 100, 100).is_empty());
}
