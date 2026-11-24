//! Whole-frame behavior of the table, driven through the headless harness:
//! the rows a hundred thousand row table mounts at three offsets, the
//! header that moves sideways with the body without either repainting,
//! the cells that clip their content, and what a screen reader hears.

use std::rc::Rc;

use smudge_core::geometry::Point;
use smudge_core::input::InputEvent;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{LabelExt, TextSystem, label};

use smudge_widgets::{Column, table};

/// The rows of the test table.
const ROWS: u64 = 100_000;

/// Where the tests park the pointer, over the body of the table.
const POINTER: Point = Point { x: 100.0, y: 100.0 };

/// The table of `rows` in a window of 300 by 200, settled with the pointer
/// over its body. The columns are Name, 180 wide, and Size, 80 wide, so a
/// sideways scroll of 60 fits.
fn table_window(rows: RwSignal<Rc<[u64]>>, columns: usize) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let rows = rows.clone();
        panel().child(
            table(
                (0..columns)
                    .map(|index| {
                        Column::new(format!("Column {index}"), 180.0, 40.0, Align::Start, false)
                    })
                    .collect::<Vec<_>>(),
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
            .height(px(200.0))
            .width(px(300.0)),
        )
    });
    harness.settle();
    harness.move_to(POINTER);
    harness
}

/// Wheels the table by `across` and `down` logical pixels.
fn wheel(harness: &mut Harness, across: f32, down: f32) {
    harness.event(InputEvent::Wheel {
        delta: Point { x: across, y: down },
    });
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

/// The first projected node with `role`.
fn first_role(harness: &mut Harness, role: Role) -> SemanticsNode {
    with_role(harness, role)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("the window shows a {role:?}"))
}

/// The names of the mounted rows, in the order they are painted.
fn row_names(harness: &mut Harness) -> Vec<String> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .filter(|node| node.role == Role::Row)
        .map(|row| {
            row.children
                .iter()
                .find_map(|&child| {
                    let cell = nodes.iter().find(|node| node.id == child)?;
                    Some(
                        cell.children
                            .iter()
                            .filter_map(|&grandchild| {
                                nodes
                                    .iter()
                                    .find(|node| node.id == grandchild)
                                    .and_then(|label| label.value.clone())
                            })
                            .next(),
                    )
                })
                .flatten()
                .unwrap_or_else(|| panic!("the row of {:?} has a cell", row.id))
        })
        .collect()
}

#[test]
fn a_hundred_thousand_rows_mount_a_window_of_rows() {
    // The body is 172 logical pixels tall, seven rows of 28 fill it and the
    // row that begins at the body's bottom edge makes eight; the overscan
    // adds four on each side.
    const MOUNTED_BOUND: usize = 16;
    let rows = RwSignal::new(Rc::from((0..ROWS).collect::<Vec<_>>()));
    let mut harness = table_window(rows, 2);

    let names = row_names(&mut harness);
    assert!(
        names.len() <= MOUNTED_BOUND,
        "the top mounts {} rows, at most {MOUNTED_BOUND}",
        names.len()
    );
    assert_eq!(names.first().map(String::as_str), Some("Item 0"));

    wheel(&mut harness, 0.0, 28_000.0);
    harness.settle();
    let names = row_names(&mut harness);
    assert!(
        names.len() <= MOUNTED_BOUND,
        "the middle mounts {} rows, at most {MOUNTED_BOUND}",
        names.len()
    );
    assert!(
        names.iter().any(|name| name == "Item 1000"),
        "the thousandth row is on screen"
    );

    wheel(&mut harness, 0.0, 10_000_000.0);
    harness.settle();
    let names = row_names(&mut harness);
    assert!(
        names.len() <= MOUNTED_BOUND,
        "the bottom mounts {} rows, at most {MOUNTED_BOUND}",
        names.len()
    );
    assert_eq!(
        names.last().map(String::as_str),
        Some("Item 99999"),
        "the last row is on screen"
    );
}

#[test]
fn a_sideways_scroll_moves_the_header_and_the_body_together() {
    let rows = RwSignal::new(Rc::from((0..ROWS).collect::<Vec<_>>()));
    let mut harness = table_window(rows, 2);

    let headers = with_role(&mut harness, Role::ColumnHeader);
    let cells = with_role(&mut harness, Role::Cell);
    let header_start = headers[0].bounds.x0;
    let cell_start = cells[0].bounds.x0;
    assert!((header_start - cell_start).abs() < 0.5, "they line up");

    wheel(&mut harness, 60.0, 0.0);
    let report = harness.frame();

    let headers = with_role(&mut harness, Role::ColumnHeader);
    let cells = with_role(&mut harness, Role::Cell);
    assert!(
        (headers[0].bounds.x0 - (header_start - 60.0)).abs() < 0.5,
        "the header moved left by the scroll"
    );
    assert!(
        (cells[0].bounds.x0 - (cell_start - 60.0)).abs() < 0.5,
        "the body moved left by the scroll"
    );
    assert_eq!(
        report.nodes_painted, 0,
        "neither's content repaints; both move under their offsets"
    );
    assert!(!report.damage.is_empty());
    harness.assert_matches_full_repaint();

    // The header never leaves the top of the table, whatever the body does.
    wheel(&mut harness, 0.0, 2_800.0);
    harness.settle();
    let headers = with_role(&mut harness, Role::ColumnHeader);
    assert!(
        (headers[0].bounds.y0 - 0.0).abs() < 0.5,
        "the header stays at the top"
    );
    let nodes = harness.app().semantics_nodes();
    let first_visible = nodes
        .iter()
        .find(|node| node.role == Role::Row && node.bounds.y0 >= 28.0 - 0.5)
        .and_then(|row| {
            let cell = row
                .children
                .first()
                .and_then(|&child| nodes.iter().find(|node| node.id == child))?;
            cell.children.iter().find_map(|&grandchild| {
                nodes
                    .iter()
                    .find(|node| node.id == grandchild)
                    .and_then(|label| label.value.clone())
            })
        });
    assert_eq!(
        first_visible.as_deref(),
        Some("Item 100"),
        "the body scrolled under it"
    );
}

#[test]
fn a_wide_cell_clips_to_its_column() {
    TextSystem::bundled_only().install();
    let long: Rc<[u64]> = Rc::from([0]);
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let long = long.clone();
        panel().child(
            table(
                [
                    Column::new("Name", 120.0, 40.0, Align::Start, false),
                    Column::new(
                        "A header whose title is far wider than its column",
                        120.0,
                        40.0,
                        Align::Start,
                        false,
                    ),
                ],
                move || long.clone(),
                |row| *row,
                |row, column_index| {
                    let text = if column_index == 0 {
                        format!("A cell whose text runs far past the column it sits in, row {row}")
                    } else {
                        format!("{row}")
                    };
                    label(text).max_lines(Some(1))
                },
            )
            .height(px(200.0))
            .width(px(300.0)),
        )
    });
    harness.settle();

    let cells = with_role(&mut harness, Role::Cell);
    let headers = with_role(&mut harness, Role::ColumnHeader);
    for cell in &cells {
        assert!(
            cell.bounds.x1 - cell.bounds.x0 <= 120.5,
            "the cell lays out at its column's width"
        );
        assert!(
            cell.bounds.y1 - cell.bounds.y0 <= 28.5,
            "the cell keeps one line's height"
        );
    }
    for header in &headers {
        assert!(
            header.bounds.x1 - header.bounds.x0 <= 120.5,
            "the header cell lays out at its column's width"
        );
    }

    // And the table under it stays as tall as the header and the body say,
    // whatever the text inside it runs to.
    let table = first_role(&mut harness, Role::Table);
    assert!((table.bounds.y1 - table.bounds.y0 - 200.0).abs() < 0.5);
}

#[test]
fn the_projection_reports_counts_and_indexes() {
    let rows = RwSignal::new(Rc::from((0..ROWS).collect::<Vec<_>>()));
    let mut harness = table_window(rows, 2);

    let table = first_role(&mut harness, Role::Table);
    assert_eq!(table.row_count, Some(100_000));
    assert_eq!(table.column_count, Some(2));

    let headers = with_role(&mut harness, Role::ColumnHeader);
    assert_eq!(headers.len(), 2, "both headers are mounted");
    for (index, header) in headers.iter().enumerate() {
        assert_eq!(header.column_index, Some(index), "header {index}");
    }

    let rows = with_role(&mut harness, Role::Row);
    assert!(
        rows.iter().all(|row| row.row_index.is_some()),
        "every mounted row reports its index"
    );
    let first_row_index = rows[0].row_index.unwrap();
    let cells = with_role(&mut harness, Role::Cell);
    assert_eq!(cells.len(), rows.len() * 2, "two cells per mounted row");
    for (at, cell) in cells.iter().enumerate() {
        assert_eq!(
            cell.row_index,
            Some(first_row_index + at / 2),
            "the cell names its row"
        );
        assert_eq!(cell.column_index, Some(at % 2), "the cell names its column");
    }
}
