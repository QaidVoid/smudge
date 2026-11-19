use proptest::prelude::*;
use smudge_core::geometry::PixelRect;

use super::Cover;

/// The side of the grid the property draws on, in pixels.
const SIDE: u32 = 24;

/// How many of `rects` hold each pixel of the grid.
fn counts(rects: &[PixelRect]) -> Vec<u32> {
    let mut counts = vec![0; (SIDE * SIDE) as usize];
    for rect in rects {
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                counts[(y * SIDE + x) as usize] += 1;
            }
        }
    }
    counts
}

/// A rectangle inside the grid, empty ones included.
fn rect() -> impl Strategy<Value = PixelRect> {
    (0..SIDE, 0..SIDE).prop_flat_map(|(x, y)| {
        (0..=SIDE - x, 0..=SIDE - y).prop_map(move |(width, height)| PixelRect {
            x,
            y,
            width,
            height,
        })
    })
}

proptest! {
    #[test]
    fn the_cover_holds_each_pixel_of_the_rectangles_once(
        rects in proptest::collection::vec(rect(), 0..8),
    ) {
        let mut cover = Cover::default();
        let covered = counts(cover.of(&rects));
        let wanted: Vec<u32> = counts(&rects).into_iter().map(|count| count.min(1)).collect();
        prop_assert_eq!(covered, wanted);
    }
}

#[test]
fn touching_spans_merge_into_one_rectangle() {
    let mut cover = Cover::default();
    let rects = [
        PixelRect {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        PixelRect {
            x: 4,
            y: 0,
            width: 3,
            height: 2,
        },
    ];
    assert_eq!(
        cover.of(&rects),
        [PixelRect {
            x: 0,
            y: 0,
            width: 7,
            height: 2,
        }]
    );
}
