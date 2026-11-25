//! A table: a header over a virtual list of rows, whose columns scroll
//! sideways together, whose cells truncate to one line, whose header
//! resizes and sorts its columns, and whose rows select with the pointer
//! and the keyboard.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use crate::canvas::{Canvas, canvas};
use crate::containers::row;
use crate::scroll::{ScrollAxis, ScrollExt, scroll};
use crate::signal;
use crate::virtual_list_keyed::keyed;

use smudge_core::element::{AnyElement, Element, IntoElement, Value, show};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::hot;
use smudge_core::input::{Drag, Handled, Key, PointerButton};
use smudge_core::path::Path;
use smudge_core::reactive::{Memo, RwSignal, prelude::*};
use smudge_core::semantics::{Role, SortDirection};
use smudge_core::style::{Align, Direction, Edges, Justify, Length, px};
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::widget::{Panel, panel};
use smudge_core::window::Cursor;
use smudge_text::{LabelExt, label};

/// The height of the header and of every row, in logical pixels. Cells
/// truncate to one line, so every row measures the same.
pub const ROW_HEIGHT: f32 = 28.0;

/// How wide the strip at a header cell's right edge is, where a press
/// starts a resize drag.
const RESIZE_ZONE: f32 = 6.0;

/// One column of a table, which [`table`] takes.
#[derive(Clone, Debug)]
pub struct Column {
    /// The name the header cell shows.
    pub title: String,
    /// How wide the column starts out, in logical pixels. A drag on the
    /// header cell's edge, or a write to the widths signal the view
    /// exposes, moves it from here.
    pub width: f32,
    /// How narrow the column may become before a resize stops it, in
    /// logical pixels.
    pub min_width: f32,
    /// Where the cell's content sits across the column.
    pub alignment: Align,
    /// Whether pressing the header may sort by the column.
    pub sortable: bool,
}

impl Column {
    /// Creates a column titled `title` that starts `width` logical pixels
    /// wide, never narrows below `min_width`, whose cells align along
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

/// A table, which [`table`] creates.
pub struct TableView<K: Eq + Hash + Clone> {
    /// How wide every column is, in order. A resize writes it, and the
    /// header and the mounted rows lay out from it.
    widths: RwSignal<Vec<f32>>,
    /// Which column the user asked to sort by, and which way. The table
    /// never reorders the rows; the application reads this and sorts its
    /// own data.
    sort: RwSignal<Option<(usize, SortDirection)>>,
    /// The keys of the rows the user selected.
    selection: RwSignal<HashSet<K>>,
    /// The key of the row the keyboard focus sits on.
    focused: RwSignal<Option<K>>,
    /// Whether presses add to the selection beside the rows already
    /// selected.
    multiple: Rc<Cell<bool>>,
    /// The table's element.
    content: Element<Panel>,
}

impl<K: Eq + Hash + Clone + 'static> TableView<K> {
    /// Returns the signal holding how wide every column is, so the
    /// application can widen a column itself or persist the widths.
    pub fn widths(&self) -> RwSignal<Vec<f32>> {
        self.widths.clone()
    }

    /// Returns the signal holding the column the user asked to sort by and
    /// the direction, so the application can sort its rows to match.
    pub fn sort(&self) -> RwSignal<Option<(usize, SortDirection)>> {
        self.sort.clone()
    }

    /// Returns the signal holding the keys of the selected rows, so the
    /// application can select rows itself or read the selection.
    pub fn selection(&self) -> RwSignal<HashSet<K>> {
        self.selection.clone()
    }

    /// Returns the signal holding the key of the row the keyboard focus
    /// sits on, so the application can move the focus itself.
    pub fn focused(&self) -> RwSignal<Option<K>> {
        self.focused.clone()
    }

    /// Lets presses add to the selection beside the rows already selected.
    /// Without this, a press replaces the selection with the one row.
    pub fn multiple(self, multiple: bool) -> TableView<K> {
        self.multiple.set(multiple);
        self
    }

    /// Sets the height, which the scroll view inside needs to virtualize.
    pub fn height(mut self, height: impl Into<Value<Length>>) -> TableView<K> {
        self.content = self.content.height(height);
        self
    }

    /// Sets the width.
    pub fn width(mut self, width: impl Into<Value<Length>>) -> TableView<K> {
        self.content = self.content.width(width);
        self
    }
}

impl<K: Eq + Hash + Clone + 'static> IntoElement for TableView<K> {
    fn into_any(self) -> AnyElement {
        self.content.into_any()
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
/// A drag on a header cell's right edge resizes the column, never below
/// its minimum, and writes the widths the [`TableView::widths`] signal
/// holds, so the header and the mounted rows lay out from the same
/// numbers. A press on a sortable header cycles its sort through
/// ascending, descending and none, written to [`TableView::sort`]; the
/// table never reorders the rows, the application does.
///
/// A press selects its row, Ctrl or Command adds it, and Shift extends
/// the selection from the last row a press or the keyboard started from,
/// when the view is [`TableView::multiple`]. The keyboard moves the focus
/// with Up, Down, Page Up, Page Down, Home and End, Space toggles the
/// focused row, and the view is one Tab stop. The view reveals a focused
/// row that lies outside the viewport by moving the body to it.
///
/// A screen reader hears a table: the table reports how many rows and
/// columns it holds, the ones off screen included, each mounted row its
/// row index, and each cell its row and column index, and each sorted
/// header its sort direction.
///
/// A row's index and a cell's indexes are read when the row mounts, so a
/// row that stays mounted while an edit reorders the rows keeps the index
/// it was built with until its key leaves the window.
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
/// .multiple(true)
/// .width(px(300.0))
/// .height(px(400.0));
/// ```
#[track_caller]
pub fn table<T, K, E>(
    columns: impl IntoIterator<Item = Column>,
    rows: impl Fn() -> Rc<[T]> + 'static,
    key: impl Fn(&T) -> K + 'static,
    cell: impl Fn(&T, usize) -> E + 'static,
) -> TableView<K>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
    E: IntoElement,
{
    let columns: Rc<[Column]> = columns.into_iter().collect();
    let column_count = columns.len();
    let horizontal = signal(0.0f32);
    let offset = signal(0.0f32);
    let visible: RwSignal<Option<f32>> = signal(None);
    let widths: RwSignal<Vec<f32>> = signal(
        columns
            .iter()
            .map(|column| column.width)
            .collect::<Vec<_>>(),
    );
    let sort: RwSignal<Option<(usize, SortDirection)>> = signal(None);
    let selection: RwSignal<HashSet<K>> = signal(HashSet::new());
    let focused: RwSignal<Option<K>> = signal(None);
    let multiple = Rc::new(Cell::new(false));
    // Where a Shift extension reaches from: the key and index of the last
    // row a press or a plain move started the selection at.
    let anchor: Rc<RefCell<Option<(K, usize)>>> = Rc::new(RefCell::new(None));
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

    // The key and index at a place in the rows, read at the event, so a
    // sort the application applied lands before the move.
    let key_at = {
        let current_rows = current_rows.clone();
        move |index: usize| {
            current_rows.with(|rows| {
                rows.0
                    .get(index)
                    .map(|row: &IndexedRow<T, K>| (row.key.clone(), row.index))
            })
        }
    };
    // Moves the body so the row at `index` stands inside the viewport.
    // Every row is exactly [`ROW_HEIGHT`] tall, so the row's top is its
    // index times the height.
    let reveal = {
        let offset = offset.clone();
        let visible = visible.clone();
        move |index: usize| {
            let Some(viewport) = visible.get() else {
                return;
            };
            if viewport <= 0.0 {
                return;
            }
            let top = index as f32 * ROW_HEIGHT;
            let current = offset.get();
            if top < current {
                offset.set(top);
            } else if top + ROW_HEIGHT > current + viewport {
                offset.set((top + ROW_HEIGHT - viewport).max(0.0));
            }
        }
    };
    // Adds the rows from the anchor through `to` to the selection.
    let extend_to = {
        let anchor = Rc::clone(&anchor);
        let selection = selection.clone();
        let key_at = key_at.clone();
        move |to: usize| {
            let from = anchor
                .borrow()
                .as_ref()
                .map(|&(_, index)| index)
                .unwrap_or(to);
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            let keys = key_at(lo)
                .into_iter()
                .chain((lo + 1..=hi).filter_map(&key_at))
                .map(|(key, _)| key)
                .collect::<Vec<_>>();
            selection.update(|set| {
                for key in keys {
                    set.insert(key);
                }
            });
        }
    };

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
                    .map(|(index, column)| header_cell(index, column, &widths, &sort)),
            )
    };

    let list_view = current_rows.clone();
    let row_widths = widths.clone();
    let row_state = RowState {
        selection: selection.clone(),
        focused: focused.clone(),
        multiple: Rc::clone(&multiple),
        anchor: Rc::clone(&anchor),
        visible: visible.clone(),
        current_rows: current_rows.clone(),
        reveal: Rc::new(reveal),
        extend_to: Rc::new(extend_to),
        key_at: Rc::new(key_at),
    };
    let list = keyed(
        move || list_view.with(|rows| Rc::clone(&rows.0)),
        |indexed: &IndexedRow<T, K>| indexed.key.clone(),
        ROW_HEIGHT,
        move |indexed: &IndexedRow<T, K>| {
            let row_index = indexed.index;
            let item = &indexed.row;
            let columns = Rc::clone(&columns);
            let widths = row_widths.clone();
            let mut cell = cell.borrow_mut();
            let cells = columns
                .iter()
                .enumerate()
                .map(|(column_index, column)| {
                    let cell_widths = widths.clone();
                    let content: AnyElement = cell(item, column_index).into_any();
                    panel()
                        .role(Role::Cell)
                        .accessible_row_index(row_index)
                        .accessible_column_index(column_index)
                        .width(move || px(cell_widths.with(|all| all[column_index])))
                        .height(px(ROW_HEIGHT))
                        .shrink(0.0)
                        .clip(true)
                        // Every cell insets its content, so an end-aligned
                        // column never butts its text against the one that
                        // follows it.
                        .padding_xy(Size {
                            width: 8.0,
                            height: 0.0,
                        })
                        .justify_content(justify(column.alignment))
                        .align_items(Align::Center)
                        .child(content)
                })
                .collect::<Vec<_>>();
            table_row(&row_state, indexed, cells)
        },
        false,
        Some((offset.clone(), visible.clone())),
        None,
    )
    // The body's list fills the view it sits in: the view stretches its
    // content box to its own height, so the list keeps the vertical scroll
    // and the view keeps the sideways one. A view that scrolled both ways
    // would lay its content out at the content's own height instead, and
    // the list inside it would have no height to keep.
    .grow(1.0)
    // A focus the framework moves, such as Tab, writes the offset the list
    // scrolls with, so a focused row at the edge pulls the body to it.
    .on_reveal(move |revealed| offset.set(revealed.y));

    TableView {
        widths,
        sort,
        selection,
        focused,
        multiple,
        content: panel()
            .column()
            .role(Role::Table)
            // One Tab stop: the rows collapse into it, and Tab re-enters on
            // the selected row, else the first mounted one.
            .navigate_with_arrows(Direction::Column)
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
            ),
    }
}

/// The state the rows of one table share, which [`table_row`] reads.
struct RowState<T, K> {
    /// The keys of the selected rows.
    selection: RwSignal<HashSet<K>>,
    /// The key of the row the focus sits on.
    focused: RwSignal<Option<K>>,
    /// Whether presses add to the selection.
    multiple: Rc<Cell<bool>>,
    /// Where a Shift extension reaches from.
    anchor: Rc<RefCell<Option<(K, usize)>>>,
    /// How tall the body's viewport is.
    visible: RwSignal<Option<f32>>,
    /// The rows with their indexes beside them.
    current_rows: Memo<IndexedRows<T, K>>,
    /// Moves the body so a row comes into view.
    reveal: Rc<dyn Fn(usize)>,
    /// Adds the rows from the anchor through a row to the selection.
    extend_to: Rc<dyn Fn(usize)>,
    /// The key and index at a place in the rows.
    key_at: Rc<dyn Fn(usize) -> Option<(K, usize)>>,
}

/// Builds one row of the body over `indexed`, holding `cells`, with the
/// selection, the keyboard and the focus of its table.
fn table_row<T, K>(
    state: &RowState<T, K>,
    indexed: &IndexedRow<T, K>,
    cells: Vec<Element<Panel>>,
) -> Element<Panel>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    let index = indexed.index;
    let selected_key = indexed.key.clone();
    let selected_signal = state.selection.clone();
    let fill_signal = selected_signal.clone();
    let fill_key = selected_key.clone();
    let press_key = indexed.key.clone();
    let press_anchor = Rc::clone(&state.anchor);
    let press_signal = state.selection.clone();
    let press_multiple = Rc::clone(&state.multiple);
    let press_extend = Rc::clone(&state.extend_to);
    let when_key = indexed.key.clone();
    let when_signal = state.focused.clone();
    let focus_key = indexed.key.clone();
    let focus_signal = state.focused.clone();
    let space_key = indexed.key.clone();
    let space_signal = state.selection.clone();
    let space_anchor = Rc::clone(&state.anchor);
    let move_anchor = Rc::clone(&state.anchor);
    let move_focus = state.focused.clone();
    let move_extend = Rc::clone(&state.extend_to);
    let move_key_at = Rc::clone(&state.key_at);
    let move_reveal = Rc::clone(&state.reveal);
    let count_rows = state.current_rows.clone();
    let page_visible = state.visible.clone();

    row()
        .role(Role::Row)
        .accessible_row_index(index)
        .accessible_selected(move || selected_signal.with(|set| set.contains(&selected_key)))
        .background(move || {
            if fill_signal.with(|set| set.contains(&fill_key)) {
                ColorValue::Token(ColorToken::SURFACE_SELECTED)
            } else {
                ColorValue::Literal(Color::TRANSPARENT)
            }
        })
        .height(px(ROW_HEIGHT))
        .shrink(0.0)
        .focusable()
        .focus_when(move || when_signal.with(|held| held.as_ref() == Some(&when_key)))
        .on_focus(move |gained| {
            if gained {
                focus_signal.set(Some(focus_key.clone()));
            }
        })
        .on_button_down(move |event| {
            if event.button != PointerButton::Primary {
                return Handled::No;
            }
            let key = press_key.clone();
            if event.modifiers.shift && press_multiple.get() {
                if press_anchor.borrow().is_none() {
                    *press_anchor.borrow_mut() = Some((key.clone(), index));
                }
                (press_extend)(index);
                return Handled::Yes;
            }
            *press_anchor.borrow_mut() = Some((key.clone(), index));
            if press_multiple.get() && event.modifiers.command() {
                press_signal.update(|set| {
                    if !set.remove(&key) {
                        set.insert(key.clone());
                    }
                });
                return Handled::Yes;
            }
            press_signal.update(|set| {
                set.clear();
                set.insert(key.clone());
            });
            Handled::Yes
        })
        .on_key(move |event| {
            if !event.pressed {
                return Handled::No;
            }
            let count = count_rows.with(|rows| rows.0.len());
            let page = page_visible
                .read(|height| height.map(|height| ((height / ROW_HEIGHT) as usize).max(1)))
                .unwrap_or(1);
            if event.key == Key::Space {
                let key = space_key.clone();
                *space_anchor.borrow_mut() = Some((key.clone(), index));
                space_signal.update(|set| {
                    if !set.remove(&key) {
                        set.insert(key.clone());
                    }
                });
                return Handled::Yes;
            }
            let target = match event.key {
                Key::ArrowUp => index.checked_sub(1),
                Key::ArrowDown => Some(index + 1).filter(|next| *next < count),
                Key::Home if count > 0 => Some(0),
                Key::End => count.checked_sub(1),
                Key::PageUp => index.checked_sub(page),
                Key::PageDown => Some((index + page).min(count.saturating_sub(1))),
                _ => None,
            };
            let Some(target) = target.filter(|target| *target < count) else {
                return Handled::No;
            };
            if event.modifiers.shift {
                (move_extend)(target);
            } else if let Some((key, at)) = (move_key_at)(target) {
                *move_anchor.borrow_mut() = Some((key, at));
            }
            if let Some((key, _)) = (move_key_at)(target) {
                move_focus.set(Some(key));
            }
            (move_reveal)(target);
            Handled::Yes
        })
        .children(cells)
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

/// Builds one header cell for the column at `index`, with the resize strip
/// at its edge and the sort the column carries.
fn header_cell(
    index: usize,
    column: &Column,
    widths: &RwSignal<Vec<f32>>,
    sort: &RwSignal<Option<(usize, SortDirection)>>,
) -> Element<Panel> {
    let sortable = column.sortable;
    let min_width = column.min_width;
    let title = column.title.clone();
    let cell_widths = widths.clone();
    let press_sort = sort.clone();
    let shown_sort = sort.clone();
    let cell_sort = sort.clone();
    let arrow_sort = sort.clone();
    panel()
        .role(Role::ColumnHeader)
        .accessible_column_index(index)
        .accessible_sort_direction(move || {
            shown_sort.with(|sorted| {
                sorted
                    .as_ref()
                    .filter(|(at, _)| *at == index)
                    .map(|(_, way)| *way)
            })
        })
        .width(move || px(cell_widths.with(|all| all[index])))
        .height(px(ROW_HEIGHT))
        .shrink(0.0)
        .padding_xy(Size {
            width: 8.0,
            height: 0.0,
        })
        .justify_content(justify(column.alignment))
        .align_items(Align::Center)
        .on_press(move || {
            if !sortable {
                return;
            }
            press_sort.update(|sorted| {
                *sorted = if let Some((at, way)) = *sorted
                    && at == index
                {
                    match way {
                        SortDirection::Ascending => Some((index, SortDirection::Descending)),
                        SortDirection::Descending | SortDirection::Other => None,
                    }
                } else {
                    Some((index, SortDirection::Ascending))
                };
            });
        })
        .child(
            row()
                .gap(Size {
                    width: 4.0,
                    height: 0.0,
                })
                .align_items(Align::Center)
                .child(label(title).max_lines(Some(1)))
                .child(show(
                    move || {
                        arrow_sort
                            .with(|sorted| sorted.as_ref().is_some_and(|(at, _)| *at == index))
                    },
                    move || {
                        let cell_sort = cell_sort.clone();
                        sort_arrow(move || {
                            cell_sort.with(|sorted| {
                                sorted
                                    .as_ref()
                                    .filter(|(at, _)| *at == index)
                                    .map(|(_, way)| *way)
                            })
                        })
                    },
                )),
        )
        .child(resize_strip(index, widths, min_width))
}

/// Draws the triangle that points the way the column sorts.
fn sort_arrow(direction: impl Fn() -> Option<SortDirection> + 'static) -> Element<Canvas> {
    canvas(move |painter| {
        let Some(way) = direction() else {
            return;
        };
        let size = painter.size();
        let inset = 1.0;
        let (left, right, top, bottom) = (inset, size.width - inset, inset, size.height - inset);
        let middle = (top + bottom) / 2.0;
        let points = match way {
            SortDirection::Ascending | SortDirection::Other => [
                Point { x: left, y: middle },
                Point {
                    x: right,
                    y: middle,
                },
                Point {
                    x: size.width / 2.0,
                    y: top,
                },
            ],
            SortDirection::Descending => [
                Point { x: left, y: middle },
                Point {
                    x: right,
                    y: middle,
                },
                Point {
                    x: size.width / 2.0,
                    y: bottom,
                },
            ],
        };
        painter.fill(
            &Path::polygon(points),
            ColorValue::Token(ColorToken::TEXT).resolve(painter.theme()),
        );
    })
    .width(px(9.0))
    .height(px(9.0))
    .shrink(0.0)
}

/// Builds the strip at a header cell's right edge that drags the column's
/// width, never below `min_width`.
fn resize_strip(index: usize, widths: &RwSignal<Vec<f32>>, min_width: f32) -> Element<Panel> {
    let start: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));
    let down_start = Rc::clone(&start);
    let drag_widths = widths.clone();
    let down_widths = widths.clone();
    panel()
        .absolute()
        .inset(move || Edges {
            left: Length::Auto,
            right: px(0.0),
            top: px(0.0),
            bottom: px(0.0),
        })
        .width(px(RESIZE_ZONE))
        .height(px(ROW_HEIGHT))
        .shrink(0.0)
        .cursor(Cursor::ColumnResize)
        // The strip takes the press so a release over it never runs the
        // press of the header cell around it: a drag that starts here
        // resizes and sorts nothing.
        .on_press(move || {})
        .on_button_down(move |event| {
            if event.button != PointerButton::Primary {
                return Handled::No;
            }
            down_start.set(Some((event.position.x, down_widths.read(|all| all[index]))));
            Handled::Yes
        })
        .on_drag(move |step| match step {
            Drag::Moved(at) => {
                if let Some((from, width)) = start.get() {
                    let next = (width + at.x - from).max(min_width);
                    drag_widths.update(|all| all[index] = next);
                }
                Handled::Yes
            }
            _ => {
                start.set(None);
                Handled::No
            }
        })
}

/// Maps a column's alignment onto where the content sits on the main axis.
fn justify(alignment: Align) -> Justify {
    match alignment {
        Align::End => Justify::End,
        Align::Center => Justify::Center,
        Align::Start | Align::Stretch => Justify::Start,
    }
}
