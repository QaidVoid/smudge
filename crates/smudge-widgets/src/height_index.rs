//! The height index of the keyed virtual list: prefix sums over row heights.
//!
//! The index answers the two questions a virtual list asks every frame, how
//! far from the top a row sits and which row a pixel belongs to, in
//! logarithmic time, and it applies one row's new height in logarithmic time.
//! A Fenwick tree over the heights does all three. Rows are identified by a
//! key the caller owns, so a row keeps its height when the item list reorders,
//! and the index maps each key to its position.

use std::collections::HashMap;
use std::hash::Hash;

/// Prefix sums over the heights of keyed rows, backed by a Fenwick tree.
///
/// The tree answers `offset_of` and `index_at` and applies `set_height` in
/// O(log n), and `push` in O(log n), which is the hot path of a list that
/// grows at the end. Any other edit builds a new index with `from_heights`
/// in O(n), which is expected to stay rare next to appends.
///
/// The heights are stored as given, and the tree sums them in f64, so a
/// long list keeps whole-pixel offsets where f32 sums would round them, and
/// repeated height changes do not drift the offsets after them.
///
/// Heights must not be negative, or `index_at`'s descent, which assumes the
/// prefix sums grow, lands on the wrong row. A NaN height poisons every sum
/// after it. The keyed list never stores either: it reports laid-out
/// rectangles, which are never negative, and guards its estimate.
///
/// Keys must be unique. A duplicate key trips a debug assertion in a debug
/// build. A release build keeps the last position for it, like the
/// keyed-children helper keeps the last row for a duplicate.
pub(crate) struct HeightIndex<K: Eq + Hash + Clone> {
    /// The keys in list order.
    keys: Vec<K>,
    /// The heights, parallel to `keys`.
    heights: Vec<f32>,
    /// The position of each key.
    positions: HashMap<K, usize>,
    /// The Fenwick tree over `heights`, 1-based, so `tree[0]` is unused and
    /// `tree[i]` sums `heights[i - (i & i.wrapping_neg())..i]`.
    tree: Vec<f64>,
    /// The sum of all heights, kept equal to the prefix sum of every height
    /// by each mutation.
    total: f64,
}

impl<K: Eq + Hash + Clone> HeightIndex<K> {
    /// Creates an empty index.
    pub(crate) fn new() -> Self {
        HeightIndex {
            keys: Vec::new(),
            heights: Vec::new(),
            positions: HashMap::new(),
            tree: vec![0.0],
            total: 0.0,
        }
    }

    /// Creates an index over `keys`, asking `height` for each row's height.
    ///
    /// This is the O(n) rebuild the keyed list runs when the item list
    /// changes, carrying the measured heights of the keys that survive.
    ///
    /// # Panics
    ///
    /// Panics in a debug build when `keys` holds a key twice.
    pub(crate) fn from_heights(
        keys: impl IntoIterator<Item = K>,
        height: impl Fn(&K) -> f32,
    ) -> Self {
        let keys: Vec<K> = keys.into_iter().collect();
        let heights: Vec<f32> = keys.iter().map(height).collect();
        let mut index = HeightIndex {
            positions: HashMap::new(),
            keys,
            heights,
            tree: Vec::new(),
            total: 0.0,
        };
        index.rebuild();
        index
    }

    /// Appends `key` with `height` in O(log n).
    ///
    /// # Panics
    ///
    /// Panics in a debug build when the index already holds `key`.
    pub(crate) fn push(&mut self, key: K, height: f32) {
        let duplicate = self.positions.insert(key.clone(), self.keys.len());
        debug_assert!(
            duplicate.is_none(),
            "the height index found a duplicate key, but its keys must be unique"
        );
        self.keys.push(key);
        self.heights.push(height);
        let count = self.keys.len();
        // The new tree entry covers the rows before the new one inside its
        // range, so it starts at their sum plus the new height, and only the
        // entries above it take the delta.
        let covered = count & count.wrapping_neg();
        let entry = self.prefix(count - 1) - self.prefix(count - covered) + f64::from(height);
        self.tree.push(entry);
        let mut at = count + covered;
        while at < self.tree.len() {
            self.tree[at] += f64::from(height);
            at += at & at.wrapping_neg();
        }
        self.total = self.prefix(count);
    }

    /// Sets the height of the row at `index` in O(log n).
    ///
    /// # Panics
    ///
    /// Panics when `index` is past the last row.
    pub(crate) fn set_height(&mut self, index: usize, height: f32) {
        let delta = f64::from(height) - f64::from(self.heights[index]);
        self.heights[index] = height;
        let count = self.keys.len();
        self.add(index + 1, delta);
        self.total = self.prefix(count);
    }

    /// Returns how far the top of the row at `index` sits from the top of
    /// the list.
    ///
    /// An index past the last row returns the total height.
    pub(crate) fn offset_of(&self, index: usize) -> f32 {
        self.prefix(index.min(self.keys.len())) as f32
    }

    /// Returns the row that holds the pixel at `offset`.
    ///
    /// A negative offset names the first row, and an offset at or past the
    /// total height names the last row. A row of positive height holds its
    /// own top, so `index_at(offset_of(i))` is `i` again for one. A
    /// zero-height row shares its top with the rows after it, so the search
    /// names the last row of such a run.
    pub(crate) fn index_at(&self, offset: f32) -> usize {
        if self.keys.is_empty() {
            return 0;
        }
        // The largest count whose prefix sum is at most `offset`, found by
        // descending the tree one power of two at a time. Each sum is
        // compared as the f32 that `offset_of` returns for it, so a query at
        // a row's reported top names that row.
        let mut pos = 0usize;
        let mut sum = 0.0f64;
        let mut bit = self.keys.len().next_power_of_two();
        while bit > 0 {
            let next = pos + bit;
            if next <= self.keys.len() {
                let reached = sum + self.tree[next];
                if reached as f32 <= offset {
                    pos = next;
                    sum = reached;
                }
            }
            bit >>= 1;
        }
        pos.min(self.keys.len() - 1)
    }

    /// Returns the height of the row at `index`.
    ///
    /// # Panics
    ///
    /// Panics when `index` is past the last row.
    pub(crate) fn height(&self, index: usize) -> f32 {
        self.heights[index]
    }

    /// Returns the sum of all heights.
    pub(crate) fn total(&self) -> f32 {
        self.total as f32
    }

    /// Returns how many rows the index holds.
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns whether the index holds no rows.
    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Returns the keys in list order.
    pub(crate) fn keys(&self) -> &[K] {
        &self.keys
    }

    /// Returns the key of the row at `index`.
    ///
    /// # Panics
    ///
    /// Panics when `index` is past the last row.
    pub(crate) fn key(&self, index: usize) -> &K {
        &self.keys[index]
    }

    /// Returns the position of `key`, or `None` when the index does not hold
    /// it.
    pub(crate) fn position(&self, key: &K) -> Option<usize> {
        self.positions.get(key).copied()
    }

    /// Adds `delta` to the height of the 1-based row `at` and to every tree
    /// entry that covers it.
    fn add(&mut self, mut at: usize, delta: f64) {
        while at < self.tree.len() {
            self.tree[at] += delta;
            at += at & at.wrapping_neg();
        }
    }

    /// Returns the sum of the heights of the first `count` rows.
    fn prefix(&self, mut count: usize) -> f64 {
        let mut sum = 0.0;
        while count > 0 {
            sum += self.tree[count];
            count -= count & count.wrapping_neg();
        }
        sum
    }

    /// Rebuilds the tree, the key positions and the total from the keys and
    /// heights, in O(n).
    fn rebuild(&mut self) {
        self.positions = self
            .keys
            .iter()
            .enumerate()
            .map(|(at, key)| (key.clone(), at))
            .collect();
        debug_assert!(
            self.positions.len() == self.keys.len(),
            "the height index found a duplicate key, but its keys must be unique"
        );
        let count = self.keys.len();
        self.tree = vec![0.0; count + 1];
        for at in 1..=count {
            self.tree[at] += f64::from(self.heights[at - 1]);
            let parent = at + (at & at.wrapping_neg());
            if parent <= count {
                self.tree[parent] += self.tree[at];
            }
        }
        self.total = self.prefix(count);
    }
}

impl<K: Eq + Hash + Clone> Default for HeightIndex<K> {
    fn default() -> Self {
        HeightIndex::new()
    }
}

#[cfg(test)]
#[path = "height_index_tests.rs"]
mod tests;
