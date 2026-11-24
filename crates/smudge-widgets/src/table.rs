//! A table: a header over a virtual list of rows, whose columns scroll
//! sideways together and whose cells truncate to one line.

use std::cell::RefCell;
use std::hash::Hash;
use std::rc::Rc;

use smudge_core::element::{AnyElement, Element, IntoElement};
use smudge_core::geometry::Point;
use smudge_core::hot;
use smudge_core::reactive::{Memo, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Align, Justify, px};
use smudge_core::widget::{Panel, panel};
use smudge_text::{LabelExt, label};

use crate::containers::row;
use crate::scroll::{ScrollAxis, ScrollExt, scroll};
use crate::signal;
use crate::virtual_list_keyed::virtual_column_keyed;

/// The height of the header and of every row, in logical pixels. Cells
/// truncate to one line, so every row measures the same.
pub const ROW_HEIGHT: f32 = 28.0;

/// One column of a table, which [`table`] takes.
#[derive(Clone, Debug)]
pub struct Column {
    /// The name the header cell shows.
    pub title: String,
    /// How wide the column is, in logical pixels.
    pub width: f32,
    /// How narrow the column may become before it stops, in logical
    /// pixels, which a resize honours.
    pub min_width: f32,
    /// Where the cell's content sits across the column.
    pub alignment: Align,
    /// Whether pressing the header may sort by the column.
    pub sortable: bool,
}

impl Column {
    /// Creates a column titled `title` that is `width` logical pixels
    /// wide, never narrower than `min_width`, whose cells align along
    /// `alignment`, and that sorts only when `sortable` holds.
    #[track_caller]
    pub fn new(
        title: impl Into<String>,
        width: f32,
        min_width: f32,
        alignment: Align,
        sortable: bool,
    ) -> Column {
        Column {
            title: title.into(),
            width,
            min_width,
            alignment,
            sortable,
        }
    }
}

/// Creates a table of `columns` over `rows`, whose rows the `key` function
/// identifies and whose cells the `cell` function builds.
///
/// The body is a keyed virtual list in a scroll view that also scrolls
/// sideways, so a table whose columns add up wider than the table scrolls
/// sideways under a header that stays in view: the header sits above the
/// body, outside its vertical scroll, and its offset follows the body's
/// horizontal one. The rows mount only near the viewport, and only the
/// mounted rows have nodes, exactly as the keyed lists report.
///
/// Size the table like any scroll view, with the width and height
/// builders: its columns never stretch it past the width the app gives
/// it.
///
/// Every cell clips its content, and a header cell ends in an ellipsis
/// when its title is wider than its column. Every row is [`ROW_HEIGHT`]
/// tall, so a row's place is exact however wide its text runs.
///
/// A screen reader hears a table: the table reports how many rows and
/// columns it holds, the ones off screen included, each mounted row its
/// row index, and each cell its row and column index.
///
/// The table keeps the columns it was built with, and a row's index and a
/// cell's indexes are read when the row mounts, so a row that stays
/// mounted while an edit reorders the rows keeps the index it was built
/// with until its key leaves the window.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::{Align, px};
/// use smudge_text::label;
/// use smudge_widgets::{Column, table};
///
/// let rows: RwSignal<Rc<[u64]>> = RwSignal::new(Rc::from([7, 3, 11]));
/// let view = table(
///     [
///         Column::new("Name", 180.0, 80.0, Align::Start, true),
///         Column::new("Size", 80.0, 40.0, Align::End, false),
///     ],
///     move || rows.get().clone(),
///     |row| *row,
///     |row, column_index| {
///         if column_index == 0 {
///             label(format!("Item {row}"))
///         } else {
///             label(format!("{row} kB"))
///         }
///     },
/// )
/// .width(px(300.0))
/// .height(px(400.0));
/// ```
#[track_caller]
pub fn table<T, K, E>(
    columns: impl IntoIterator<Item = Column>,
    rows: impl Fn() -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    cell: impl Fn(&T, usize) -> E + 'static,
) -> Element<Panel>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
    E: IntoElement,
{
    let columns: Rc<[Column]> = columns.into_iter().collect();
    let column_count = columns.len();
    let horizontal = signal(0.0f32);
    let rows = hot::wrap(rows);
    let key = Rc::new(hot::wrap1_ref(key));
    let cell = RefCell::new(hot::wrap2_ref_mut(cell));

    // The rows with their index beside them, so a mounted row reports the
    // index it was built with and no frame scans the rows for one.
    let current_rows = Memo::new(move || {
        let rows = rows();
        IndexedRows(Rc::from(
            rows.iter()
                .enumerate()
                .map(|(index, row)| IndexedRow {
                    row: row.clone(),
                    key: key(row),
                    index,
                })
                .collect::<Vec<_>>(),
        ))
    });

    let header = {
        let columns = Rc::clone(&columns);
        let horizontal = horizontal.clone();
        row()
            .shrink(0.0)
            // The titles slide sideways under an offset scope, the way the
            // body's scroll view moves its content, so a scroll repaints
            // neither the header's cells nor the body's rows. The offset is
            // the row's own, so the row's children move by minus it.
            .scroll_offset(move || Point {
                x: horizontal.get(),
                y: 0.0,
            })
            .children(
                columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| header_cell(index, column)),
            )
    };

    let list_view = current_rows.clone();
    let list = virtual_column_keyed(
        move || list_view.with(|rows| Rc::clone(&rows.0)),
        |indexed: &IndexedRow<T, K>| indexed.key.clone(),
        ROW_HEIGHT,
        move |indexed: &IndexedRow<T, K>| {
            let row_index = indexed.index;
            let item = &indexed.row;
            let columns = Rc::clone(&columns);
            let mut cell = cell.borrow_mut();
            let cells = columns
                .iter()
                .enumerate()
                .map(|(column_index, column)| {
                    let content: AnyElement = cell(item, column_index).into_any();
                    panel()
                        .role(Role::Cell)
                        .accessible_row_index(row_index)
                        .accessible_column_index(column_index)
                        .width(px(column.width))
                        .height(px(ROW_HEIGHT))
                        .shrink(0.0)
                        .clip(true)
                        .justify_content(justify(column.alignment))
                        .align_items(Align::Center)
                        .child(content)
                })
                .collect::<Vec<_>>();
            row()
                .role(Role::Row)
                .accessible_row_index(row_index)
                .height(px(ROW_HEIGHT))
                .shrink(0.0)
                .children(cells)
        },
    )
    // The body's list fills the view it sits in: the view stretches its
    // content box to its own height, so the list keeps the vertical scroll
    // and the view keeps the sideways one. A view that scrolled both ways
    // would lay its content out at the content's own height instead, and
    // the list inside it would have no height to keep.
    .grow(1.0);

    panel()
        .column()
        .role(Role::Table)
        .accessible_row_count(move || current_rows.with(|rows| rows.0.len()))
        .accessible_column_count(column_count)
        .child(
            panel()
                .row()
                .clip(true)
                .height(px(ROW_HEIGHT))
                .shrink(0.0)
                .child(header),
        )
        .child(
            scroll(list)
                .axis(ScrollAxis::Horizontal)
                .horizontal_offset(horizontal)
                .grow(1.0),
        )
}

/// A row of the data with its index beside it, which the body's list keys
/// by the row's key.
struct IndexedRow<T, K> {
    /// The row itself.
    row: T,
    /// The row's key, which the list keys the row by.
    key: K,
    /// The row's place in the rows the data holds.
    index: usize,
}

/// The indexed rows, compared by identity, so the memo that holds them
/// hands a new value on only when the data handed back a new `Rc`.
struct IndexedRows<T, K>(Rc<[IndexedRow<T, K>]>);

impl<T, K> PartialEq for IndexedRows<T, K> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Builds one header cell for the column at `index`.
fn header_cell(index: usize, column: &Column) -> Element<Panel> {
    panel()
        .role(Role::ColumnHeader)
        .accessible_column_index(index)
        .width(px(column.width))
        .height(px(ROW_HEIGHT))
        .shrink(0.0)
        .justify_content(justify(column.alignment))
        .align_items(Align::Center)
        .child(label(column.title.clone()).max_lines(Some(1)))
}

/// Maps a column's alignment onto where the content sits on the main axis.
fn justify(alignment: Align) -> Justify {
    match alignment {
        Align::End => Justify::End,
        Align::Center => Justify::Center,
        Align::Start | Align::Stretch => Justify::Start,
    }
}
