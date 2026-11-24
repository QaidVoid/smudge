//! Whole-frame behavior of the table's interactions, driven through the
//! headless harness: the resize drag that clamps at the minimum, the sort
//! that cycles and reports its direction, the selection by pointer and by
//! key, the keyboard that moves the focus and reveals a row far down the
//! table, and the one Tab stop the rows collapse into.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode, SortDirection};
use smudge_core::style::{Align, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{LabelExt, TextSystem, label};

use smudge_widgets::{Column, TableView, table};

/// The rows of the test table.
const ROWS: u64 = 100_000;

/// The signals of the table the window's build stashed, which the tests
/// read to hear what the user did to it.
#[derive(Clone)]
struct Signals {
    /// How wide every column is.
    widths: RwSignal<Vec<f32>>,
    /// Which column the user asked to sort by, and which way.
    sort: RwSignal<Option<(usize, SortDirection)>>,
    /// The keys of the selected rows.
    selection: RwSignal<HashSet<u64>>,
    /// The key of the row the focus sits on.
    focused: RwSignal<Option<u64>>,
}

/// A multiple-selection table of 100,000 rows in a window of 300 by 220,
/// settled, with its signals. The table is 200 tall, so the window keeps a
/// strip below it that a press can clear the focus with. The columns are
/// Name, 180 wide with a minimum of 80 and sortable, and Size, 80 wide
/// with a minimum of 40.
fn interactions_window() -> (Harness, Signals) {
    TextSystem::bundled_only().install();
    let rows: RwSignal<Rc<[u64]>> = RwSignal::new(Rc::from((0..ROWS).collect::<Vec<_>>()));
    let stash: Rc<RefCell<Option<Signals>>> = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(300.0, 220.0, 1.0, {
        let rows = rows.clone();
        let stash = Rc::clone(&stash);
        move || {
            let view = table(
                [
                    Column::new("Name", 180.0, 80.0, Align::Start, true),
                    Column::new("Size", 80.0, 40.0, Align::Start, false),
                ],
                move || rows.get().clone(),
                |row| *row,
                |row, column_index| {
                    if column_index == 0 {
                        label(format!("Item {row}")).max_lines(Some(1))
                    } else {
                        label(format!("{row} kB"))
                    }
                },
            )
            .multiple(true)
            .width(px(300.0))
            .height(px(200.0));
            *stash.borrow_mut() = Some(Signals {
                widths: view.widths(),
                sort: view.sort(),
                selection: view.selection(),
                focused: view.focused(),
            });
            panel().width(px(300.0)).height(px(200.0)).child(view)
        }
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 100.0 });
    let signals = stash.borrow().clone().expect("the build stashed signals");
    (harness, signals)
}

/// A single-selection table of a hundred rows, settled, with its signals.
fn single_window() -> (Harness, Signals) {
    TextSystem::bundled_only().install();
    let rows: RwSignal<Rc<[u64]>> = RwSignal::new(Rc::from((0..100u64).collect::<Vec<_>>()));
    let stash: Rc<RefCell<Option<Signals>>> = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(300.0, 220.0, 1.0, {
        let rows = rows.clone();
        let stash = Rc::clone(&stash);
        move || {
            let view = table(
                [Column::new("Name", 180.0, 80.0, Align::Start, false)],
                move || rows.get().clone(),
                |row| *row,
                |row, _| label(format!("Item {row}")).max_lines(Some(1)),
            )
            .width(px(300.0))
            .height(px(200.0));
            *stash.borrow_mut() = Some(Signals {
                widths: view.widths(),
                sort: view.sort(),
                selection: view.selection(),
                focused: view.focused(),
            });
            panel().width(px(300.0)).height(px(200.0)).child(view)
        }
    });
    harness.settle();
    harness.move_to(Point { x: 100.0, y: 100.0 });
    let signals = stash.borrow().clone().expect("the build stashed signals");
    (harness, signals)
}

/// The middle of the first cell of `node`, where these tests press rows.
fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: node.bounds.x0 + 40.0,
        y: (node.bounds.y0 + node.bounds.y1) / 2.0,
    }
}

/// The bounds of the header cell of the first column.
fn first_header(harness: &mut Harness) -> Rect {
    with_role(harness, Role::ColumnHeader)[0].bounds
}

/// The projected nodes with `role`, in paint order.
fn with_role(harness: &mut Harness, role: Role) -> Vec<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter(|node| node.role == role)
        .collect()
}

/// The first cell's text of `row`, which names the row in these tests.
fn cell_text(nodes: &[SemanticsNode], row: &SemanticsNode) -> String {
    row.children
        .iter()
        .find_map(|&child| {
            let cell = nodes.iter().find(|node| node.id == child)?;
            cell.children.iter().find_map(|&grandchild| {
                nodes
                    .iter()
                    .find(|node| node.id == grandchild)
                    .and_then(|label| label.value.clone())
            })
        })
        .unwrap_or_else(|| panic!("the row of {:?} has a cell", row.id))
}

/// The names of the rows the table reports as selected, in the order they
/// are painted.
fn selected_names(harness: &mut Harness) -> Vec<String> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .filter(|node| node.role == Role::Row && node.selected == Some(true))
        .map(|row| cell_text(&nodes, row))
        .collect()
}

/// The name of the row that holds the keyboard focus.
fn focused_row_name(harness: &mut Harness) -> Option<String> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .find(|node| node.focused && node.role == Role::Row)
        .map(|row| cell_text(&nodes, row))
}

/// The names of the mounted rows, in the order they are painted.
fn row_names(harness: &mut Harness) -> Vec<String> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .filter(|node| node.role == Role::Row)
        .map(|row| cell_text(&nodes, row))
        .collect()
}

/// Clears the focus, as a press on the strip below the table does.
fn clear_focus(harness: &mut Harness) {
    harness.click(Point { x: 290.0, y: 210.0 });
    harness.frame();
}

/// Injects `modifiers` into the window, so the next button events carry
/// them.
fn hold(harness: &mut Harness, modifiers: Modifiers) {
    harness.event(InputEvent::ModifiersChanged(modifiers));
}

/// The Shift key alone.
fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

/// The Control key alone.
fn control() -> Modifiers {
    Modifiers {
        control: true,
        ..Modifiers::default()
    }
}

/// Lets the modifiers go.
fn release_modifiers(harness: &mut Harness) {
    hold(harness, Modifiers::default());
}

#[test]
fn a_drag_on_the_header_edge_resizes_the_column_and_clamps_at_the_minimum() {
    let (mut harness, signals) = interactions_window();
    let header = first_header(&mut harness);
    let edge = Point {
        x: header.x1 - 2.0,
        y: (header.y0 + header.y1) / 2.0,
    };

    harness.button_down(edge);
    harness.move_to(Point {
        x: edge.x + 120.0,
        y: edge.y,
    });
    harness.button_up(Point {
        x: edge.x + 120.0,
        y: edge.y,
    });
    harness.frame();

    assert_eq!(
        signals.widths.read(|all| all[0]),
        300.0,
        "the first column took the drag"
    );
    let headers = with_role(&mut harness, Role::ColumnHeader);
    assert!(
        (headers[0].bounds.x1 - headers[0].bounds.x0 - 300.0).abs() < 0.5,
        "the header cell lays out at the new width"
    );
    assert_eq!(
        headers[1].bounds.x0, headers[0].bounds.x1,
        "the second column starts where the first ends"
    );

    // Narrowing stops at the minimum, and the drag over the header sorts
    // nothing.
    let header = first_header(&mut harness);
    let edge = Point {
        x: header.x1 - 2.0,
        y: (header.y0 + header.y1) / 2.0,
    };
    harness.button_down(edge);
    harness.move_to(Point {
        x: edge.x - 220.0,
        y: edge.y,
    });
    harness.button_up(Point {
        x: edge.x - 220.0,
        y: edge.y,
    });
    harness.frame();

    assert_eq!(
        signals.widths.read(|all| all[0]),
        80.0,
        "the first column stops at its minimum"
    );
    assert!(
        with_role(&mut harness, Role::ColumnHeader)
            .iter()
            .all(|header| header.sort_direction.is_none()),
        "a resize drag sorts nothing"
    );
}

#[test]
fn a_press_on_a_sortable_header_cycles_the_sort_and_reports_it() {
    let (mut harness, signals) = interactions_window();
    let header = first_header(&mut harness);
    let at = Point {
        x: (header.x0 + header.x1) / 2.0,
        y: (header.y0 + header.y1) / 2.0,
    };

    harness.click(at);
    harness.frame();
    assert_eq!(
        signals.sort.read(|sorted| *sorted),
        Some((0, SortDirection::Ascending)),
        "the first press sorts ascending"
    );
    let headers = with_role(&mut harness, Role::ColumnHeader);
    assert_eq!(headers[0].sort_direction, Some(SortDirection::Ascending));
    assert_eq!(headers[1].sort_direction, None);

    harness.click(at);
    harness.frame();
    assert_eq!(
        signals.sort.read(|sorted| *sorted),
        Some((0, SortDirection::Descending)),
        "the second press sorts descending"
    );

    harness.click(at);
    harness.frame();
    assert_eq!(
        signals.sort.read(|sorted| *sorted),
        None,
        "the third press clears the sort"
    );
    let headers = with_role(&mut harness, Role::ColumnHeader);
    assert_eq!(headers[0].sort_direction, None);

    // A press on a column that cannot sort does nothing.
    let headers = with_role(&mut harness, Role::ColumnHeader);
    let second = middle(&headers[1]);
    harness.click(second);
    harness.frame();
    assert!(
        with_role(&mut harness, Role::ColumnHeader)
            .iter()
            .all(|header| header.sort_direction.is_none()),
        "the second column never sorts"
    );
}

#[test]
fn a_press_selects_and_the_modifiers_extend_the_selection() {
    let (mut harness, signals) = interactions_window();

    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[0]));
    harness.frame();
    assert_eq!(
        signals.selection.read(|set| set.clone()),
        HashSet::from([0u64]),
        "a press selects its row"
    );

    hold(&mut harness, control());
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[2]));
    release_modifiers(&mut harness);
    harness.frame();
    assert_eq!(
        signals.selection.read(|set| set.clone()),
        HashSet::from([0u64, 2]),
        "Ctrl adds the pressed row"
    );

    hold(&mut harness, shift());
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[5]));
    release_modifiers(&mut harness);
    harness.frame();
    let selected = signals.selection.read(|set| set.clone());
    assert_eq!(
        selected,
        HashSet::from([0u64, 2, 3, 4, 5]),
        "Shift extends from the anchor through the pressed row"
    );
    assert_eq!(
        selected_names(&mut harness).len(),
        5,
        "the projection hears every row"
    );
}

#[test]
fn a_press_replaces_the_selection_when_the_view_holds_one_row() {
    let (mut harness, signals) = single_window();

    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[0]));
    harness.frame();
    assert_eq!(
        signals.selection.read(|set| set.clone()),
        HashSet::from([0u64]),
        "the first press selects"
    );

    hold(&mut harness, control());
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[2]));
    release_modifiers(&mut harness);
    harness.frame();
    assert_eq!(
        signals.selection.read(|set| set.clone()),
        HashSet::from([2u64]),
        "a single view replaces the selection whatever Ctrl holds"
    );
}

#[test]
fn the_keyboard_moves_the_focus_and_extends_the_selection() {
    let (mut harness, signals) = interactions_window();
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[0]));
    harness.frame();
    assert_eq!(
        focused_row_name(&mut harness).as_deref(),
        Some("Item 0"),
        "a press focuses its row"
    );

    harness.press_key(Key::ArrowDown);
    harness.settle();
    assert_eq!(focused_row_name(&mut harness).as_deref(), Some("Item 1"));

    harness.press_key_with(Key::ArrowDown, shift());
    harness.settle();
    assert_eq!(
        signals.selection.read(|set| set.len()),
        3,
        "Shift extends the selection through the row the focus moved to"
    );
    assert_eq!(focused_row_name(&mut harness).as_deref(), Some("Item 2"));

    harness.press_key(Key::Space);
    harness.settle();
    assert_eq!(
        signals.selection.read(|set| set.len()),
        2,
        "Space toggles the focused row, which the extension held"
    );

    harness.press_key(Key::Home);
    harness.settle();
    assert_eq!(focused_row_name(&mut harness).as_deref(), Some("Item 0"));

    harness.press_key(Key::PageDown);
    harness.settle();
    let page = focused_row_name(&mut harness)
        .and_then(|name| name.trim_start_matches("Item ").parse::<usize>().ok())
        .expect("the focus sits on a numbered row");
    assert!(
        page > 3,
        "a page is more than the rows the shift reached, {page}"
    );
    assert!(page < 50, "a page is the viewport, not the whole table");
}

#[test]
fn the_end_key_reveals_a_row_far_down_the_table() {
    let (mut harness, signals) = interactions_window();
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[0]));
    harness.frame();

    harness.press_key(Key::End);
    harness.settle();

    assert_eq!(
        focused_row_name(&mut harness).as_deref(),
        Some("Item 99999"),
        "the last row holds the focus"
    );
    assert_eq!(
        signals.focused.read(|held| held.clone()),
        Some(99_999u64),
        "the view writes the focused key"
    );
    let names = row_names(&mut harness);
    assert!(
        names.len() <= 16,
        "the end mounts a window of rows, {}",
        names.len()
    );
    assert_eq!(
        names.last().map(String::as_str),
        Some("Item 99999"),
        "the window ends at the last row"
    );

    harness.press_key(Key::Home);
    harness.settle();
    assert_eq!(
        focused_row_name(&mut harness).as_deref(),
        Some("Item 0"),
        "Home comes back to the first row"
    );
}

#[test]
fn the_rows_take_one_tab_stop() {
    let (mut harness, _signals) = interactions_window();

    harness.press_key(Key::Tab);
    harness.settle();
    assert_eq!(
        focused_row_name(&mut harness).as_deref(),
        Some("Item 0"),
        "Tab enters the table on its first row"
    );

    // With a selection, Tab re-enters at the selected row, not at the
    // first, and never on a second row of the table.
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[5]));
    harness.frame();
    clear_focus(&mut harness);
    assert_eq!(focused_row_name(&mut harness), None, "the focus cleared");
    harness.press_key(Key::Tab);
    harness.settle();
    assert_eq!(
        focused_row_name(&mut harness).as_deref(),
        Some("Item 5"),
        "Tab re-enters at the selected row"
    );
}

#[test]
fn a_sorted_table_with_a_selected_row_in_the_light_theme() {
    let (mut harness, _signals) = interactions_window();
    let header = first_header(&mut harness);
    harness.click(Point {
        x: (header.x0 + header.x1) / 2.0,
        y: (header.y0 + header.y1) / 2.0,
    });
    harness.frame();
    let rows = with_role(&mut harness, Role::Row);
    harness.click(middle(&rows[1]));
    harness.frame();
    harness.app().set_theme(Theme::light());
    harness.settle();
    harness.frame();
    insta::assert_binary_snapshot!(".png", harness.png());
}
