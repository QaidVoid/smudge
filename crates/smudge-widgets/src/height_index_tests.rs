//! Tests of the height index against a naive model of parallel vectors.

use proptest::prelude::*;

use super::HeightIndex;

/// The model the index is checked against: keys and heights in list order.
///
/// The model sums in f64, where the sums of the heights the strategies draw
/// are exact in any order, so the index must agree with it exactly. An index
/// that summed in f32 would round and fail the comparison.
struct Model {
    keys: Vec<u64>,
    heights: Vec<f32>,
}

impl Model {
    fn offset_of(&self, index: usize) -> f32 {
        self.heights[..index.min(self.heights.len())]
            .iter()
            .map(|height| f64::from(*height))
            .sum::<f64>() as f32
    }

    fn total(&self) -> f32 {
        self.offset_of(self.heights.len())
    }

    /// Builds an index over the model with the O(n) constructor, the way the
    /// keyed list does after an edit that is not an append.
    fn rebuilt(&self) -> HeightIndex<u64> {
        let at: std::collections::HashMap<u64, usize> = self
            .keys
            .iter()
            .enumerate()
            .map(|(at, key)| (*key, at))
            .collect();
        HeightIndex::from_heights(self.keys.iter().copied(), |key| self.heights[at[key]])
    }
}

/// One edit of the random sequence.
#[derive(Debug, Clone)]
enum Edit {
    /// Appends a row.
    Push(f32),
    /// Inserts a row at a position, which rebuilds the index.
    Insert(usize, f32),
    /// Removes a row at a position, which rebuilds the index.
    Remove(usize),
    /// Sets the height of the row at a position.
    Set(usize, f32),
}

/// Heights are small and positive, so the f32 sums stay exact enough for the
/// tolerance, and every row keeps a width a mid-row query can land in.
fn height_strategy() -> impl Strategy<Value = f32> {
    0.5f32..100.0
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => height_strategy().prop_map(Edit::Push),
        1 => (0usize..64usize, height_strategy()).prop_map(|(at, height)| Edit::Insert(at, height)),
        1 => (0usize..64usize).prop_map(Edit::Remove),
        2 => (0usize..64usize, height_strategy()).prop_map(|(at, height)| Edit::Set(at, height)),
    ]
}

/// Applies one edit to both the index and the model, keeping the positions
/// the same for both.
fn apply(index: &mut HeightIndex<u64>, model: &mut Model, edit: &Edit, next_key: &mut u64) {
    match *edit {
        Edit::Push(height) => {
            index.push(*next_key, height);
            model.keys.push(*next_key);
            model.heights.push(height);
            *next_key += 1;
        }
        Edit::Insert(at, height) => {
            let at = at.min(model.keys.len());
            model.keys.insert(at, *next_key);
            model.heights.insert(at, height);
            *next_key += 1;
            *index = model.rebuilt();
        }
        Edit::Remove(at) => {
            if model.keys.is_empty() {
                return;
            }
            let at = at % model.keys.len();
            model.keys.remove(at);
            model.heights.remove(at);
            *index = model.rebuilt();
        }
        Edit::Set(at, height) => {
            if model.keys.is_empty() {
                return;
            }
            let at = at % model.keys.len();
            index.set_height(at, height);
            model.heights[at] = height;
        }
    }
}

/// Asserts that the index agrees with the model: the keys in order, the
/// offset of every row, the row at the middle of every row, and the total.
fn assert_agrees(index: &HeightIndex<u64>, model: &Model) {
    assert_eq!(index.len(), model.keys.len());
    assert_eq!(index.is_empty(), model.keys.is_empty());
    for (at, key) in model.keys.iter().enumerate() {
        assert_eq!(index.key(at), key, "key at {at}");
        assert_eq!(index.position(key), Some(at), "position of key {key}");
    }
    for at in 0..model.keys.len() {
        let expected = model.offset_of(at);
        assert_eq!(index.offset_of(at), expected, "offset of row {at}");
        assert_eq!(index.height(at), model.heights[at], "height of row {at}");
        if model.heights[at] > 0.0 {
            let mid = expected + model.heights[at] * 0.5;
            assert_eq!(
                index.index_at(mid),
                at,
                "row holding the middle of row {at}"
            );
        } else {
            // A zero-height row holds no pixel: its top names the row
            // after the run it starts, which shares the top.
            let named = index.index_at(expected);
            assert!(named >= at, "the top of zero-height row {at} names {named}");
            assert_eq!(
                index.offset_of(named),
                expected,
                "row {named} shares the top"
            );
            assert!(
                model.heights[named] > 0.0 || named == model.keys.len() - 1,
                "the top of zero-height row {at} names zero-height row {named}"
            );
        }
    }
    assert_eq!(index.total(), model.total(), "total");
    assert_eq!(
        index.offset_of(model.keys.len()),
        model.total(),
        "offset past the end"
    );
}

#[test]
fn an_empty_index_names_no_rows() {
    let index = HeightIndex::<u64>::new();
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert_eq!(index.total(), 0.0);
    assert_eq!(index.index_at(0.0), 0);
    assert_eq!(index.index_at(-5.0), 0);
    assert_eq!(index.position(&7), None);
}

#[test]
fn pushes_build_offsets_in_order() {
    let mut index = HeightIndex::new();
    for (key, height) in [30.0, 10.0, 24.0].into_iter().enumerate() {
        index.push(key as u64, height);
    }
    assert_eq!(index.offset_of(0), 0.0);
    assert_eq!(index.offset_of(1), 30.0);
    assert_eq!(index.offset_of(2), 40.0);
    assert_eq!(index.offset_of(3), 64.0);
    assert_eq!(index.total(), 64.0);
    assert_eq!(index.index_at(0.0), 0);
    assert_eq!(index.index_at(29.9), 0);
    assert_eq!(index.index_at(30.0), 1);
    assert_eq!(index.index_at(39.9), 1);
    assert_eq!(index.index_at(40.0), 2);
    assert_eq!(index.index_at(1_000.0), 2);
}

#[test]
fn set_height_moves_the_rows_after_it() {
    let mut index = HeightIndex::from_heights(0..4u64, |key| 10.0 + *key as f32);
    index.set_height(1, 40.0);
    assert_eq!(index.offset_of(1), 10.0);
    assert_eq!(index.offset_of(2), 50.0);
    assert_eq!(index.offset_of(3), 62.0);
    assert_eq!(index.total(), 75.0);
    assert_eq!(index.index_at(45.0), 1);
    assert_eq!(index.index_at(50.0), 2);
}

#[test]
fn offsets_past_the_end_name_the_total_and_the_last_row() {
    let index = HeightIndex::from_heights(0..3u64, |_| 10.0);
    assert_eq!(index.offset_of(7), 30.0);
    assert_eq!(index.index_at(29.0), 2);
    assert_eq!(index.index_at(-1.0), 0);
}

#[test]
fn a_rebuilt_index_matches_one_built_in_order() {
    let mut pushed = HeightIndex::new();
    for key in 0..8u64 {
        pushed.push(key, (key as f32 + 1.0) * 5.0);
    }
    let built = HeightIndex::from_heights(0..8u64, |key| (*key as f32 + 1.0) * 5.0);
    for at in 0..built.len() {
        assert_eq!(pushed.key(at), built.key(at));
        assert_eq!(pushed.offset_of(at), built.offset_of(at));
    }
    assert_eq!(pushed.total(), built.total());
}

/// Heights like the main strategy's, with one in four drawn as zero, the
/// height of a row that shows nothing.
fn height_or_zero_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![1 => Just(0.0f32), 3 => height_strategy()]
}

fn edit_or_zero_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => height_or_zero_strategy().prop_map(Edit::Push),
        1 => (0usize..64usize, height_or_zero_strategy())
            .prop_map(|(at, height)| Edit::Insert(at, height)),
        1 => (0usize..64usize).prop_map(Edit::Remove),
        2 => (0usize..64usize, height_or_zero_strategy())
            .prop_map(|(at, height)| Edit::Set(at, height)),
    ]
}

#[test]
fn a_zero_height_run_shares_its_top_with_the_row_after_it() {
    let index = HeightIndex::from_heights(0..5u64, |key| match *key {
        1..=3 => 0.0,
        _ => 10.0,
    });
    assert_eq!(index.offset_of(1), 10.0);
    assert_eq!(index.offset_of(4), 10.0);
    assert_eq!(
        index.index_at(10.0),
        4,
        "the run's top names the row after it"
    );
    assert_eq!(index.index_at(9.9), 0);
    assert_eq!(index.total(), 20.0);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "duplicate key")]
fn a_duplicate_pushed_key_trips_a_debug_assertion() {
    let mut index = HeightIndex::new();
    index.push(1u64, 10.0);
    index.push(1u64, 10.0);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "duplicate key")]
fn a_duplicate_built_key_trips_a_debug_assertion() {
    HeightIndex::from_heights([1u64, 2, 1], |_| 10.0);
}

proptest! {
    #[test]
    fn edits_with_zero_heights_match_a_naive_model(
        edits in prop::collection::vec(edit_or_zero_strategy(), 1..=64),
    ) {
        let mut index = HeightIndex::new();
        let mut model = Model { keys: Vec::new(), heights: Vec::new() };
        let mut next_key = 0u64;
        for edit in &edits {
            apply(&mut index, &mut model, edit, &mut next_key);
            assert_agrees(&index, &model);
        }
    }

    #[test]
    fn edits_match_a_naive_model(edits in prop::collection::vec(edit_strategy(), 1..=64)) {
        let mut index = HeightIndex::new();
        let mut model = Model { keys: Vec::new(), heights: Vec::new() };
        let mut next_key = 0u64;
        for edit in &edits {
            apply(&mut index, &mut model, edit, &mut next_key);
            assert_agrees(&index, &model);
        }
    }
}

#[test]
fn many_height_changes_on_a_long_list_do_not_drift() {
    let rows = 100_000u64;
    let mut index = HeightIndex::from_heights(0..rows, |_| 110.0);
    let mut first = 110.0f32;
    for step in 0..20_000u32 {
        first = 100.0 + (step % 7) as f32 * 0.37;
        index.set_height(0, first);
    }
    let last = (rows - 1) as usize;
    let expected_offset = f64::from(first) + 110.0 * (rows - 2) as f64;
    let offset = f64::from(index.offset_of(last));
    assert!(
        (offset - expected_offset).abs() <= 2.0,
        "the last row's offset drifted to {offset} from {expected_offset}"
    );
    let expected_total = expected_offset + 110.0;
    let total = f64::from(index.total());
    assert!(
        (total - expected_total).abs() <= 2.0,
        "the total drifted to {total} from {expected_total}"
    );
}
