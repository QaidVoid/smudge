//! The virtual list: a scroll view that keeps only the visible rows mounted.
//!
//! The list never gives the layout engine more than a window of rows, so a
//! list of any length costs the tree and the layout the same as its visible
//! part. The rows hang, absolutely positioned, from a spacer panel as tall as
//! the whole list, which is what keeps the scrollbar and the wheel geometry
//! honest while the mounted window moves.

use std::rc::Rc;

use smudge_core::element::{Element, IntoElement, for_each};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::panel;

use crate::scroll::{ScrollExt, ScrollView, scroll_with_visible_height};

/// The rows mounted above and below the viewport, so a small scroll shows
/// content in the frame before the mount follows it.
const OVERSCAN: usize = 4;

/// Creates a list of `count` rows of one height inside a scroll view, with
/// only the rows near the viewport mounted.
///
/// The returned element is the scroll view; give it a height, either directly
/// or through a parent that sizes it. It scrolls with the wheel like any
/// scroll view, and it shows a thumb for the whole list, because its content
/// is a spacer panel as tall as `count` rows, whatever the mounted window is.
///
/// The rows are built by `row`, which receives the row's index. A row is
/// built once, the first time its index enters the window, and it stays
/// mounted, with its reactive state, until the window moves past it. Content
/// that changes while a row stays mounted must come from the signals that row
/// reads: a new value for an index that never left the window reaches no
/// mounted row. The row should keep to `row_height`; a taller one overlaps
/// the row below it, and a shorter one leaves the spacer's background showing.
///
/// `count` runs inside a binding, so only the signals it reads remount the
/// window. A shrinking count clamps the offset, and the window follows on the
/// frame after the new height has been measured.
///
/// The list owns its scroll offset; do not call
/// [`ScrollExt::offset`](crate::ScrollExt::offset) on the returned view,
/// because the window would follow the signal the list started with while the
/// view scrolls by the one the call installs.
///
/// `row_height` must be positive and finite. Anything else mounts no rows,
/// and a list built that way shows an empty view.
///
/// ```
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_text::label;
/// use smudge_widgets::virtual_list;
///
/// let count = RwSignal::new(100_000);
/// let list = virtual_list(
///     move || count.get(),
///     24.0,
///     |index| label(format!("Row {index}")),
/// )
/// .height(px(480.0));
/// ```
pub fn virtual_list<E: IntoElement>(
    count: impl Fn() -> usize + 'static,
    row_height: f32,
    row: impl Fn(usize) -> E + 'static,
) -> Element<ScrollView> {
    let row_height = if row_height.is_finite() && row_height > 0.0 {
        row_height
    } else {
        0.0
    };
    let count = Rc::new(count);
    let row = Rc::new(row);
    let visible: RwSignal<Option<f32>> = RwSignal::new(None);
    let offset = RwSignal::new(0.0);

    let range_count = Rc::clone(&count);
    let range_offset = offset.clone();
    let range_visible = visible.clone();
    let rows = for_each(
        move || {
            let count = range_count();
            let Some(viewport) = range_visible.get() else {
                return Vec::new();
            };
            if row_height == 0.0 {
                return Vec::new();
            }
            let offset = range_offset.get();
            let first = (offset / row_height).floor();
            let last = ((offset + viewport) / row_height).ceil();
            let start = (first as usize).saturating_sub(OVERSCAN);
            let end = (last as usize).saturating_add(OVERSCAN).min(count);
            (start..end).collect()
        },
        |&index| index,
        move |index| {
            panel()
                .absolute()
                .inset(Edges {
                    top: px(index as f32 * row_height),
                    right: px(0.0),
                    bottom: Length::Auto,
                    left: px(0.0),
                })
                .child(row(index))
        },
    );

    let height_count = Rc::clone(&count);
    // A column stretches the keyed slot to the list's width, which the
    // absolutely positioned rows then span.
    let content = panel()
        .column()
        .height(move || px(height_count() as f32 * row_height))
        .child(rows);

    scroll_with_visible_height(content, visible).offset(offset)
}
