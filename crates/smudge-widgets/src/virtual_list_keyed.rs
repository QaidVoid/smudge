//! The keyed virtual list: a scroll view over keyed items of different
//! heights, with an anchor that keeps the view steady under edits. It also
//! backs the keyed virtual column, the same view without list semantics.
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

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use smudge_core::element::{Element, IntoElement, for_each};
use smudge_core::input::{Handled, Key, KeyEvent};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{Memo, Owner, RenderEffect, RwSignal, prelude::*};
use smudge_core::semantics::{Announcer, Politeness, Role};
use smudge_core::style::{Edges, Length, px};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};

use crate::height_index::HeightIndex;
use crate::scroll::{ScrollExt, ScrollView, scroll_with_visible_height};

/// The rows mounted above and below the viewport, so a small scroll shows
/// content in the frame before the mount follows it.
const OVERSCAN: usize = 4;

/// The thickness of the drop indicator line, in logical pixels.
const DROP_INDICATOR: f32 = 2.0;

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
    /// The height the index gave the rows no measurement reached, which a
    /// remount after a hot patch can find changed in the state it carried.
    estimate: f32,
    /// The smallest positive height a row has measured. It never grows, so
    /// it stays a bound after the row that set it goes.
    smallest: f32,
}

/// Creates a list of keyed rows of different heights inside a scroll view,
/// with only the rows near the viewport mounted.
///
/// The returned element is the scroll view; give it a height, either directly
/// or through a parent that sizes it. It scrolls with the wheel like any
/// scroll view, and it shows a thumb for the whole list, whose length
/// follows the total height the index gives the rows.
///
/// The scroll view holds a list for the accessibility tree, which reports
/// how many items there are as its size, and each mounted row is a list
/// item with its item's place, which follows edits before it. Only the
/// mounted rows have nodes. For content that is not a list, such as the
/// blocks of a document, use [`virtual_column_keyed`], which reports no list.
///
/// `items` runs inside a memo of its own, so an edit to the signals it reads
/// remeasures the list and a scroll never runs it. Every update calls `key`
/// once for each item, so prefer a cheap key such as an integer id: a key
/// that builds a `String` allocates on every call. An update whose old keys
/// all sit where they were only appends, and pushes each new key onto the
/// index in O(log n) without rebuilding it. Any other edit rebuilds the
/// index in O(n): it takes the items' keys in the new order, keeps each
/// surviving key's measured height and estimates the rest. Either way the list recomputes the scroll offset from the
/// anchor, which is the key of the first visible row plus the offset into
/// it. A row that grows above the anchor therefore pushes the view down
/// instead of moving the content on screen, and the anchored row itself grows
/// downward. An anchor key that an edit removed leaves the offset where it
/// was, clamped to the new end.
///
/// The list tells an edit from a no-op by comparing the `Rc` the closure
/// returns: keep the items in a signal of `Rc<[T]>` and hand back a clone of
/// it, so an unrelated wake of the closure skips the walk over the items.
///
/// The item keys must be unique, like the keys of [`for_each`] are: a
/// duplicate key trips a debug assertion in a debug build when the list
/// takes it in. A release build keeps one position for the key, the last
/// one, while the index counts both items: the anchor and the measurement
/// of that key follow the last item, so the view can jump to it, and rows
/// near the two items can sit out of place.
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
/// The list owns its scroll offset, so do not call
/// [`ScrollExt::offset`](crate::ScrollExt::offset) on the returned view,
/// because the anchor would fight the signal the call installs.
/// [`ScrollExt::follow_end`](crate::ScrollExt::follow_end) works as it does
/// on a plain scroll view: while the view follows and the last item is
/// mounted, the rows flow from where the index puts the first mounted one,
/// and the view pins to the bottom of the last row as laid out, so a row
/// that mounts at the end taller than its estimate, or grows there, never
/// shows its end out of view. Rows that mount there shorter than any row
/// the list has measured can leave the top of the viewport empty for the
/// frame they mount in, until their measurements land.
///
/// The list scrolls up and down only, whatever
/// [`ScrollExt::axis`](crate::ScrollExt::axis) it is given.
///
/// Pick this list over [`virtual_list`](crate::virtual_list) when the rows
/// differ in height or the item list edits in the middle: insertions,
/// removals and re-sorts. Pick the fixed list when every row has one height
/// and the list only grows at the end, because it measures nothing and its
/// rows never move but by the scroll.
///
/// # Panics
///
/// In a debug build, panics when two items share a key.
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
#[track_caller]
pub fn virtual_list_keyed<T: 'static, K: Eq + Hash + Clone + 'static, E: IntoElement>(
    items: impl Fn() -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    estimate: f32,
    row: impl Fn(&T) -> E + 'static,
) -> Element<ScrollView> {
    keyed(items, key, estimate, row, true, None, None)
}

/// A scroll view that mounts only the rows near its viewport, as
/// [`virtual_list_keyed`] does, for content that is not a list.
///
/// It behaves as [`virtual_list_keyed`] does in every way but one: the
/// accessibility tree sees no list and no list items, only the rows' own
/// content, so a screen reader reads a document built from it as a document
/// rather than announcing how many items it holds.
///
/// # Panics
///
/// In a debug build, panics when two items share a key.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_text::label;
/// use smudge_widgets::virtual_column_keyed;
///
/// let blocks: RwSignal<Rc<[u32]>> = RwSignal::new(Rc::from([1, 2, 3]));
/// let document = virtual_column_keyed(
///     move || blocks.get(),
///     |block| *block,
///     48.0,
///     |block| label(format!("Paragraph {block}")),
/// )
/// .height(px(480.0));
/// ```
#[track_caller]
pub fn virtual_column_keyed<T: 'static, K: Eq + Hash + Clone + 'static, E: IntoElement>(
    items: impl Fn() -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    estimate: f32,
    row: impl Fn(&T) -> E + 'static,
) -> Element<ScrollView> {
    keyed(items, key, estimate, row, false, None, None)
}

/// Builds a keyed virtual column, reported as a list with its items when
/// `list` holds.
#[track_caller]
pub(crate) fn keyed<T: 'static, K: Eq + Hash + Clone + 'static, E: IntoElement>(
    items: impl Fn() -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    estimate: f32,
    row: impl Fn(&T) -> E + 'static,
    list: bool,
    // The offset the view scrolls with and the signal it reports its
    // viewport height to, when the caller hands signals in. A widget that
    // wraps the list, such as the table, drives and reads them; the list
    // makes its own when the caller leaves them out.
    shared: Option<(RwSignal<f32>, RwSignal<Option<f32>>)>,
    // The key a caller asks the view to bring into the viewport, such as
    // the row a keyboard move just moved far away. The view aims the offset
    // at it before the window is computed, so the row mounts and keeps
    // whatever state, the focus included, it had, and the signal clears
    // once the request is spent.
    reveal: Option<RwSignal<Option<K>>>,
) -> Element<ScrollView> {
    let estimate = if estimate.is_finite() && estimate > 0.0 {
        estimate
    } else {
        0.0
    };
    let items = smudge_core::hot::wrap(items);
    let current_items = Memo::new(move || Snapshot(items()));
    let key = Rc::new(smudge_core::hot::wrap1_ref(key));
    let row = Rc::new(smudge_core::hot::wrap1_ref(row));

    // The state sits in a signal so the remount after a hot patch carries
    // it, measured heights and anchor included, and the view comes back on
    // the row it showed instead of re-aiming against estimates. The owner
    // keeps the signal until it cleans up, since a remount carries only the
    // signals that still live when it takes the old build's values.
    let carried = crate::signal(Rc::new(RefCell::new(KeyedList {
        items: RefCell::new(Rc::from([])),
        index: HeightIndex::new(),
        measured: HashMap::new(),
        anchor: None,
        estimate,
        smallest: f32::INFINITY,
    })));
    let state: Rc<RefCell<KeyedList<T, K>>> = carried.read(Rc::clone);
    Owner::on_cleanup(move || drop(carried));
    let (offset, visible) = if let Some((shared_offset, shared_visible)) = shared {
        (shared_offset, shared_visible)
    } else {
        (crate::signal(0.0), crate::signal(None))
    };
    // A remount after a hot patch can hand this the measurement of the old
    // build's layout, which says nothing of the new one. Dropped before the
    // list's first run, it neither clamps a carried offset nor mounts rows
    // a later reset of the measurement would take down again.
    if visible.read(Option::is_some) {
        visible.set(None);
    }
    let total = crate::signal(0.0);
    let window: RwSignal<Vec<K>> = crate::signal(Vec::new());
    let pin: RwSignal<Option<(usize, f32)>> = crate::signal(None);
    // Where the first mounted row starts while the rows flow, which they do
    // while the view follows and the last row is mounted. The content then
    // ends at the laid-out bottom of the last row, which the follow pin
    // reaches in the frame the row grows, where the index's end would lag
    // until the measurement lands.
    let lead: RwSignal<Option<f32>> = crate::signal(None);
    let following: RwSignal<Option<RwSignal<bool>>> = crate::signal(None);
    let reported = crate::signal(0u64);
    // Counts the edits that moved keys already in the list, which change the
    // places the mounted rows report. Appends, measurements and scrolls move
    // none.
    let order = crate::signal(0u64);
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
        let lead = lead.clone();
        let following = following.clone();
        let reported = reported.clone();
        let order = order.clone();
        let pending = Rc::clone(&pending);
        // The state a remount carried was built by the old code, whose key
        // and estimate this list's first run must check, even when the items
        // are the very ones the old list last saw.
        let first_run = std::cell::Cell::new(true);
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
            let follows = following.get().is_some_and(|signal| signal.get());

            let mut geometry_changed = false;
            let mut reordered = false;
            {
                let mut state = state.borrow_mut();
                // Flowing rows put the screen at the laid-out end, which an
                // index still waiting on measurements places too high or
                // too low. The distance from the bottom of the last row,
                // which the scroll binding and the wheel measure against the
                // index, is what the view keeps when it stops flowing, so
                // the measurements that land next move nothing on screen.
                // The row is found again by its key, so rows an edit appends
                // below it or removes above it move nothing either.
                let from_end = viewport
                    .filter(|_| lead.read(Option::is_some))
                    .zip(state.index.keys().last().cloned())
                    .map(|(viewport, last)| {
                        ((state.index.total() - viewport).max(0.0) - wanted, last)
                    });
                // The scroll of this frame joins the anchor before anything
                // re-aims it, so a measurement that lands in the same frame
                // compensates around where the view is now, not where it
                // was last frame.
                if !state.index.is_empty() {
                    let at = anchor_row(&state.index, wanted, viewport);
                    let within = wanted - state.index.offset_of(at);
                    state.anchor = Some((state.index.key(at).clone(), within));
                }
                let first = first_run.replace(false);
                if first && !state.index.is_empty() {
                    // The carried maps were hashed by the old code, whose
                    // hash of a key the patch may have changed, so they are
                    // hashed again by this code, heights kept.
                    let list = &mut *state;
                    list.measured = std::mem::take(&mut list.measured).into_iter().collect();
                    list.index.rehash();
                }
                let same_items = !first && Rc::ptr_eq(&*state.items.borrow(), &snapshot);
                *state.items.borrow_mut() = snapshot;
                if estimate == 0.0 {
                    state.index = HeightIndex::new();
                    state.anchor = None;
                } else {
                    if state.estimate != estimate {
                        let list = &mut *state;
                        let keys: Vec<K> = list.index.keys().to_vec();
                        let measured = &list.measured;
                        list.index = HeightIndex::from_heights(keys, |key| {
                            measured.get(key).copied().unwrap_or(estimate)
                        });
                        list.estimate = estimate;
                        geometry_changed = true;
                    }
                    if !same_items {
                        let list = &mut *state;
                        let items = Rc::clone(&*list.items.borrow());
                        let old_len = list.index.len();
                        // Every old key is compared in place against the one
                        // the index stores, so an edit anywhere in the old
                        // items rebuilds, and an append keys each item once
                        // without copying a stored key.
                        let appended = items.len() >= old_len
                            && items[..old_len]
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
                            reordered = true;
                        }
                    }
                    for (item_key, height) in batch {
                        if height > 0.0 && height < state.smallest {
                            state.smallest = height;
                        }
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
                    if let Some(((from_end, last), viewport)) = from_end.zip(viewport)
                        && let Some(at) = state.index.position(&last)
                    {
                        let flowed_end = state.index.offset_of(at + 1);
                        wanted = (flowed_end - viewport).max(0.0) - from_end;
                    }
                }
                if let Some(reveal) = &reveal {
                    let reveal_key = reveal.get();
                    if let Some(reveal_key) = reveal_key.filter(|_| !state.index.is_empty()) {
                        reveal.set(None);
                        if let (Some(at), Some(viewport)) =
                            (state.index.position(&reveal_key), viewport)
                        {
                            let top = state.index.offset_of(at);
                            let bottom = state.index.offset_of(at + 1);
                            wanted = if top < wanted {
                                top
                            } else if bottom > wanted + viewport {
                                bottom - viewport
                            } else {
                                wanted
                            };
                        }
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
                    let at = anchor_row(&state.index, wanted, viewport);
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

            let state = state.borrow();
            let range = match (viewport, anchor_at) {
                (Some(viewport), Some((at, _))) if estimate > 0.0 => {
                    Some(window_range(&state.index, viewport, wanted, at))
                }
                _ => None,
            };
            let flows = follows && range.is_some_and(|(_, end)| end + 1 == state.index.len());
            // Flowing rows stack up from the start of the window, so rows
            // that lay out shorter than the index says would leave the top of
            // the viewport empty. The window reaches back far enough to fill
            // it, counting each row no measurement reached at the smallest
            // height a row has shown.
            let range = match (range, viewport) {
                (Some((start, end)), Some(viewport)) if flows => {
                    let reach = flow_reach(&state, end, viewport, estimate);
                    Some((start.min(reach), end))
                }
                _ => range,
            };
            let next = range.map_or(&[][..], |(start, end)| &state.index.keys()[start..=end]);
            let next_lead = range
                .filter(|_| flows)
                .map(|(start, _)| state.index.offset_of(start));
            // The anchored row sits where the index puts it, and the rows
            // around it stack from it with their laid-out heights, so a row
            // that grows before its measurement reaches the index pushes its
            // neighbours away instead of covering them. Flowing rows start
            // at the lead instead, so the first one stays in place.
            let next_pin = match next_lead {
                Some(_) => Some((0, 0.0)),
                None => anchor_at
                    .zip(range)
                    .map(|((at, top), (start, _))| (at - start, top)),
            };
            // A scroll inside the mounted rows keeps the window, so the keys
            // are compared before anything is copied, and a new window
            // reuses the buffer of the old one.
            if window.read(|value| value.as_slice() != next) {
                window.update(|value| {
                    value.clear();
                    value.extend_from_slice(next);
                });
            }
            if pin.read(|value| *value != next_pin) {
                pin.set(next_pin);
            }
            if reordered {
                order.update(|count| *count += 1);
            }
            if lead.read(|value| *value != next_lead) {
                lead.set(next_lead);
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
                // A column stretches the row's content to the list's width,
                // as any column does with its children.
                let measured = Element::new(MeasureRow {
                    report: Rc::new(report),
                })
                .column()
                .shrink(0.0);
                if !list {
                    return measured.child(content);
                }
                // A row's place moves only when an edit moves the keys before
                // it, which the order counter reports.
                let position = {
                    let state = Rc::clone(&state);
                    let order = order.clone();
                    let item_key = item_key.clone();
                    move || {
                        order.with(|_| ());
                        state.borrow().index.position(&item_key).unwrap_or(0)
                    }
                };
                measured
                    .role(Role::ListItem)
                    .accessible_position(position)
                    .child(content)
            },
        )
        .column()
        .shrink(0.0)
        .pin_child(move || pin.get())
    };

    let content = panel().column();
    let content = if list {
        content
            .role(Role::List)
            .accessible_set_size(move || current_items.with(|snapshot| snapshot.0.len()))
    } else {
        content
    };
    let content = content
        .height({
            let total = total.clone();
            let lead = lead.clone();
            move || match lead.get() {
                Some(_) => Length::Auto,
                None => px(total.get()),
            }
        })
        .child(
            panel()
                .shrink(0.0)
                .height(move || px(lead.get().unwrap_or(0.0))),
        )
        .child(rows);

    // The view clamps the offset with the total the content panel takes in
    // this frame's layout. The painted content height lags a frame, and a
    // re-aim that grows the list at its end would be clamped back by it.
    scroll_with_visible_height(content, visible, Some(total), following, true).offset(offset)
}

/// Returns the most rows the window mounts for a viewport `viewport`
/// logical pixels tall, the overscan included.
///
/// Rows a pixel tall or more show at most one per pixel plus a partial one,
/// so a full viewport is never cut.
fn window_cap(viewport: f32) -> usize {
    MIN_WINDOW_CAP.max((viewport.ceil() as usize).saturating_add(1)) + 2 * OVERSCAN
}

/// Returns the first row a flowing window mounts so the rows up to `end`
/// fill a viewport `viewport` logical pixels tall, the overscan included and
/// within the window cap.
///
/// A measured row counts its height. A row no measurement reached counts
/// the smallest height any row has shown, or `estimate` when that is less,
/// since it may lay out that short.
fn flow_reach<T, K: Eq + Hash + Clone>(
    state: &KeyedList<T, K>,
    end: usize,
    viewport: f32,
    estimate: f32,
) -> usize {
    let unmeasured = state.smallest.min(estimate);
    let lowest = (end + 1).saturating_sub(window_cap(viewport) - 2 * OVERSCAN);
    let mut reach = end;
    let mut covered = 0.0;
    loop {
        let key = state.index.key(reach);
        covered += if state.measured.contains_key(key) {
            state.index.height(reach)
        } else {
            unmeasured
        };
        if covered >= viewport || reach <= lowest {
            break;
        }
        reach -= 1;
    }
    reach.saturating_sub(OVERSCAN)
}

/// Returns the row the list anchors on at `offset`: the row that holds the
/// pixel at `offset`, or, when `offset` is that row's top, the first of the
/// zero-height rows right before it, which share that top.
///
/// Rows that measure nothing at the top of the viewport therefore stay in
/// the window below the anchor, and one that grows again pushes the rows
/// after it down in view. Stacked above the anchor it would grow above the
/// viewport, where it never paints, so its new height would never land.
/// The walk back takes no more rows than the window cap leaves after the
/// visible ones.
///
/// # Panics
///
/// Panics when the index is empty.
fn anchor_row<K: Eq + Hash + Clone>(
    index: &HeightIndex<K>,
    offset: f32,
    viewport: Option<f32>,
) -> usize {
    let at = index.index_at(offset);
    if index.offset_of(at) != offset {
        return at;
    }
    let viewport = viewport.unwrap_or(0.0);
    let last = index.index_at(offset + viewport);
    let spare = window_cap(viewport).saturating_sub(2 * OVERSCAN + (last - at) + 1);
    let mut first = at;
    while first > 0 && at - first < spare && index.height(first - 1) == 0.0 {
        first -= 1;
    }
    first
}

/// Returns the first and the last position of the rows the window mounts:
/// from the anchored row `first` to the last row the viewport shows, plus
/// the overscan on both sides, and at most the window cap.
///
/// # Panics
///
/// Panics when the index is empty.
fn window_range<K: Eq + Hash + Clone>(
    index: &HeightIndex<K>,
    viewport: f32,
    offset: f32,
    first: usize,
) -> (usize, usize) {
    let last = index.index_at(offset + viewport);
    let start = first.saturating_sub(OVERSCAN);
    let end = (last + OVERSCAN)
        .min(index.len() - 1)
        .min(start.saturating_add(window_cap(viewport) - 1));
    (start, end)
}

/// A keyed virtual list whose rows the user reorders by dragging one onto
/// another, or by pressing Alt with an arrow key while a row holds the
/// focus.
///
/// It wraps [`virtual_list_keyed`] and keeps that list's every behaviour:
/// rows mount near the viewport, keep their measured heights across edits,
/// and survive a move unmounted-by-nothing, because the move writes the new
/// order to `items` once and the rows are keyed, so the moved row changes
/// place while every other row keeps its widget and its state.
///
/// While a row drags, a line in the [`ColorToken::ACCENT`] token marks
/// where it would land, between the row the pointer is over and its
/// neighbour. Releasing the press moves the dragged item there; releasing
/// anywhere else, or pressing Escape, cancels the move. With the focus on a
/// row, Alt+Up and Alt+Down move it one place, and Alt+Home and Alt+End
/// move it to the start and the end, the view following so the moved row
/// stays mounted and keeps the focus. Every move announces where the row
/// went, as "Moved to position 3 of 12", through an announcer this list
/// attaches to itself.
///
/// Each row is a drag source whose payload is the row's key tagged with an
/// identity of this list, and a drop target that accepts only payloads of
/// its own list, so the rows of two reorderable lists never move items
/// into each other, even when both hold the same keys.
///
/// The items live in a `RwSignal<Vec<T>>`, so a move can write the new
/// order back: every edit hands the list a fresh `Vec`, the way a plain
/// `RwSignal<Vec<T>>` read does anyway.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_text::label;
/// use smudge_widgets::reorderable_list;
///
/// let playlist: RwSignal<Vec<u32>> = RwSignal::new(vec![1, 2, 3]);
/// let list = reorderable_list(
///     playlist,
///     |track| *track,
///     48.0,
///     |track| label(format!("Track {track}")),
/// )
/// .height(px(480.0));
/// ```
#[track_caller]
pub fn reorderable_list<T: Clone + 'static, K: Eq + Hash + Clone + 'static, E: IntoElement>(
    items: RwSignal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    estimate: f32,
    row: impl Fn(&T) -> E + 'static,
) -> Element<ScrollView> {
    let key = Rc::new(smudge_core::hot::wrap1_ref(key));
    let row = Rc::new(smudge_core::hot::wrap1_ref(row));
    // Two lists whose key spaces overlap must still refuse each other's
    // drags, so every payload of this list carries an identity only this
    // list's rows build.
    let identity = Rc::new(());
    // Where a drag of this list would land: the key of the row the pointer
    // hovers and whether it would land after that row. `None` while no drag
    // hovers the list, which hides both line halves of every row.
    let slot: RwSignal<Option<(K, bool)>> = crate::signal(None);
    // The hover the row under the pointer reported last. A plain press
    // writes it too, before its drag grows a payload, so the row keeps it
    // out of the reactive `slot` until a target's `accepts` hears the
    // payload, which only a drag that left its source reaches.
    let hover: Rc<RefCell<Option<(K, bool)>>> = Rc::new(RefCell::new(None));
    // The row a keyboard move just sent out of the viewport, which the
    // list asks the view to bring back so the row stays mounted and keeps
    // the focus.
    let reveal: RwSignal<Option<K>> = crate::signal(None);
    // An edit to the items can remove the row a drag flies from, whose
    // drag end then runs nowhere and would leave the last line lit. The
    // watch clears whatever a drag left behind whenever the items change.
    let watch_items = items.clone();
    let watch_slot = slot.clone();
    let watch_hover = Rc::clone(&hover);
    RenderEffect::new(move || {
        watch_items.with(|_| ());
        if watch_slot.read(|current| current.is_some()) {
            watch_slot.set(None);
        }
        *watch_hover.borrow_mut() = None;
    });
    let announcer = Announcer::new();

    let list = {
        let view_items = items.clone();
        let view_key = Rc::clone(&key);
        let row_items = items.clone();
        let row_key = Rc::clone(&key);
        let row_row = Rc::clone(&row);
        let row_slot = slot.clone();
        let row_hover = Rc::clone(&hover);
        let row_reveal = reveal.clone();
        let row_announcer = announcer.clone();
        let row_identity = Rc::clone(&identity);
        keyed(
            move || Rc::from(view_items.get().as_slice()),
            move |item| view_key(item),
            estimate,
            move |item| {
                let items = row_items.clone();
                let slot = row_slot.clone();
                let hover = Rc::clone(&row_hover);
                let reveal = row_reveal.clone();
                let announcer = row_announcer.clone();
                let key = Rc::clone(&row_key);
                let row = Rc::clone(&row_row);
                let identity = Rc::clone(&row_identity);
                let my_key = key(item);
                let height = Rc::new(Cell::new(f32::INFINITY));
                let report_height = Rc::clone(&height);
                Element::new(MeasureRow {
                    report: Rc::new(move |measured| report_height.set(measured.max(0.0))),
                })
                .column()
                .shrink(0.0)
                .role(Role::Group)
                .child(row(item))
                .child(drop_line(&slot, &my_key, false))
                .child(drop_line(&slot, &my_key, true))
                .drag_source({
                    let identity = Rc::clone(&identity);
                    let my_key = my_key.clone();
                    move || (Rc::clone(&identity), my_key.clone())
                })
                .drop_target(
                    {
                        let items = items.clone();
                        let key = Rc::clone(&key);
                        let slot = slot.clone();
                        let hover = Rc::clone(&hover);
                        let identity = Rc::clone(&identity);
                        move |payload: &(Rc<()>, K)| {
                            // A drag from another list never lands here,
                            // whatever its keys say.
                            if !Rc::ptr_eq(&payload.0, &identity) {
                                return false;
                            }
                            let member =
                                items.read(|list| list.iter().any(|item| key(item) == payload.1));
                            if !member {
                                return false;
                            }
                            // The landing needs a hovered row that still
                            // exists, and hovering the dragged row itself
                            // marks a landing on itself, which moves
                            // nothing and would light a line for a move
                            // that never happens.
                            let landed = hover.borrow().clone();
                            let live = landed.as_ref().is_some_and(|(edge, _)| {
                                *edge != payload.1
                                    && items.read(|list| list.iter().any(|item| key(item) == *edge))
                            });
                            if live && slot.read(|current| *current != landed) {
                                slot.set(landed);
                            }
                            live
                        }
                    },
                    {
                        let items = items.clone();
                        let key = Rc::clone(&key);
                        let announcer = announcer.clone();
                        let my_key = my_key.clone();
                        let height = Rc::clone(&height);
                        move |payload: (Rc<()>, K), at| {
                            // The edge comes from the release position the
                            // core hands over, not from the last hover,
                            // which can trail a fast pointer.
                            let after = at.y * 2.0 > height.get();
                            let target = items.read(|list| {
                                let from = list.iter().position(|item| key(item) == payload.1)?;
                                let mut to = list.iter().position(|item| key(item) == my_key)?;
                                if after {
                                    to += 1;
                                }
                                // Removing the dragged item first shifts every
                                // position after it one place up.
                                if to > from {
                                    to -= 1;
                                }
                                Some((from, to))
                            });
                            let Some((from, to)) = target else {
                                return;
                            };
                            if from == to {
                                return;
                            }
                            items.update(|list| {
                                let item = list.remove(from);
                                list.insert(to, item);
                            });
                            let count = items.read(|list| list.len());
                            announcer.announce(
                                format!("Moved to position {} of {}", to + 1, count),
                                Politeness::Polite,
                            );
                        }
                    },
                )
                .on_drag_over({
                    let hover = Rc::clone(&hover);
                    let my_key = my_key.clone();
                    move |at| {
                        let after = at.y * 2.0 > height.get();
                        *hover.borrow_mut() = Some((my_key.clone(), after));
                    }
                })
                .on_drag_end({
                    let slot = slot.clone();
                    let hover = Rc::clone(&hover);
                    move |_delivered| {
                        slot.set(None);
                        *hover.borrow_mut() = None;
                    }
                })
                .on_key({
                    let items = items.clone();
                    let key = Rc::clone(&key);
                    let my_key = my_key.clone();
                    let reveal = reveal.clone();
                    let announcer = announcer.clone();
                    move |event: &KeyEvent| {
                        if !event.pressed || !event.modifiers.alt {
                            return Handled::No;
                        }
                        let here = items.read(|list| {
                            list.iter()
                                .position(|item| key(item) == my_key)
                                .map(|from| (from, list.len()))
                        });
                        let Some((from, count)) = here else {
                            return Handled::No;
                        };
                        let last = count - 1;
                        let move_to = match event.key {
                            Key::ArrowUp if from > 0 => Some(from - 1),
                            Key::ArrowDown if from < last => Some(from + 1),
                            Key::Home if from > 0 => Some(0),
                            Key::End if from < last => Some(last),
                            // The row sits at the edge the key asks to cross.
                            // The list takes the key, so the scroll view does
                            // not also read it as a scroll.
                            Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End => None,
                            _ => return Handled::No,
                        };
                        let Some(to) = move_to else {
                            return Handled::Yes;
                        };
                        items.update(|list| {
                            let item = list.remove(from);
                            list.insert(to, item);
                        });
                        // The moved row can land far outside the viewport,
                        // where the view would unmount it and drop the
                        // focus. The view aims itself at the row again.
                        reveal.set(Some(my_key.clone()));
                        announcer.announce(
                            format!("Moved to position {} of {}", to + 1, count),
                            Politeness::Polite,
                        );
                        Handled::Yes
                    }
                })
            },
            true,
            None,
            Some(reveal.clone()),
        )
    };
    list.on_drag_over({
        let slot = slot.clone();
        let hover = Rc::clone(&hover);
        move |_at| {
            // The rows' own handlers sit deeper in the hit chain, so this
            // runs only while no row is under the pointer, where no
            // landing is.
            *hover.borrow_mut() = None;
            if slot.read(|current| current.is_some()) {
                slot.set(None);
            }
        }
    })
    .announcer(&announcer)
}

/// Builds the line one row draws at its top or bottom edge while a drag of
/// the list would land there.
///
/// The line hangs out of flow over the row's edge, so showing it never moves
/// the rows, and it hides again as soon as the drag would land elsewhere.
fn drop_line<K: Eq + Clone + 'static>(
    slot: &RwSignal<Option<(K, bool)>>,
    key: &K,
    after: bool,
) -> Element<Panel> {
    let slot = slot.clone();
    let key = key.clone();
    panel()
        .height(px(DROP_INDICATOR))
        .background(ColorToken::ACCENT)
        .absolute()
        .inset(Edges {
            top: if after { Length::Auto } else { Length::px(0.0) },
            bottom: if after { Length::px(0.0) } else { Length::Auto },
            left: Length::px(0.0),
            right: Length::px(0.0),
        })
        .hidden(move || {
            !slot
                .get()
                .is_some_and(|(hovered, landed_after)| landed_after == after && hovered == key)
        })
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

#[cfg(test)]
#[path = "virtual_list_keyed_tests.rs"]
mod tests;
