//! Whole-frame behavior of the virtual list, driven through the headless
//! harness.
//!
//! The list measures its viewport when the view paints, so the window of rows
//! mounts a frame after the view first lays out: every test settles the
//! harness after mounting and after every edit. A scrolled or resized list is
//! checked against a full repaint after every one of those frames, and in the
//! end against a plain scroll view that mounts every row at the same offset,
//! so a window that misses a visible row shows up as a pixel difference.

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use smudge_core::geometry::{Point, Size};
use smudge_core::input::InputEvent;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};

use smudge_widgets::{ScrollExt, column, scroll, virtual_list};

/// The height of one row, in logical pixels.
const ROW_HEIGHT: f32 = 24.0;

/// The height of the list, in logical pixels. Twenty rows fit exactly.
const VIEWPORT_HEIGHT: f32 = 480.0;

/// The window the list sits in. Its width gives the rows room; its height
/// matches the list, so nothing below the list can be mistaken for a row.
const WINDOW: Size = Size {
    width: 640.0,
    height: 480.0,
};

/// Where the tests park the pointer, over the middle of the viewport, so the
/// wheel events reach the list.
const POINTER: Point = Point { x: 320.0, y: 240.0 };

/// The number of cases the property test runs when `PROPTEST_CASES` is unset.
const DEFAULT_CASES: u32 = 64;

/// The proptest configuration the gate runs.
///
/// A failure is written to `tests/proptest-regressions/`, beside the test
/// that found it, and that file is kept under version control.
fn config() -> ProptestConfig {
    ProptestConfig {
        cases: DEFAULT_CASES,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/proptest-regressions/virtual_list.txt"
        )))),
        ..ProptestConfig::default()
    }
}

/// Builds a list of `count` rows of labels in a settled harness, together
/// with the signal that holds its row count.
fn list_harness(count: RwSignal<usize>) -> Harness {
    let bound = count.clone();
    Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        panel().column().child(
            virtual_list(
                move || bound.get(),
                ROW_HEIGHT,
                |index| label(format!("Row {index}")),
            )
            .height(px(VIEWPORT_HEIGHT)),
        )
    })
}

/// Builds a plain scroll view holding all `count` rows, scrolled to
/// `offset`, which is what the virtual list must look like.
fn full_list_harness(count: usize, offset: f32) -> Harness {
    Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        let rows = (0..count).fold(column(), |rows, index| {
            rows.child(
                panel()
                    .height(px(ROW_HEIGHT))
                    .child(label(format!("Row {index}"))),
            )
        });
        panel().column().child(
            scroll(rows)
                .offset(RwSignal::new(offset))
                .height(px(VIEWPORT_HEIGHT)),
        )
    })
}

/// Wheels the list by `down` logical pixels, scrolling down when positive.
fn wheel(harness: &mut Harness, down: f32) {
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: down },
    });
}

/// Runs one edit, then checks every frame it takes against a full repaint.
fn apply_edit(edit: &Edit, count: &RwSignal<usize>, harness: &mut Harness) {
    match *edit {
        Edit::Wheel(down) => wheel(harness, down),
        Edit::Count(rows) => count.set(rows),
    }
    harness.frame();
    assert_settled(harness);
    harness.assert_matches_full_repaint();
    settle(harness);
}

/// One edit of the random sequence: a wheel step or a new row count.
#[derive(Debug, Clone)]
enum Edit {
    /// A wheel step, in logical pixels, positive for scrolling down.
    Wheel(f32),
    /// A new row count.
    Count(usize),
}

/// Wheel steps are the common edit, and counts change less often. Counts stay
/// small because the full list the result is compared with mounts every row.
fn edit_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => (-600.0f32..6_000.0).prop_map(Edit::Wheel),
        1 => (0usize..400usize).prop_map(Edit::Count),
    ]
}

/// Panics when the app still holds work that only a frame can finish.
fn assert_settled(harness: &mut Harness) {
    assert!(
        !harness.app().needs_frame(),
        "the frame left work pending, so the screen would not be settled \
         until another frame ran"
    );
}

/// Runs the frames that a measurement made during a frame asks for, and
/// checks each one like the frame before it.
///
/// The list learns its viewport when the view paints, so a count change
/// reaches the mounted window one frame after the frame that laid the new
/// height out. The measurement wakes the app, and the frames run until no
/// write wakes it.
fn settle(harness: &mut Harness) {
    for _ in 0..4 {
        if !harness.woken() {
            return;
        }
        harness.frame();
        assert_settled(harness);
        harness.assert_matches_full_repaint();
    }
    panic!("the list still woke the app after four frames, so it never settles");
}

/// The vertical position of every mounted label, from the layout dump.
fn label_rows(harness: &Harness) -> Vec<f32> {
    harness
        .layout_dump()
        .lines()
        .filter_map(|line| {
            let start = line.find("Label [")?;
            let numbers = &line[start + "Label [".len()..line.len() - 1];
            numbers.split(", ").nth(1)?.parse().ok()
        })
        .collect()
}

/// The indices of the mounted rows, read back from the labels' positions.
///
/// The rows sit at `index * row_height` inside content that the list has
/// scrolled by `offset`, so a label's position names its row.
fn mounted_indices(harness: &Harness, offset: f32) -> Vec<usize> {
    let mut indices: Vec<usize> = label_rows(harness)
        .into_iter()
        .map(|y0| ((y0 + offset) / ROW_HEIGHT).round() as usize)
        .collect();
    indices.sort_unstable();
    indices
}

#[test]
fn a_hundred_thousand_rows_mount_a_bounded_window() {
    TextSystem::bundled_only().install();
    let count = RwSignal::new(100_000);
    let mut harness = list_harness(count.clone());
    harness.frame();
    harness.settle();

    let mounted = label_rows(&harness).len();
    assert!(mounted <= 28, "the top of the list mounted {mounted} rows");

    harness.move_to(POINTER);
    // Two steps of exactly six hundred rows land the offset on a row
    // boundary in the middle, at 1,200,000.
    wheel(&mut harness, 600_000.0);
    harness.frame();
    harness.settle();
    wheel(&mut harness, 600_000.0);
    harness.frame();
    harness.settle();

    let mounted = label_rows(&harness).len();
    assert!(
        mounted <= 28,
        "the middle of the list mounted {mounted} rows"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn scrolling_to_the_middle_mounts_the_rows_around_it() {
    TextSystem::bundled_only().install();
    let count = RwSignal::new(100_000);
    let mut harness = list_harness(count.clone());
    harness.frame();
    harness.settle();
    harness.move_to(POINTER);

    wheel(&mut harness, 600_000.0);
    harness.frame();
    harness.settle();
    wheel(&mut harness, 600_000.0);
    harness.frame();
    harness.settle();

    // The viewport starts at row 50,000 and the overscan reaches four rows
    // past it in both directions.
    let expected: Vec<usize> = (49_996..50_024).collect();
    assert_eq!(
        mounted_indices(&harness, 1_200_000.0),
        expected,
        "the middle of the list mounted the wrong rows"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_scroll_inside_one_row_moves_the_rows_without_remounting() {
    TextSystem::bundled_only().install();
    let count = RwSignal::new(100_000);
    let mut harness = list_harness(count.clone());
    harness.frame();
    harness.settle();
    harness.move_to(POINTER);

    // The first step below a row mounts the row the sliver reveals, and the
    // window settles at 25 rows, overscan included.
    wheel(&mut harness, 10.0);
    harness.frame();
    harness.settle();
    let before = mounted_indices(&harness, 10.0);

    // A second step inside the same row moves every row and mounts nothing.
    wheel(&mut harness, 10.0);
    let report = harness.frame();

    assert_eq!(
        mounted_indices(&harness, 20.0),
        before,
        "a scroll inside one row changed the mounted window"
    );
    let rows = label_rows(&harness);
    assert_eq!(rows.first().copied(), Some(-20.0), "the rows moved up");
    assert!(
        !report.layout_ran,
        "a scroll that mounts nothing needs no layout pass"
    );
    harness.settle();
    harness.assert_matches_full_repaint();
}

#[test]
fn a_shrinking_count_clamps_the_offset_and_shows_the_first_rows() {
    TextSystem::bundled_only().install();
    let count = RwSignal::new(100_000);
    let mut harness = list_harness(count.clone());
    harness.frame();
    harness.settle();
    harness.move_to(POINTER);

    wheel(&mut harness, 1_000_000_000.0);
    harness.frame();
    harness.settle();
    assert_eq!(
        mounted_indices(&harness, 2399520.0),
        (99_976..100_000).collect::<Vec<usize>>(),
        "the end of the list mounted the wrong rows"
    );

    count.set(10);
    harness.frame();
    harness.settle();

    assert_eq!(
        mounted_indices(&harness, 0.0),
        (0..10).collect::<Vec<usize>>(),
        "the shrunken list clamped to the top and shows rows 0 to 9"
    );
    let first_row = label_rows(&harness).first().copied();
    assert_eq!(first_row, Some(0.0), "the first row sits at the top");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_row_height_that_is_not_positive_mounts_no_rows() {
    TextSystem::bundled_only().install();
    for row_height in [-24.0, f32::NAN, 0.0] {
        let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
            panel().column().child(
                virtual_list(
                    || 100_000,
                    row_height,
                    |index| label(format!("Row {index}")),
                )
                .height(px(VIEWPORT_HEIGHT)),
            )
        });
        harness.frame();
        harness.settle();
        assert_eq!(
            label_rows(&harness).len(),
            0,
            "a row height of {row_height} mounted rows"
        );
    }
}

// The damage equivalence property over random scroll and count edits.
proptest! {
    #![proptest_config(config())]
    #[test]
    fn damaged_frames_match_full_repaints_and_the_full_list(edits in prop::collection::vec(edit_strategy(), 1..=8)) {
        TextSystem::bundled_only().install();
        let count = RwSignal::new(300usize);
        let mut harness = list_harness(count.clone());
        harness.frame();
        assert_settled(&mut harness);
        harness.assert_matches_full_repaint();
        settle(&mut harness);
        harness.move_to(POINTER);

        // The view clamps every offset to the content below the viewport,
        // and so does the model the full list is scrolled by.
        let mut offset = 0.0f32;
        for edit in &edits {
            apply_edit(edit, &count, &mut harness);
            let max = (count.get() as f32 * ROW_HEIGHT - VIEWPORT_HEIGHT).max(0.0);
            offset = match *edit {
                Edit::Wheel(down) => (offset + down).clamp(0.0, max),
                Edit::Count(_) => offset.clamp(0.0, max),
            };
        }

        let mut full = full_list_harness(count.get(), offset);
        full.frame();
        full.settle();
        harness.assert_matches(&full);
    }
}
