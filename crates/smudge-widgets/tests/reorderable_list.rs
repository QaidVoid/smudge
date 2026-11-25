//! Whole-frame behavior of the reorderable list, driven through the headless
//! harness: the drag that moves an item once, the indicator line that tracks
//! where a drag would land and hides when the drag cancels, the Alt+key
//! moves with one announcement each, the edge keys that move nothing, and
//! the move that leaves every other row mounted.
//!
//! The rows are 40 logical pixels tall and the four of them together fill
//! the top of a 480 tall window, so a release in the strip below them lands
//! on no row. The probes sample at x 100, well left of the drag preview,
//! which the window draws from the pointer to the right edge.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::geometry::Point;
use smudge_core::input::{Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Politeness, Role, SemanticsNode};
use smudge_core::style::px;
use smudge_core::tree::WidgetId;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};

use smudge_widgets::reorderable_list;

/// The height of the list and of its window, in logical pixels.
const HEIGHT: f32 = 480.0;

/// The width of the window, in logical pixels.
const WIDTH: f32 = 640.0;

/// The height of every row: four of them take the top 160 pixels of the
/// window, and the strip below stays free of rows.
const ROW_HEIGHT: f32 = 40.0;

/// The accent color the light theme resolves the token to, which is the
/// color of the drop indicator.
const ACCENT: [u8; 4] = [0, 95, 204, 255];

/// The surface color a row paints behind its content.
const SURFACE: [u8; 4] = [255, 255, 255, 255];

/// The center column of the window, where these tests press and release.
const CENTER_X: f32 = 320.0;

/// The middle of row `row`, whose top sits at `row * 40`.
fn row_center(row: usize) -> Point {
    Point {
        x: CENTER_X,
        y: row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0,
    }
}

/// A settled window with a list of the keys 0 to 3, with the items signal
/// the tests reorder and the count of row builds the tests compare.
struct Scene {
    harness: Harness,
    items: RwSignal<Vec<u64>>,
    mounts: Rc<Cell<u32>>,
}

/// Builds the scene. The build stashes the items signal, because the
/// closure that owns it runs inside the harness.
fn scene() -> Scene {
    TextSystem::bundled_only().install();
    let items: RwSignal<Vec<u64>> = RwSignal::new(vec![0, 1, 2, 3]);
    let mounts = Rc::new(Cell::new(0));
    let stash: Rc<RefCell<Option<RwSignal<Vec<u64>>>>> = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(WIDTH, HEIGHT, 1.0, {
        let items = items.clone();
        let mounts = Rc::clone(&mounts);
        let stash = Rc::clone(&stash);
        move || {
            let mounts = Rc::clone(&mounts);
            let list = reorderable_list(
                items.clone(),
                |item| *item,
                ROW_HEIGHT,
                move |item| {
                    mounts.set(mounts.get() + 1);
                    panel()
                        .height(px(ROW_HEIGHT))
                        .child(label(format!("Key {item}")))
                },
            )
            .height(px(HEIGHT));
            *stash.borrow_mut() = Some(items.clone());
            panel().column().child(list)
        }
    });
    harness.settle();
    let items = stash
        .borrow()
        .clone()
        .expect("the build stashed its items signal");
    Scene {
        harness,
        items,
        mounts,
    }
}

/// The announcements the app queued, oldest first, as message and
/// politeness, and takes off the queue.
fn announcements(harness: &mut Harness) -> Vec<(String, Politeness)> {
    harness
        .app()
        .take_a11y_changes()
        .announcements
        .into_iter()
        .map(|announcement| (announcement.message, announcement.politeness))
        .collect()
}

/// The text of the node that holds the focus, found by walking down from it
/// to the first text it projects.
fn focused_text(harness: &mut Harness) -> Option<String> {
    let nodes = harness.app().semantics_nodes();
    let focused = nodes.iter().find(|node| node.focused)?;
    text_below(&nodes, focused.id)
}

/// The first text the subtree of `id` projects, depth first.
fn text_below(nodes: &[SemanticsNode], id: WidgetId) -> Option<String> {
    let node = nodes.iter().find(|node| node.id == id)?;
    node.value
        .clone()
        .or_else(|| node.label.clone())
        .or_else(|| {
            node.children
                .iter()
                .find_map(|&child| text_below(nodes, child))
        })
}

/// The accessible place of every mounted list item, as text and position.
fn item_places(harness: &mut Harness) -> Vec<(String, usize)> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .filter(|node| node.role == Role::ListItem)
        .filter_map(|item| {
            let text = text_below(&nodes, item.id)?;
            Some((text, item.position?))
        })
        .collect()
}

/// The screen bands of the drop indicator lines, from the layout dump.
///
/// The indicator is the only panel two pixels tall, and it hangs out of
/// flow, so its rectangle says where the line paints without sampling
/// pixels the faint drag preview covers.
fn indicator_lines(harness: &Harness) -> Vec<(f32, f32)> {
    harness
        .layout_dump()
        .lines()
        .filter_map(|line| {
            let start = line.find("Panel [")?;
            let numbers = &line[start + "Panel [".len()..line.len() - 1];
            let values: Vec<f32> = numbers
                .split(", ")
                .filter_map(|value| value.parse().ok())
                .collect();
            (values[3] - values[1] == 2.0).then_some((values[1], values[3]))
        })
        .collect()
}

#[test]
fn a_drag_to_the_lower_half_of_another_row_moves_the_item_once() {
    let Scene {
        mut harness,
        items,
        mounts,
    } = scene();
    let before = mounts.get();

    harness.button_down(row_center(0));
    harness.move_to(Point {
        x: CENTER_X,
        y: 70.0,
    });
    harness.settle();
    assert_eq!(
        indicator_lines(&harness),
        [(78.0, 80.0)],
        "the hover marks the line after row 1"
    );
    assert_eq!(
        items.get(),
        vec![0, 1, 2, 3],
        "the hover moves nothing; the release does"
    );

    harness.button_up(Point {
        x: CENTER_X,
        y: 70.0,
    });
    harness.settle();
    assert_eq!(items.get(), vec![1, 0, 2, 3]);
    assert_eq!(
        announcements(&mut harness),
        [(String::from("Moved to position 2 of 4"), Politeness::Polite)],
        "one announcement per move"
    );
    assert_eq!(
        harness.pixel(100, 79),
        Some(SURFACE),
        "the line hid after the drop"
    );
    assert_eq!(mounts.get(), before, "no row rebuilt for the move");
}

#[test]
fn the_indicator_sits_where_the_drag_would_land() {
    let Scene {
        mut harness,
        items,
        mounts: _,
    } = scene();

    harness.button_down(row_center(0));
    harness.move_to(Point {
        x: CENTER_X,
        y: 85.0,
    });
    harness.settle();
    assert_eq!(
        harness.pixel(100, 81),
        Some(ACCENT),
        "the upper half of row 2 marks its top edge"
    );
    assert_eq!(harness.pixel(100, 79), Some(SURFACE));
    assert_eq!(harness.pixel(100, 83), Some(SURFACE));
    assert_eq!(items.get(), vec![0, 1, 2, 3], "hovering moves no item");

    harness.move_to(Point {
        x: CENTER_X,
        y: 110.0,
    });
    harness.settle();
    assert_eq!(
        indicator_lines(&harness),
        [(118.0, 120.0)],
        "the lower half of row 2 marks its bottom edge"
    );

    harness.press_key(Key::Escape);
    harness.settle();
    assert!(indicator_lines(&harness).is_empty(), "Escape hid the line");
    assert_eq!(items.get(), vec![0, 1, 2, 3], "Escape moved no item");
    assert!(
        announcements(&mut harness).is_empty(),
        "a cancelled drag announces nothing"
    );
}

#[test]
fn alt_down_moves_the_focused_row_one_place_and_announces_once() {
    let Scene {
        mut harness,
        items,
        mounts: _,
    } = scene();

    harness.click(row_center(1));
    harness.settle();
    assert_eq!(focused_text(&mut harness).as_deref(), Some("Key 1"));
    let before = announcements(&mut harness);

    harness.press_key_with(
        Key::ArrowDown,
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
    );
    harness.settle();
    assert_eq!(items.get(), vec![0, 2, 1, 3]);
    assert_eq!(
        announcements(&mut harness),
        [(String::from("Moved to position 3 of 4"), Politeness::Polite)],
        "one announcement per move, and the plain click announced none of {:?}",
        before
    );
    assert_eq!(
        focused_text(&mut harness).as_deref(),
        Some("Key 1"),
        "the focus stays on the moved row"
    );
}

#[test]
fn alt_up_moves_the_focused_row_one_place_up() {
    let Scene {
        mut harness, items, ..
    } = scene();
    harness.click(row_center(2));
    harness.settle();
    let _ = announcements(&mut harness);

    harness.press_key_with(
        Key::ArrowUp,
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
    );
    harness.settle();
    assert_eq!(items.get(), vec![0, 2, 1, 3]);
    assert_eq!(
        announcements(&mut harness),
        [(String::from("Moved to position 2 of 4"), Politeness::Polite)]
    );
    assert_eq!(focused_text(&mut harness).as_deref(), Some("Key 2"));
}

#[test]
fn alt_home_and_alt_end_move_the_focused_row_to_the_ends() {
    let Scene {
        mut harness, items, ..
    } = scene();
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };

    harness.click(row_center(2));
    harness.settle();
    let _ = announcements(&mut harness);
    harness.press_key_with(Key::Home, alt);
    harness.settle();
    assert_eq!(items.get(), vec![2, 0, 1, 3]);
    assert_eq!(
        announcements(&mut harness),
        [(String::from("Moved to position 1 of 4"), Politeness::Polite)]
    );
    assert_eq!(focused_text(&mut harness).as_deref(), Some("Key 2"));

    harness.press_key_with(Key::End, alt);
    harness.settle();
    assert_eq!(items.get(), vec![0, 1, 3, 2]);
    assert_eq!(
        announcements(&mut harness),
        [(String::from("Moved to position 4 of 4"), Politeness::Polite)]
    );
    assert_eq!(focused_text(&mut harness).as_deref(), Some("Key 2"));
}

#[test]
fn a_release_over_the_dragged_row_moves_nothing() {
    let Scene {
        mut harness, items, ..
    } = scene();

    harness.button_down(row_center(1));
    harness.move_to(Point {
        x: CENTER_X,
        y: 70.0,
    });
    harness.settle();
    assert!(
        indicator_lines(&harness).is_empty(),
        "hovering the dragged row marks no landing"
    );
    harness.button_up(Point {
        x: CENTER_X,
        y: 70.0,
    });
    harness.settle();
    assert_eq!(items.get(), vec![0, 1, 2, 3]);
    assert!(
        announcements(&mut harness).is_empty(),
        "a release over the dragged row announces nothing"
    );
}

#[test]
fn a_drag_to_the_upper_half_of_an_earlier_row_moves_the_item_up() {
    let Scene {
        mut harness, items, ..
    } = scene();

    harness.button_down(row_center(3));
    harness.move_to(Point {
        x: CENTER_X,
        y: 45.0,
    });
    harness.settle();
    assert_eq!(
        indicator_lines(&harness),
        [(40.0, 42.0)],
        "the upper half of row 1 marks its top edge"
    );
    harness.button_up(Point {
        x: CENTER_X,
        y: 45.0,
    });
    harness.settle();
    assert_eq!(items.get(), vec![0, 3, 1, 2]);
    assert_eq!(
        announcements(&mut harness),
        [(String::from("Moved to position 2 of 4"), Politeness::Polite)]
    );
}

#[test]
fn a_move_at_the_edge_of_the_list_moves_nothing_and_announces_nothing() {
    let Scene {
        mut harness, items, ..
    } = scene();
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };

    harness.click(row_center(0));
    harness.settle();
    let _ = announcements(&mut harness);

    harness.press_key_with(Key::ArrowUp, alt);
    harness.settle();
    harness.press_key_with(Key::Home, alt);
    harness.settle();
    assert_eq!(items.get(), vec![0, 1, 2, 3], "the edges held the row");
    assert!(
        announcements(&mut harness).is_empty(),
        "a move that cannot happen announces nothing"
    );

    let Scene {
        mut harness, items, ..
    } = scene();
    harness.click(row_center(3));
    harness.settle();
    let _ = announcements(&mut harness);
    harness.press_key_with(Key::ArrowDown, alt);
    harness.settle();
    harness.press_key_with(Key::End, alt);
    harness.settle();
    assert_eq!(items.get(), vec![0, 1, 2, 3], "the far edge held the row");
    assert!(
        announcements(&mut harness).is_empty(),
        "a move that cannot happen announces nothing"
    );
}

#[test]
fn a_move_keeps_the_other_rows_mounted_and_updates_their_places() {
    let Scene {
        mut harness,
        items,
        mounts,
    } = scene();
    harness.settle();
    assert_eq!(mounts.get(), 4, "the four rows mounted once");
    let places = item_places(&mut harness);
    assert_eq!(
        places,
        vec![
            (String::from("Key 0"), 0),
            (String::from("Key 1"), 1),
            (String::from("Key 2"), 2),
            (String::from("Key 3"), 3),
        ]
    );

    harness.click(row_center(0));
    harness.settle();
    let _ = announcements(&mut harness);
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::ArrowDown, alt);
    harness.settle();
    assert_eq!(items.get(), vec![1, 0, 2, 3]);
    assert_eq!(
        item_places(&mut harness),
        vec![
            (String::from("Key 1"), 0),
            (String::from("Key 0"), 1),
            (String::from("Key 2"), 2),
            (String::from("Key 3"), 3),
        ],
        "every row reports its new place, in the order they paint"
    );
    assert_eq!(
        mounts.get(),
        4,
        "the move rebuilt no row: the keys kept their rows"
    );
}

#[test]
fn a_release_over_nothing_moves_nothing() {
    let Scene {
        mut harness,
        items,
        mounts,
    } = scene();
    let before = mounts.get();

    harness.button_down(row_center(0));
    harness.move_to(Point {
        x: CENTER_X,
        y: 70.0,
    });
    harness.settle();
    assert_eq!(
        indicator_lines(&harness),
        [(78.0, 80.0)],
        "the line lit while the drag was over row 1"
    );
    harness.move_to(Point {
        x: CENTER_X,
        y: 300.0,
    });
    harness.settle();
    assert!(
        indicator_lines(&harness).is_empty(),
        "the strip below the rows cleared the lit line"
    );
    harness.button_up(Point {
        x: CENTER_X,
        y: 300.0,
    });
    harness.settle();
    assert_eq!(items.get(), vec![0, 1, 2, 3]);
    assert!(
        announcements(&mut harness).is_empty(),
        "a release over no row announces nothing"
    );
    assert_eq!(mounts.get(), before, "the refused drag rebuilt no row");
}

#[test]
fn a_drag_from_one_list_never_moves_the_rows_of_another() {
    TextSystem::bundled_only().install();
    let first: RwSignal<Vec<u64>> = RwSignal::new(vec![0, 1, 2]);
    let second: RwSignal<Vec<u64>> = RwSignal::new(vec![0, 1, 2]);
    let stash = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(WIDTH, HEIGHT, 1.0, {
        let first = first.clone();
        let second = second.clone();
        let stash = Rc::clone(&stash);
        move || {
            let top = reorderable_list(
                first.clone(),
                |item| *item,
                ROW_HEIGHT,
                |item| {
                    panel()
                        .height(px(ROW_HEIGHT))
                        .child(label(format!("Key {item}")))
                },
            )
            .height(px(ROW_HEIGHT * 3.0));
            let bottom = reorderable_list(
                second.clone(),
                |item| *item,
                ROW_HEIGHT,
                |item| {
                    panel()
                        .height(px(ROW_HEIGHT))
                        .child(label(format!("Key {item}")))
                },
            )
            .height(px(ROW_HEIGHT * 3.0));
            *stash.borrow_mut() = Some((first.clone(), second.clone()));
            panel().column().child(top).child(bottom)
        }
    });
    harness.settle();
    let (first, second) = stash.borrow().clone().expect("the build stashed both");

    // The top list's row 0 drags over the bottom list's row 1, whose key
    // differs. The lists share a key space on purpose: the top list holds
    // three rows of 40, so the bottom list's row 1 spans y 160..200 and its
    // center is y 180, over which a drag of the top list's row 0 would land
    // if the lists accepted each other's payloads.
    harness.button_down(Point {
        x: CENTER_X,
        y: 20.0,
    });
    harness.move_to(Point {
        x: CENTER_X,
        y: 180.0,
    });
    harness.settle();
    assert!(
        indicator_lines(&harness).is_empty(),
        "the second list marks no landing for another list's drag"
    );
    harness.button_up(Point {
        x: CENTER_X,
        y: 180.0,
    });
    harness.settle();
    assert_eq!(first.get(), vec![0, 1, 2], "the refused drag moved nothing");
    assert_eq!(
        second.get(),
        vec![0, 1, 2],
        "and moved nothing in the other list"
    );
    assert!(
        announcements(&mut harness).is_empty(),
        "a refused drop announces nothing"
    );
}

#[test]
fn alt_end_brings_a_moved_row_far_below_back_into_the_view_with_its_focus() {
    TextSystem::bundled_only().install();
    let items: RwSignal<Vec<u64>> = RwSignal::new((0..200).collect());
    let stash: Rc<RefCell<Option<RwSignal<Vec<u64>>>>> = Rc::new(RefCell::new(None));
    let mut harness = Harness::new(WIDTH, HEIGHT, 1.0, {
        let items = items.clone();
        let stash = Rc::clone(&stash);
        move || {
            let list = reorderable_list(
                items.clone(),
                |item| *item,
                ROW_HEIGHT,
                |item| {
                    panel()
                        .height(px(ROW_HEIGHT))
                        .child(label(format!("Key {item}")))
                },
            )
            .height(px(HEIGHT));
            *stash.borrow_mut() = Some(items.clone());
            panel().column().child(list)
        }
    });
    harness.settle();
    let items = stash.borrow().clone().expect("the build stashed its items");

    harness.click(Point {
        x: CENTER_X,
        y: 60.0,
    });
    harness.settle();
    assert_eq!(focused_text(&mut harness).as_deref(), Some("Key 1"));
    let _ = announcements(&mut harness);

    harness.press_key_with(
        Key::End,
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
    );
    harness.settle();
    assert_eq!(items.get().last(), Some(&1), "the row moved to the end");
    assert_eq!(
        announcements(&mut harness),
        [(
            String::from("Moved to position 200 of 200"),
            Politeness::Polite
        )]
    );
    assert_eq!(
        focused_text(&mut harness).as_deref(),
        Some("Key 1"),
        "the moved row kept the focus, mounted in the view that followed it"
    );
    assert!(
        item_places(&mut harness).contains(&(String::from("Key 1"), 199)),
        "the moved row is mounted at the end of the list"
    );
}
