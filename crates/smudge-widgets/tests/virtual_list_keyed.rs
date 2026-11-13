//! Whole-frame behavior of the keyed virtual list, driven through the
//! headless harness.
//!
//! The list measures its viewport when the view paints and every mounted row
//! reports its laid-out height the same way, so each test settles the
//! harness after mounting and after every edit. The row heights are whole
//! numbers between 20 and 200, so every position the assertions compare is
//! exact in f32. The estimate sits at 100, which no row of the small lists
//! has, so the tests that tell a measurement from an estimate can rely on
//! the difference.

use std::cell::Cell;
use std::rc::Rc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use smudge_core::geometry::{Point, Size};
use smudge_core::input::InputEvent;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};

use smudge_widgets::virtual_list_keyed;

/// The height of the list, in logical pixels.
const VIEWPORT_HEIGHT: f32 = 480.0;

/// The window the list sits in. Its height matches the list, so nothing
/// below the list can be mistaken for a row.
const WINDOW: Size = Size {
    width: 640.0,
    height: 480.0,
};

/// Where the tests park the pointer, over the middle of the viewport, so the
/// wheel events reach the list.
const POINTER: Point = Point { x: 320.0, y: 240.0 };

/// The rows the list mounts above and below the viewport.
const OVERSCAN: usize = 4;

/// The most rows a window can mount: 25 rows of 20 pixels touch the viewport
/// at worst, and the overscan adds four on each side.
const MOUNTED_BOUND: usize = 33;

/// The most rows the list mounts at once in a viewport this tall, whatever
/// the heights say: 256 or one per logical pixel of the viewport plus a
/// partial one, whichever is more, with the overscan on both sides.
const MAX_WINDOW_ROWS: usize = {
    let per_pixel = VIEWPORT_HEIGHT as usize + 1;
    let rows = if per_pixel > 256 { per_pixel } else { 256 };
    rows + 2 * OVERSCAN
};

/// The height the list starts from. No row of the small lists has it, so a
/// stale estimate cannot pass for a measurement in them.
const ESTIMATE: f32 = 100.0;

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
            "/tests/proptest-regressions/virtual_list_keyed.txt"
        )))),
        ..ProptestConfig::default()
    }
}

/// The true height of the row of `key`, from its key and a seed.
///
/// The values are whole numbers between 20 and 200, so the sums the
/// assertions make are exact in f32. The seed spreads the heights across the
/// property cases.
fn row_height(key: u64, seed: u64) -> f32 {
    20.0 + (key.wrapping_mul(37).wrapping_add(seed) % 181) as f32
}

/// How far the top of the row of `key` sits from the top of the list.
fn top_of(key: u64, seed: u64) -> f32 {
    (0..key).map(|at| row_height(at, seed)).sum()
}

/// One height signal per row, which the row's own binding reads, so a test
/// can grow a row while it stays mounted.
fn height_signals_for(count: u64, seed: u64) -> Vec<RwSignal<f32>> {
    (0..count)
        .map(|key| RwSignal::new(row_height(key, seed)))
        .collect()
}

/// Builds a keyed list of label rows whose row of `key` is as tall as
/// `heights` says, in a window the size of the viewport.
fn static_harness(items: RwSignal<Vec<u64>>, seed: u64, estimate: f32) -> Harness {
    Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        let items = items.clone();
        panel().column().child(
            virtual_list_keyed(
                move || Rc::from(items.get().as_slice()),
                |item| *item,
                estimate,
                move |item| {
                    panel()
                        .height(px(row_height(*item, seed)))
                        .child(label(format!("Key {item}")))
                },
            )
            .height(px(VIEWPORT_HEIGHT)),
        )
    })
}

/// Builds a keyed list whose row of `key` reads its height from
/// `heights[key]`, so a test can grow a row while it stays mounted.
fn keyed_harness(items: RwSignal<Vec<u64>>, heights: Vec<RwSignal<f32>>, estimate: f32) -> Harness {
    let bound_items = items;
    let bound_heights: Rc<[RwSignal<f32>]> = Rc::from(heights);
    Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        let items = bound_items.clone();
        let heights = Rc::clone(&bound_heights);
        panel().column().child(
            virtual_list_keyed(
                move || Rc::from(items.get().as_slice()),
                |item| *item,
                estimate,
                move |item| {
                    let key = *item as usize;
                    let heights = Rc::clone(&heights);
                    panel()
                        .height(move || px(heights[key].get()))
                        .child(label(format!("Key {key}")))
                },
            )
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

/// Panics when the app still holds work that only a frame can finish.
fn assert_settled(harness: &mut Harness) {
    assert!(
        !harness.app().needs_frame(),
        "the frame left work pending, so the screen would not be settled \
         until another frame ran"
    );
}

/// Runs the frames that a paint-time report asks for, and checks each one
/// like the frame before it.
///
/// A keyed list can walk its offset while a batch of measurements changes
/// the heights behind the viewport, so settling may take a frame per batch.
fn settle(harness: &mut Harness) {
    for _ in 0..8 {
        if !harness.woken() {
            return;
        }
        harness.frame();
        assert_settled(harness);
        harness.assert_matches_full_repaint();
    }
    panic!("the list still woke the app after eight frames, so it never settles");
}

/// The screen top and height of every mounted row, from the layout dump.
///
/// Each row is one `MeasureRow` wrapper, as tall as its content. The rows
/// flow edge to edge from the anchored one, which sits where the index
/// puts it.
fn row_rects(harness: &Harness) -> Vec<(f32, f32)> {
    harness
        .layout_dump()
        .lines()
        .filter_map(|line| {
            let start = line.find("MeasureRow [")?;
            let numbers = &line[start + "MeasureRow [".len()..line.len() - 1];
            let values: Vec<f32> = numbers
                .split(", ")
                .filter_map(|value| value.parse().ok())
                .collect();
            Some((values[1], values[3] - values[1]))
        })
        .collect()
}

/// The mounted rows in screen order.
fn rows_by_position(harness: &Harness) -> Vec<(f32, f32)> {
    let mut rows = row_rects(harness);
    rows.sort_by(|first, second| first.0.total_cmp(&second.0));
    rows
}

/// The rows a full index over `keys` mounts at `offset`, as the screen top
/// and height each row should have, in list order.
///
/// The walk mirrors the index's contract: the first visible row is the last
/// one whose top sits at or before the offset, and the overscan reaches four
/// rows past the visible range on both sides.
fn expected_rows(keys: &[u64], height: impl Fn(u64) -> f32, offset: f32) -> Vec<(f32, f32)> {
    let tops: Vec<f32> = keys
        .iter()
        .scan(0.0f32, |top, key| {
            let here = *top;
            *top += height(*key);
            Some(here)
        })
        .collect();
    let first = tops.iter().rposition(|top| *top <= offset).unwrap_or(0);
    let last = tops
        .iter()
        .rposition(|top| *top <= offset + VIEWPORT_HEIGHT)
        .unwrap_or(0);
    let start = first.saturating_sub(OVERSCAN);
    let end = (last + OVERSCAN).min(keys.len() - 1);
    (start..=end)
        .map(|at| (tops[at] - offset, height(keys[at])))
        .collect()
}

#[test]
fn ten_thousand_rows_mount_a_bounded_window_at_three_offsets() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..10_000).collect::<Vec<u64>>());
    let mut harness = static_harness(items, 0, ESTIMATE);
    harness.frame();
    settle(&mut harness);

    let keys: Vec<u64> = (0..10_000).collect();
    let mounted = row_rects(&harness).len();
    assert!(
        mounted <= MOUNTED_BOUND,
        "the top of the list mounted {mounted} rows"
    );
    assert_eq!(
        rows_by_position(&harness),
        expected_rows(&keys, |key| row_height(key, 0), 0.0),
        "the top of the list mounted the wrong rows"
    );

    // The middle of the list holds no measurement yet, so the rows mount at
    // estimated positions first. The row the offset lands in anchors the
    // view, and keeps its place on screen while the measurements of the
    // rows around it land.
    harness.move_to(POINTER);
    wheel(&mut harness, 5_000.0 * ESTIMATE);
    harness.frame();
    let anchored = anchored_row(&rows_by_position(&harness));
    settle(&mut harness);
    let middle = rows_by_position(&harness);
    assert!(
        middle.len() <= MOUNTED_BOUND,
        "the middle of the list mounted {} rows",
        middle.len()
    );
    let anchored = anchored.expect("a row sits at the top of the viewport");
    assert!(
        middle.contains(&anchored),
        "the anchored row {anchored:?} moved while the rows around it were measured"
    );
    assert_contiguous_over_viewport(&middle);

    // The end of the list: the last mounted row is the last key, and once
    // those rows are measured a scroll to the end puts its bottom on the
    // bottom of the viewport.
    wheel(&mut harness, 100_000_000.0);
    harness.frame();
    settle(&mut harness);
    wheel(&mut harness, 100_000_000.0);
    harness.frame();
    settle(&mut harness);
    let end = rows_by_position(&harness);
    assert!(
        end.len() <= MOUNTED_BOUND,
        "the end of the list mounted {} rows",
        end.len()
    );
    assert_contiguous_over_viewport(&end);
    let last_heights: Vec<f32> = end.iter().rev().map(|row| row.1).collect();
    let expected_heights: Vec<f32> = (0..end.len() as u64)
        .map(|back| row_height(9_999 - back, 0))
        .collect();
    assert_eq!(
        last_heights, expected_heights,
        "the window ends at the last key with every height measured"
    );
    let bottom = end.last().map(|row| row.0 + row.1);
    assert_eq!(
        bottom,
        Some(VIEWPORT_HEIGHT),
        "the last row ends at the bottom of the viewport"
    );
    harness.assert_matches_full_repaint();
}

/// The row the list anchors on: the last one whose top is at or above the
/// top of the viewport.
///
/// Before the measurements land, the rows sit where the estimates put them,
/// so the anchored row can end above the viewport while its estimate still
/// reaches into it.
fn anchored_row(rows: &[(f32, f32)]) -> Option<(f32, f32)> {
    rows.iter().rev().find(|row| row.0 <= 0.0).copied()
}

/// Panics unless the rows sit edge to edge and cover the whole viewport.
fn assert_contiguous_over_viewport(rows: &[(f32, f32)]) {
    for pair in rows.windows(2) {
        assert_eq!(
            pair[0].0 + pair[0].1,
            pair[1].0,
            "a row does not start where the row above it ends: {pair:?}"
        );
    }
    let first = rows.first().expect("the window holds rows");
    let last = rows.last().expect("the window holds rows");
    assert!(first.0 <= 0.0, "the rows start below the top: {first:?}");
    assert!(
        last.0 + last.1 >= VIEWPORT_HEIGHT,
        "the rows end above the bottom: {last:?}"
    );
}

#[test]
fn a_row_growing_above_the_anchor_keeps_every_mounted_row_where_it_was() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..60).collect::<Vec<u64>>());
    let heights = height_signals_for(60, 0);
    let mut harness = keyed_harness(items, heights.clone(), ESTIMATE);
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);

    // Rows 0 to 9 carry their measured heights by now, so scrolling exactly
    // to the top of key 9 anchors it, with key 5 mounted in the overscan
    // above the viewport.
    wheel(&mut harness, top_of(9, 0));
    harness.frame();
    settle(&mut harness);

    let before = rows_by_position(&harness);
    assert_eq!(before.len(), 13, "the window holds keys 5 to 17");
    assert!(
        before.iter().any(|(top, _)| *top == 0.0),
        "key 9 is anchored at the top of the viewport"
    );

    let growth = 40.0;
    heights[5].update(|height| *height += growth);
    harness.frame();
    settle(&mut harness);

    let after = rows_by_position(&harness);

    // The view compensates for the growth, so every row but the grown one
    // keeps its screen position, and the grown one keeps its place in the
    // list while moving up by the growth, still above the viewport.
    let moved: Vec<(f32, f32)> = after
        .iter()
        .filter(|row| !before.contains(row))
        .copied()
        .collect();
    let grown = (
        top_of(5, 0) - top_of(9, 0) - growth,
        row_height(5, 0) + growth,
    );
    assert_eq!(moved, vec![grown], "only the grown row changed on screen");
    assert!(
        grown.0 + grown.1 <= 0.0,
        "the grown row stayed above the viewport"
    );
    let emptied: Vec<(f32, f32)> = before
        .iter()
        .filter(|row| !after.contains(row))
        .copied()
        .collect();
    let old_grown = (top_of(5, 0) - top_of(9, 0), row_height(5, 0));
    assert_eq!(
        emptied,
        vec![old_grown],
        "only the grown row's place changed"
    );
}

#[test]
fn a_row_growing_at_the_anchor_keeps_its_top_and_moves_the_rows_below() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..60).collect::<Vec<u64>>());
    let heights = height_signals_for(60, 0);
    let mut harness = keyed_harness(items, heights.clone(), ESTIMATE);
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);
    wheel(&mut harness, top_of(9, 0));
    harness.frame();
    settle(&mut harness);

    let before = rows_by_position(&harness);
    let grown = heights[9].get() + 40.0;

    heights[9].update(|height| *height += 40.0);
    harness.frame();
    settle(&mut harness);

    let after = rows_by_position(&harness);

    // The anchored row keeps its top and grows downward.
    let anchored = after
        .iter()
        .find(|(top, _)| *top == 0.0)
        .expect("the anchored row stays mounted");
    assert_eq!(anchored.1, grown, "the anchored row grew downward");

    // Every row above the anchor keeps its place, and every row below it
    // moves down by exactly the growth.
    for (top, height) in after.iter().filter(|(top, _)| *top < 0.0) {
        assert!(
            before.contains(&(*top, *height)),
            "a row above the anchor moved to {top}"
        );
    }
    for (top, height) in after.iter().filter(|(top, _)| *top > 0.0) {
        assert!(
            before.contains(&(*top - 40.0, *height)),
            "a row below the anchor moved to {top}, not by the growth alone"
        );
    }
    assert_eq!(
        after.iter().filter(|(top, _)| *top > 0.0).count(),
        before.iter().filter(|(top, _)| *top > 0.0).count(),
        "the rows below the anchor stayed mounted"
    );
}

#[test]
fn a_scroll_landing_in_the_frame_a_measurement_applies_is_not_lost() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..60).collect::<Vec<u64>>());
    let mut harness = static_harness(items, 0, ESTIMATE);
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);

    // The first wheel mounts rows the list has not measured, and their
    // reports land while the frame is over. The second wheel lands in the
    // frame that applies those reports, and must not be undone by the
    // re-aiming the measurements trigger.
    wheel(&mut harness, top_of(9, 0));
    harness.frame();
    let second = 123.0;
    wheel(&mut harness, second);
    harness.frame();
    settle(&mut harness);

    let offset = top_of(9, 0) + second;
    let keys: Vec<u64> = (0..60).collect();
    assert_eq!(
        rows_by_position(&harness),
        expected_rows(&keys, |key| row_height(key, 0), offset),
        "the scroll that landed with a measurement was lost"
    );
}

#[test]
fn a_filtered_item_list_keeps_the_surviving_rows_measured_heights() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..40).collect::<Vec<u64>>());
    let mut harness = keyed_harness(items.clone(), height_signals_for(40, 0), ESTIMATE);
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);

    // Scroll through the whole list, so every row mounts once and its
    // measured height lands in the index.
    for _ in 0..12 {
        wheel(&mut harness, 400.0);
        harness.frame();
        settle(&mut harness);
    }

    // Every row is measured now, so the way back is exact.
    wheel(&mut harness, -100_000.0);
    harness.frame();
    settle(&mut harness);
    let anchor_offset = top_of(20, 0);
    wheel(&mut harness, anchor_offset);
    harness.frame();
    settle(&mut harness);

    let keys: Vec<u64> = (0..40).collect();
    assert_eq!(
        rows_by_position(&harness),
        expected_rows(&keys, |key| row_height(key, 0), anchor_offset),
        "the anchor did not land on key 20 with the measured heights"
    );

    // Keep every other item. The survivors keep their measured heights, so
    // the anchored row stays at the top of the view and the rows around it
    // sit where their measured heights put them.
    items.update(|list| list.retain(|key| key % 2 == 0));
    harness.frame();
    settle(&mut harness);

    let survivors: Vec<u64> = (0..40).step_by(2).collect();
    let filtered_offset = (0..20)
        .step_by(2)
        .map(|key| row_height(key, 0))
        .sum::<f32>();
    assert_eq!(
        rows_by_position(&harness),
        expected_rows(&survivors, |key| row_height(key, 0), filtered_offset),
        "the filtered list did not keep the surviving rows' measured heights"
    );
}

// The mounted-window bound over random scrolls of a list whose heights the
// case seeds.
proptest! {
    #![proptest_config(config())]
    #[test]
    fn wheels_over_seeded_heights_keep_the_mounted_window_bounded(
        seed in 0u64..1_000,
        steps in prop::collection::vec(0.0f32..250_000.0, 1..=6),
    ) {
        TextSystem::bundled_only().install();
        let items = RwSignal::new((0..10_000).collect::<Vec<u64>>());
        let mut harness = static_harness(items, seed, ESTIMATE);
        harness.frame();
        settle(&mut harness);
        harness.move_to(POINTER);
        for step in steps {
            wheel(&mut harness, step);
            harness.frame();
            settle(&mut harness);
            let mounted = row_rects(&harness).len();
            assert!(
                mounted <= MOUNTED_BOUND,
                "the window mounted {mounted} rows after scrolling by {step}"
            );
        }
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn a_row_growing_above_the_viewport_at_the_end_keeps_the_rows_in_view() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..60).collect::<Vec<u64>>());
    let heights = height_signals_for(60, 0);
    let mut harness = keyed_harness(items, heights.clone(), ESTIMATE);
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);
    for _ in 0..2 {
        wheel(&mut harness, 100_000.0);
        harness.frame();
        settle(&mut harness);
    }

    let before = rows_by_position(&harness);
    let bottom = before.last().map(|row| row.0 + row.1);
    assert_eq!(bottom, Some(VIEWPORT_HEIGHT), "the view sits at the end");
    // The rows are keys 59 and down, from the bottom, so the first mounted
    // row that sits wholly above the viewport names its key by its place.
    let above = before
        .iter()
        .rposition(|row| row.0 + row.1 <= 0.0)
        .expect("the overscan mounts a row above the viewport");
    let key = 59 - (before.len() - 1 - above);
    heights[key].update(|height| *height += 40.0);
    harness.frame();
    settle(&mut harness);

    let after = rows_by_position(&harness);
    let visible = |rows: &[(f32, f32)]| -> Vec<(f32, f32)> {
        rows.iter()
            .filter(|row| row.0 + row.1 > 0.0)
            .copied()
            .collect()
    };
    assert_eq!(
        visible(&after),
        visible(&before),
        "the rows in view did not move when a row above them grew"
    );
}

#[test]
fn zero_height_rows_between_visible_ones_mount_a_bounded_window() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..5_000).collect::<Vec<u64>>());
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        panel().column().child(
            virtual_list_keyed(
                move || Rc::from(items.get().as_slice()),
                |item| *item,
                20.0,
                // One row in fifty shows; the rest measure nothing, the way
                // rows a filter hides do.
                |item| panel().height(px(if item % 50 == 0 { 20.0 } else { 0.0 })),
            )
            .height(px(VIEWPORT_HEIGHT)),
        )
    });
    harness.frame();
    let mut mounted = row_rects(&harness).len();
    for _ in 0..400 {
        if !harness.woken() {
            break;
        }
        harness.frame();
        mounted = mounted.max(row_rects(&harness).len());
    }
    assert!(
        !harness.woken(),
        "the list kept asking for frames while it measured"
    );
    assert!(
        mounted <= MAX_WINDOW_ROWS,
        "rows that measure nothing pulled {mounted} rows into the window"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn many_small_rows_fill_a_tall_viewport_without_a_gap() {
    TextSystem::bundled_only().install();
    let tall = 1_200.0;
    let row = 4.0;
    let items = RwSignal::new((0..600).collect::<Vec<u64>>());
    let mut harness = Harness::new(WINDOW.width, tall, 1.0, move || {
        panel().column().child(
            virtual_list_keyed(
                move || Rc::from(items.get().as_slice()),
                |item| *item,
                row,
                move |_| panel().height(px(row)),
            )
            .height(px(tall)),
        )
    });
    harness.frame();
    settle(&mut harness);

    let rows = rows_by_position(&harness);
    for pair in rows.windows(2) {
        assert_eq!(
            pair[0].0 + pair[0].1,
            pair[1].0,
            "a row does not start where the row above it ends: {pair:?}"
        );
    }
    let bottom = rows.last().map_or(0.0, |last| last.0 + last.1);
    assert!(
        bottom >= tall,
        "{} rows of {row} pixels end at {bottom}, leaving the bottom of a \
         {tall}-pixel viewport blank",
        rows.len()
    );
}

#[test]
fn scrolling_does_not_run_the_items_closure() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..1_000).collect::<Vec<u64>>());
    let runs = Rc::new(Cell::new(0u32));
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, {
        let runs = Rc::clone(&runs);
        move || {
            panel().column().child(
                virtual_list_keyed(
                    move || {
                        runs.set(runs.get() + 1);
                        Rc::from(items.get().as_slice())
                    },
                    |item| *item,
                    ESTIMATE,
                    |item| panel().height(px(row_height(*item, 0))),
                )
                .height(px(VIEWPORT_HEIGHT)),
            )
        }
    });
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);
    let before = runs.get();
    for _ in 0..5 {
        wheel(&mut harness, 300.0);
        harness.frame();
        settle(&mut harness);
    }
    assert_eq!(
        runs.get(),
        before,
        "a frame that only scrolls reads no items"
    );
}

thread_local! {
    /// How many times a [`CountedKey`] was cloned on this test's thread.
    static KEY_CLONES: Cell<u64> = const { Cell::new(0) };
}

/// A key that counts its clones, so a test can tell a frame that copies the
/// window's keys from one that keeps them.
#[derive(Debug, PartialEq, Eq, Hash)]
struct CountedKey(u64);

impl Clone for CountedKey {
    fn clone(&self) -> Self {
        KEY_CLONES.with(|clones| clones.set(clones.get() + 1));
        CountedKey(self.0)
    }
}

#[test]
fn an_append_keys_each_item_once_and_keeps_the_index() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new(Rc::<[u64]>::from((0..10_000).collect::<Vec<u64>>()));
    let key_calls = Rc::new(Cell::new(0u64));
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, {
        let items = items.clone();
        let key_calls = Rc::clone(&key_calls);
        move || {
            let key_calls = Rc::clone(&key_calls);
            panel().column().child(
                virtual_list_keyed(
                    move || items.get(),
                    move |item| {
                        key_calls.set(key_calls.get() + 1);
                        CountedKey(*item)
                    },
                    ESTIMATE,
                    |item| panel().height(px(row_height(*item, 0))),
                )
                .height(px(VIEWPORT_HEIGHT)),
            )
        }
    });
    harness.frame();
    settle(&mut harness);

    key_calls.set(0);
    KEY_CLONES.with(|clones| clones.set(0));
    items.update(|list| {
        let mut grown = list.to_vec();
        grown.extend([10_000, 10_001]);
        *list = Rc::from(grown);
    });
    harness.frame();
    settle(&mut harness);
    let calls = key_calls.get();
    assert_eq!(calls, 10_002, "an append keys each item exactly once");
    // A rebuild clones every key into the index's positions; the window
    // and the anchor clone a few dozen at most.
    let clones = KEY_CLONES.with(Cell::get);
    assert!(
        clones < 1_000,
        "an append to 10,000 rows cloned {clones} keys, so it rebuilt the index"
    );
}

#[test]
fn an_edit_in_the_middle_with_an_append_rebuilds_the_index() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..40).collect::<Vec<u64>>());
    let mut harness = keyed_harness(items.clone(), height_signals_for(41, 0), ESTIMATE);
    harness.frame();
    settle(&mut harness);

    // Swapping two items in view and appending one in the same update must
    // show the swapped rows where the new order puts them.
    items.update(|list| {
        list.swap(1, 2);
        list.push(40);
    });
    harness.frame();
    settle(&mut harness);

    let keys: Vec<u64> = items.get();
    assert_eq!(
        rows_by_position(&harness),
        expected_rows(&keys, |key| row_height(key, 0), 0.0),
        "the rows do not follow the edited order"
    );
}

#[test]
fn a_scroll_inside_the_mounted_rows_copies_no_window() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new(Rc::<[u64]>::from((0..1_000).collect::<Vec<u64>>()));
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, {
        let items = items.clone();
        move || {
            panel().column().child(
                virtual_list_keyed(
                    move || items.get(),
                    |item| CountedKey(*item),
                    ESTIMATE,
                    |item| panel().height(px(row_height(*item, 0))),
                )
                .height(px(VIEWPORT_HEIGHT)),
            )
        }
    });
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);

    // Key 0 is 20 pixels tall, so steps of one pixel stay inside it and the
    // window keeps the same rows.
    let window = row_rects(&harness).len() as u64;
    let steps = 5;
    KEY_CLONES.with(|clones| clones.set(0));
    for _ in 0..steps {
        wheel(&mut harness, 1.0);
        harness.frame();
        settle(&mut harness);
    }
    let clones = KEY_CLONES.with(Cell::get);
    assert!(
        clones < steps * window,
        "{steps} scroll frames over {window} mounted rows cloned {clones} keys, so \
         each frame copied the window"
    );
}

#[test]
fn a_leading_run_of_empty_rows_shows_again_when_it_grows() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..60).collect::<Vec<u64>>());
    let heights = height_signals_for(60, 0);
    let empty = 0..10usize;
    for key in empty.clone() {
        heights[key].set(0.0);
    }
    let mut harness = keyed_harness(items, heights.clone(), ESTIMATE);
    harness.frame();
    settle(&mut harness);

    // The ten empty rows measured nothing at the top of the list. When they
    // grow back, the list must still hold them, so their new heights land
    // and the top of the list shows them again.
    for key in empty.clone() {
        heights[key].set(30.0);
    }
    harness.frame();
    settle(&mut harness);

    let keys: Vec<u64> = (0..60).collect();
    let height = |key: u64| {
        if empty.contains(&(key as usize)) {
            30.0
        } else {
            row_height(key, 0)
        }
    };
    assert_eq!(
        rows_by_position(&harness),
        expected_rows(&keys, height, 0.0),
        "the rows that grew back at the top of the list are not shown"
    );
}

/// Panics when two mounted rows overlap on screen.
fn assert_no_overlap(rows: &[(f32, f32)], frame: &str) {
    for pair in rows.windows(2) {
        assert!(
            pair[0].0 + pair[0].1 <= pair[1].0,
            "{frame}: a row overlaps the row below it: {pair:?}"
        );
    }
}

#[test]
fn no_frame_shows_overlapping_rows_while_rows_grow() {
    TextSystem::bundled_only().install();
    let items = RwSignal::new((0..60).collect::<Vec<u64>>());
    let heights = height_signals_for(60, 0);
    let mut harness = keyed_harness(items, heights.clone(), ESTIMATE);
    harness.frame();
    settle(&mut harness);
    harness.move_to(POINTER);
    wheel(&mut harness, top_of(9, 0));
    harness.frame();
    settle(&mut harness);
    let before = rows_by_position(&harness);
    let in_view = |rows: &[(f32, f32)]| -> Vec<(f32, f32)> {
        rows.iter().filter(|row| row.0 >= 0.0).copied().collect()
    };

    // Key 8 ends exactly at the top of the viewport. Growing it must not
    // reach into the view, not even in the frame before its measurement
    // lands in the index.
    heights[8].update(|height| *height += 40.0);
    harness.frame();
    let rows = rows_by_position(&harness);
    assert_no_overlap(&rows, "the frame a row above the anchor grew");
    assert_eq!(
        in_view(&rows),
        in_view(&before),
        "the rows in view moved in the frame a row above them grew"
    );
    settle(&mut harness);

    // The anchored row streams: it grows every frame, and every frame shows
    // the rows below it pushed down by exactly its growth.
    for step in 1..=5 {
        heights[9].update(|height| *height += 12.0);
        harness.frame();
        let rows = rows_by_position(&harness);
        assert_no_overlap(&rows, &format!("streaming frame {step}"));
        let anchored = rows
            .iter()
            .find(|row| row.0 == 0.0)
            .expect("the anchored row keeps its top");
        assert_eq!(
            anchored.1,
            heights[9].get(),
            "frame {step} shows the new height"
        );
    }
    settle(&mut harness);
    harness.assert_matches_full_repaint();
}
