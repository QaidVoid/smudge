//! Behavior tests for keyed dynamic children.
//!
//! Every test drives the same frame pipeline a window does, through the
//! harness, and checks what the surface shows against a harness that mounts
//! the expected list plainly.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{Element, for_each};
use smudge_core::geometry::{Color, PixelRect, Point};
use smudge_core::reactive::{WriteSignal, prelude::*};
use smudge_core::style::{Direction, Edges, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// The size of one row, in logical pixels.
const ROW: (f32, f32) = (40.0, 10.0);

/// The color the row of `key` paints.
fn row_color(key: u64) -> Color {
    match key {
        1 => Color::rgb(200, 30, 30),
        2 => Color::rgb(30, 200, 30),
        3 => Color::rgb(30, 30, 200),
        _ => Color::rgb(200, 200, 30),
    }
}

/// The element one row mounts for its key: a focusable, colored panel.
fn row(key: u64) -> Element<Panel> {
    panel()
        .width(px(ROW.0))
        .height(px(ROW.1))
        .background(row_color(key))
        .focusable()
}

/// A keyed list of rows under a root panel, with the write half of the list
/// signal and the count of row builds.
fn list_app(initial: Vec<u64>) -> (Harness, WriteSignal<Vec<u64>>, Rc<Cell<u32>>) {
    let (items, set_items) = signal_local(initial);
    let builds = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&builds);
    let harness = Harness::new(100.0, 80.0, 1.0, move || {
        panel().child(
            for_each(
                move || items.get(),
                |item| *item,
                move |key| {
                    counted.set(counted.get() + 1);
                    row(key)
                },
            )
            .direction(Direction::Column),
        )
    });
    (harness, set_items, builds)
}

/// The center of the row at `index`, in logical pixels.
fn row_center(index: usize) -> Point {
    Point {
        x: ROW.0 / 2.0,
        y: index as f32 * ROW.1 + ROW.1 / 2.0,
    }
}

/// Clicks the row at `index` and returns the id of the node it focused.
fn focused_row(harness: &mut Harness, index: usize) -> smudge_core::tree::WidgetId {
    harness.click(row_center(index));
    harness
        .app()
        .focused()
        .expect("the clicked row took the focus")
}

#[test]
fn appending_an_item_mounts_one_node_and_paints_only_it() {
    let (mut harness, set_items, builds) = list_app(vec![1, 2]);
    harness.frame();

    set_items.set(vec![1, 2, 3]);
    let frame = harness.frame();

    assert_eq!(
        frame.damage,
        vec![PixelRect {
            x: 0,
            y: 20,
            width: 40,
            height: 10,
        }],
        "the new row is the only thing the frame repaints"
    );
    let mut nodes = 0;
    harness.app().visit_nodes(|_, _, _| nodes += 1);
    assert_eq!(nodes, 5, "the root, the slot and three rows are mounted");
    assert_eq!(builds.get(), 3, "each key built its row once");
    harness.assert_matches_full_repaint();
}

#[test]
fn removing_the_middle_item_removes_only_that_node() {
    let (mut harness, set_items, builds) = list_app(vec![1, 2, 3]);
    harness.frame();

    let one = focused_row(&mut harness, 0);
    let two = focused_row(&mut harness, 1);
    let three = focused_row(&mut harness, 2);
    assert_ne!(one, two);
    assert_ne!(two, three);
    assert_eq!(builds.get(), 3);

    // Put the focus back on the row that is about to be removed.
    assert_eq!(focused_row(&mut harness, 1), two);

    set_items.set(vec![1, 3]);
    harness.frame();
    assert!(
        harness.app().focused().is_none(),
        "removing the focused row clears the focus"
    );

    // The surviving rows answer at their new positions with their old ids.
    assert_eq!(focused_row(&mut harness, 0), one);
    assert_eq!(focused_row(&mut harness, 1), three);
    assert_eq!(builds.get(), 3, "the rows that survived were not rebuilt");

    harness.assert_matches_full_repaint();
}

#[test]
fn reversing_the_list_keeps_all_ids_and_reorders_the_rows() {
    let (mut harness, set_items, builds) = list_app(vec![1, 2, 3]);
    harness.frame();

    let one = focused_row(&mut harness, 0);
    let two = focused_row(&mut harness, 1);
    let three = focused_row(&mut harness, 2);

    set_items.set(vec![3, 2, 1]);
    let frame = harness.frame();
    assert_eq!(
        frame.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 40,
            height: 30,
        }],
        "the rows restack, so the whole list repaints"
    );

    // The rows answer at their new positions with their old ids.
    assert_eq!(focused_row(&mut harness, 0), three);
    assert_eq!(focused_row(&mut harness, 1), two);
    assert_eq!(focused_row(&mut harness, 2), one);
    assert_eq!(builds.get(), 3, "no row was rebuilt");

    let mut expected = Harness::new(100.0, 80.0, 1.0, || {
        panel().child(
            panel()
                .direction(Direction::Column)
                .child(row(3))
                .child(row(2))
                .child(row(1)),
        )
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "duplicate key")]
fn duplicate_keys_trip_the_debug_assertion() {
    let (mut harness, _set_items, _builds) = list_app(vec![1, 1]);
    harness.frame();
}

/// The element one layer of an overlapping pair mounts for its key: an
/// absolutely positioned colored panel over the whole stack.
fn layer(key: u64) -> Element<Panel> {
    panel()
        .absolute()
        .inset(Edges::all(px(0.0)))
        .width(px(40.0))
        .height(px(20.0))
        .background(row_color(key))
}

#[test]
fn reordering_overlapping_children_swaps_which_one_is_on_top() {
    let (items, set_items) = signal_local(vec![1u64, 2]);
    let mut harness = Harness::new(100.0, 80.0, 1.0, move || {
        panel().child(for_each(move || items.get(), |item| *item, layer))
    });
    harness.frame();

    // Row two is the last child, so it covers row one.
    let mut stacked = Harness::new(100.0, 80.0, 1.0, || {
        panel().child(panel().child(layer(1)).child(layer(2)))
    });
    stacked.frame();
    harness.assert_matches(&stacked);

    set_items.set(vec![2, 1]);
    let frame = harness.frame();
    assert_eq!(
        frame.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 40,
            height: 20,
        }],
        "the overlap repaints although no rectangle changed"
    );

    // Row one paints last now, and the screen shows it.
    let mut swapped = Harness::new(100.0, 80.0, 1.0, || {
        panel().child(panel().child(layer(2)).child(layer(1)))
    });
    swapped.frame();
    harness.assert_matches(&swapped);
}
