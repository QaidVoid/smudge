//! Tests of the height index against a naive model of parallel vectors.

use proptest::prelude::*;

use super::HeightIndex;

/// The sums of the model and the tree are f32 accumulations in different
/// orders, so comparisons carry an absolute tolerance. The heights the
/// strategy draws and the op counts bound the error far below it.
const EPSILON: f32 = 0.05;

/// The model the index is checked against: keys and heights in list order.
struct Model {
    keys: Vec<u64>,
    heights: Vec<f32>,
}

impl Model {
    fn offset_of(&self, index: usize) -> f32 {
        self.heights[..index.min(self.heights.len())].iter().sum()
    }

    fn total(&self) -> f32 {
        self.heights.iter().sum()
    }
}

/// One edit of the random sequence.
#[derive(Debug, Clone)]
enum Edit {
    /// Appends a row.
    Push(f32),
    /// Inserts a row at a position.
    Insert(usize, f32),
    /// Removes a row at a position.
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
            index.insert(at, *next_key, height);
            model.keys.insert(at, *next_key);
            model.heights.insert(at, height);
            *next_key += 1;
        }
        Edit::Remove(at) => {
            if model.keys.is_empty() {
                return;
            }
            let at = at % model.keys.len();
            index.remove(at);
            model.keys.remove(at);
            model.heights.remove(at);
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
        let actual = index.offset_of(at);
        assert!(
            (actual - expected).abs() < EPSILON,
            "offset of row {at}: {actual} against {expected}"
        );
        let mid = expected + model.heights[at] * 0.5;
        assert_eq!(
            index.index_at(mid),
            at,
            "row holding the middle of row {at}"
        );
    }
    let total = model.total();
    assert!(
        (index.total() - total).abs() < EPSILON,
        "total {} against {}",
        index.total(),
        total
    );
    let last = model.offset_of(model.keys.len());
    assert!(
        (index.offset_of(model.keys.len()) - last).abs() < EPSILON,
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
fn insert_and_remove_rebuild_every_offset() {
    let mut index = HeightIndex::from_heights(0..3u64, |_| 10.0);
    index.insert(1, 100, 50.0);
    assert_eq!(index.key(1), &100);
    assert_eq!(index.offset_of(2), 60.0);
    assert_eq!(index.position(&2), Some(3));
    assert_eq!(index.total(), 80.0);

    index.remove(1);
    assert_eq!(index.key(1), &1);
    assert_eq!(index.offset_of(2), 20.0);
    assert_eq!(index.position(&100), None);
    assert_eq!(index.total(), 30.0);
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
    pushed.insert(2, 100, 30.0);
    pushed.remove(0);
    let built = HeightIndex::from_heights([1, 100, 2, 3, 4, 5, 6, 7], |key| match *key {
        100 => 30.0,
        key => (key as f32 + 1.0) * 5.0,
    });
    for at in 0..built.len() {
        assert_eq!(pushed.key(at), built.key(at));
        assert_eq!(pushed.offset_of(at), built.offset_of(at));
    }
    assert_eq!(pushed.total(), built.total());
}

proptest! {
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
