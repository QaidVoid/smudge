//! The keyed virtual list: a scroll view over keyed items of different
//! heights, with an anchor that keeps the view steady under edits.
//!
//! The list mounts only the rows near the viewport, like the fixed-height
//! [`virtual_list`](crate::virtual_list) does, and it measures each mounted
//! row's real height, which the fixed list never needs to. The heights live
//! in a [`HeightIndex`] that starts from the estimate and replaces it with
//! the measurement of every row that has been shown. Rows are keyed by a
//! stable id, so an item keeps its measured height when the item list
//! reorders or shrinks.
//!
//! The anchor is the key of the first visible row plus the offset into it.
//! After an edit to the items or a measurement changes the heights above the
//! anchor, the list recomputes its scroll offset from the anchor, so the
//! content in view does not jump.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use smudge_core::element::{Element, IntoElement, for_each};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{Memo, RenderEffect, RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::{PaintCx, Widget, panel};

use crate::height_index::HeightIndex;
use crate::scroll::{ScrollExt, ScrollView, scroll_with_visible_height};

/// The rows mounted above and below the viewport, so a small scroll shows
/// content in the frame before the mount follows it.
const OVERSCAN: usize = 4;

/// The fewest rows the window cap allows, before the overscan.
///
/// Rows that measure nothing would otherwise pull every row between two
/// visible ones into the window. The cap is this or one row per logical
/// pixel of the viewport, whichever is more, so rows at least a pixel tall
/// always fill the viewport.
const MIN_WINDOW_CAP: usize = 256;

/// The items the closure returned, compared by identity, so the memo that
/// holds them wakes the list only when the closure hands back a new `Rc`.
struct Snapshot<T>(Rc<[T]>);

impl<T> PartialEq for Snapshot<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// The state one keyed list keeps across frames.
struct KeyedList<T: 'static, K: Eq + Hash + Clone> {
    /// The items of the last run of the items closure, which the row
    /// builders read when a row mounts.
    items: RefCell<Rc<[T]>>,
    /// The heights of the keys in item order, estimates where no measurement
    /// has landed yet.
    index: HeightIndex<K>,
    /// The measured height of each key that has been shown, kept across item
    /// edits so a re-sort keeps them.
    measured: HashMap<K, f32>,
    /// The first visible key and the offset into it, which the offset is
    /// recomputed from after an edit or a measurement.
    anchor: Option<(K, f32)>,
}

/// Creates a list of keyed rows of different heights inside a scroll view,
/// with only the rows near the viewport mounted.
///
/// The returned element is the scroll view; give it a height, either directly
/// or through a parent that sizes it. It scrolls with the wheel like any
/// scroll view, and it shows a thumb for the whole list, because its content
/// is a panel as tall as the height index says the rows are.
///
/// `items` runs inside a memo of its own, so an edit to the signals it reads
/// remeasures the list and a scroll never runs it. An edit that only appends
/// items pushes their keys onto the index in O(log n) each. Any other edit
/// rebuilds the index in O(n): it takes the items' keys in the new order,
/// keeps each surviving key's measured height and estimates the rest.
/// Either way the list recomputes the scroll offset from the anchor, which
/// is the key of the first visible row plus the offset into it. A row that grows above the anchor therefore pushes the view down
/// instead of moving the content on screen, and the anchored row itself grows
/// downward. An anchor key that an edit removed leaves the offset where it
/// was, clamped to the new end.
///
/// The list tells an edit from a no-op by comparing the `Rc` the closure
/// returns: keep the items in a signal of `Rc<[T]>` and hand back a clone of
/// it, so an unrelated wake of the closure skips the walk over the items.
///
/// The item keys must be unique, like the keys of [`for_each`] are: a
/// duplicate key trips a debug assertion in a debug build, and a release
/// build mounts one row for both items while the index counts them twice.
///
/// The rows are built by `row`, which receives the item. A row is built once,
/// the first time its key enters the window, and it stays mounted, with its
/// reactive state, until the key leaves the window. Content that changes
/// while a row stays mounted must come from the signals that row reads: the
/// list does not rebuild a mounted row when its item's data changes. The
/// mounted rows flow edge to edge, so an edit that shifts them moves them
/// without rebuilding them.
///
/// The list learns a row's height when the row is mounted: the laid-out
/// height lands in the index one frame after the row is laid out, following
/// the one-frame rule. The screen never waits for it: the anchored row is
/// pinned where the index puts it and the mounted rows stack from it with
/// their laid-out heights, so a row that grows pushes its neighbours away
/// in the frame it grows instead of covering them. An item that never
/// mounted keeps its estimate, so `estimate` should be close to the typical
/// row height.
///
/// `estimate` must be positive and finite. Anything else mounts no rows, and
/// a list built that way shows an empty view. However small the rows
/// measure, the window mounts at most 256 rows, or one row per logical pixel
/// of the viewport when that is more, plus the overscan, so rows that
/// measure nothing cannot pull the whole list in.
///
/// Build the list where an owner is current, which the app's build closure
/// is: the effect that reconciles the list is owned by that owner and dies
/// with it.
///
/// The list owns its scroll offset; do not call
/// [`ScrollExt::offset`](crate::ScrollExt::offset) on the returned view,
/// because the anchor would fight the signal the call installs.
///
/// Pick this list over [`virtual_list`](crate::virtual_list) when the rows
/// differ in height or the item list edits in the middle: insertions,
/// removals and re-sorts. Pick the fixed list when every row has one height
/// and the list only grows at the end, because it measures nothing and its
/// rows never move but by the scroll.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_text::label;
/// use smudge_widgets::virtual_list_keyed;
///
/// let items: RwSignal<Rc<[u32]>> = RwSignal::new(Rc::from([12, 40, 8, 24]));
/// let list = virtual_list_keyed(
///     move || items.get(),
///     |item| *item,
///     24.0,
///     |item| label(format!("Item {item}")),
/// )
/// .height(px(480.0));
/// ```
pub fn virtual_list_keyed<T: 'static, K: Eq + Hash + Clone + 'static, E: IntoElement>(
    items: impl Fn() -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    estimate: f32,
    row: impl Fn(&T) -> E + 'static,
) -> Element<ScrollView> {
    let estimate = if estimate.is_finite() && estimate > 0.0 {
        estimate
    } else {
        0.0
    };
    let current_items = Memo::new(move || Snapshot(items()));
    let key = Rc::new(key);
    let row = Rc::new(row);

    let state: Rc<RefCell<KeyedList<T, K>>> = Rc::new(RefCell::new(KeyedList {
        items: RefCell::new(Rc::from([])),
        index: HeightIndex::new(),
        measured: HashMap::new(),
        anchor: None,
    }));
    let offset = RwSignal::new(0.0);
    let visible: RwSignal<Option<f32>> = RwSignal::new(None);
    let total = RwSignal::new(0.0);
    let window: RwSignal<Vec<K>> = RwSignal::new(Vec::new());
    let pin: RwSignal<Option<(usize, f32)>> = RwSignal::new(None);
    let reported = RwSignal::new(0u64);
    let pending: Rc<RefCell<Vec<(K, f32)>>> = Rc::new(RefCell::new(Vec::new()));

    // One effect reconciles the list with its inputs: it takes the items,
    // applies the measurements the rows reported while painting, keeps the
    // anchor steady, and re-aims the window. A scroll reaches it through the
    // offset; an edit and a measurement reach it through the signals they
    // write.
    {
        let state = Rc::clone(&state);
        let current_items = current_items.clone();
        let key = Rc::clone(&key);
        let offset = offset.clone();
        let visible = visible.clone();
        let total = total.clone();
        let window = window.clone();
        let pin = pin.clone();
        let reported = reported.clone();
        let pending = Rc::clone(&pending);
        RenderEffect::new(move || {
            let snapshot = current_items.with(|snapshot| Rc::clone(&snapshot.0));
            // The counter is the tracked read that a report at paint wakes.
            // The batch moves out of the cell instead of the signal, because
            // writing the signal back from here would wake this effect
            // against itself.
            let batch = if reported.get() > 0 {
                std::mem::take(&mut *pending.borrow_mut())
            } else {
                Vec::new()
            };
            let viewport = visible.get();
            let mut wanted: f32 = offset.get();

            let mut geometry_changed = false;
            {
                let mut state = state.borrow_mut();
                // The scroll of this frame joins the anchor before anything
                // re-aims it, so a measurement that lands in the same frame
                // compensates around where the view is now, not where it
                // was last frame.
                if !state.index.is_empty() {
                    let at = state.index.index_at(wanted);
                    let within = wanted - state.index.offset_of(at);
                    state.anchor = Some((state.index.key(at).clone(), within));
                }
                let same_items = Rc::ptr_eq(&*state.items.borrow(), &snapshot);
                *state.items.borrow_mut() = snapshot;
                if estimate == 0.0 {
                    state.index = HeightIndex::new();
                    state.anchor = None;
                } else {
                    if !same_items {
                        let list = &mut *state;
                        let items = Rc::clone(&*list.items.borrow());
                        let old_len = list.index.len();
                        let appended = items.len() >= old_len
                            && items
                                .iter()
                                .zip(list.index.keys())
                                .all(|(item, old)| key(item) == *old);
                        if appended {
                            // Nothing above the old end moved, so the anchor
                            // needs no re-aiming.
                            for item in &items[old_len..] {
                                let item_key = key(item);
                                let height =
                                    list.measured.get(&item_key).copied().unwrap_or(estimate);
                                list.index.push(item_key, height);
                            }
                        } else {
                            let keys: Vec<K> = items.iter().map(|item| key(item)).collect();
                            let live: HashSet<&K> = keys.iter().collect();
                            list.measured.retain(|item_key, _| live.contains(item_key));
                            let measured = &list.measured;
                            list.index = HeightIndex::from_heights(keys, |missing| {
                                measured.get(missing).copied().unwrap_or(estimate)
                            });
                            geometry_changed = true;
                        }
                    }
                    for (item_key, height) in batch {
                        if state.measured.get(&item_key) != Some(&height) {
                            state.measured.insert(item_key.clone(), height);
                            if let Some(at) = state.index.position(&item_key) {
                                state.index.set_height(at, height);
                                geometry_changed = true;
                            }
                        }
                    }
                    // The anchor re-aims the offset after an edit or a
                    // measurement. A scroll needs no re-aiming: the absorb
                    // above already moved the anchor to where the view is.
                    if geometry_changed
                        && let Some((anchor_key, within)) = state.anchor.take()
                        && let Some(at) = state.index.position(&anchor_key)
                    {
                        wanted = state.index.offset_of(at) + within;
                    }
                }
            }

            if let Some(viewport) = viewport {
                let max = (state.borrow().index.total() - viewport).max(0.0);
                wanted = wanted.clamp(0.0, max);
            }
            if wanted != offset.read(|value| *value) {
                offset.set(wanted);
            }

            let anchor_at = {
                let mut state = state.borrow_mut();
                let anchor_at = if state.index.is_empty() {
                    state.anchor = None;
                    None
                } else {
                    let at = state.index.index_at(wanted);
                    let top = state.index.offset_of(at);
                    state.anchor = Some((state.index.key(at).clone(), wanted - top));
                    Some((at, top))
                };
                let end = state.index.total();
                if total.read(|value| *value) != end {
                    total.set(end);
                }
                anchor_at
            };

            let (start, next) = window_keys(&state, viewport, estimate, wanted);
            // The anchored row sits where the index puts it, and the rows
            // around it stack from it with their laid-out heights, so a row
            // that grows before its measurement reaches the index pushes its
            // neighbours away instead of covering them.
            let next_pin = anchor_at
                .filter(|_| !next.is_empty())
                .map(|(at, top)| (at - start, top));
            if window.read(|value| *value != next) {
                window.set(next);
            }
            if pin.read(|value| *value != next_pin) {
                pin.set(next_pin);
            }
        });
    }

    // The rows are keyed by the item key, so a row survives an edit that
    // keeps it in the window. They flow edge to edge in window order, and
    // the pin places the anchored one, so an edit or a scroll moves the
    // mounted rows without rebuilding them.
    let rows = {
        let state = Rc::clone(&state);
        let window = window.clone();
        let row = Rc::clone(&row);
        for_each(
            move || window.get(),
            |item_key| item_key.clone(),
            move |item_key| {
                let content = {
                    let state = state.borrow();
                    let at = state.index.position(&item_key);
                    let snapshot = state.items.borrow();
                    match at.and_then(|at| snapshot.get(at)) {
                        Some(item) => row(item).into_any(),
                        None => panel().into_any(),
                    }
                };
                let report_state = Rc::clone(&state);
                let report_key = item_key.clone();
                let report_signal = reported.clone();
                let report_cell = Rc::clone(&pending);
                let report = move |height: f32| {
                    let height = height.max(0.0);
                    if report_state.borrow().measured.get(&report_key) != Some(&height) {
                        report_cell.borrow_mut().push((report_key.clone(), height));
                        report_signal.update(|count| *count += 1);
                    }
                };
                Element::new(MeasureRow {
                    report: Rc::new(report),
                })
                .shrink(0.0)
                .child(content)
            },
        )
        .column()
        .shrink(0.0)
        .pin_child(move || pin.get())
    };

    let content = panel()
        .column()
        .height({
            let total = total.clone();
            move || px(total.get())
        })
        .child(rows);

    // The view clamps the offset with the total the content panel takes in
    // this frame's layout. The painted content height lags a frame, and a
    // re-aim that grows the list at its end would be clamped back by it.
    scroll_with_visible_height(content, visible, Some(total)).offset(offset)
}

/// Returns the position of the first row the window mounts, and the keys
/// of those rows: the rows the viewport shows, plus the overscan on both
/// sides.
fn window_keys<T: 'static, K: Eq + Hash + Clone>(
    state: &RefCell<KeyedList<T, K>>,
    viewport: Option<f32>,
    estimate: f32,
    offset: f32,
) -> (usize, Vec<K>) {
    let state = state.borrow();
    if viewport.is_none() || estimate == 0.0 || state.index.is_empty() {
        return (0, Vec::new());
    }
    let viewport = viewport.unwrap_or(0.0);
    let first = state.index.index_at(offset);
    let last = state.index.index_at(offset + viewport);
    let start = first.saturating_sub(OVERSCAN);
    // Rows a pixel tall or more show at most one per pixel plus a partial
    // one, so a full viewport is never cut.
    let cap = MIN_WINDOW_CAP.max((viewport.ceil() as usize).saturating_add(1)) + 2 * OVERSCAN;
    let end = (last + OVERSCAN)
        .min(state.index.len() - 1)
        .min(start.saturating_add(cap - 1));
    (start, state.index.keys()[start..=end].to_vec())
}

/// A row wrapper that reports its laid-out height when it paints, the way the
/// scroll view reports its viewport.
struct MeasureRow {
    /// Receives the wrapper's laid-out height.
    report: Rc<dyn Fn(f32)>,
}

impl Widget for MeasureRow {
    fn paint(&self, cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {
        (self.report)(cx.rect.height());
    }
}
