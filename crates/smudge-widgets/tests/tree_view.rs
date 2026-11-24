//! Whole-frame behavior of the tree view, driven through the headless
//! harness: picking and folding by press and by keys, the depth-first row
//! order, the rows a ten thousand node tree keeps mounted, the branches a
//! closed node never asks for, what a screen reader hears, and the one Tab
//! stop the tree takes.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};

use smudge_widgets::tree_view;

/// The signals a tree's build stashed for the test to drive and read.
#[derive(Default)]
struct Handles {
    expanded: Option<RwSignal<HashSet<u64>>>,
    selected: Option<RwSignal<Option<u64>>>,
}

/// Where a build leaves the tree's signals.
type Stash = Rc<RefCell<Handles>>;

/// One node of the test tree.
#[derive(Clone)]
struct Node {
    id: u64,
    name: &'static str,
    children: Vec<Node>,
}

/// A small tree: two roots, one of them two deep.
fn model() -> Rc<[Node]> {
    Rc::from(vec![
        Node {
            id: 1,
            name: "Alpha",
            children: vec![
                Node {
                    id: 11,
                    name: "Alpha one",
                    children: vec![Node {
                        id: 111,
                        name: "Leaf",
                        children: Vec::new(),
                    }],
                },
                Node {
                    id: 12,
                    name: "Alpha two",
                    children: Vec::new(),
                },
            ],
        },
        Node {
            id: 2,
            name: "Beta",
            children: vec![Node {
                id: 21,
                name: "Beta one",
                children: Vec::new(),
            }],
        },
    ])
}

/// A window of 300 by 220 showing the tree of `model`, 200 pixels tall,
/// with its signals stashed and every ask for children counted in `asks`.
fn window(stash: &Stash, asks: Rc<Cell<u32>>, model: Rc<[Node]>) -> Harness {
    TextSystem::bundled_only().install();
    let stash = Rc::clone(stash);
    let mut harness = Harness::new(300.0, 220.0, 1.0, move || {
        let model = Rc::clone(&model);
        let asks = Rc::clone(&asks);
        let view = tree_view(
            move || Rc::clone(&model),
            move |node| {
                asks.set(asks.get() + 1);
                Rc::from(node.children.as_slice())
            },
            |node| node.id,
            24.0,
            |node, _depth| label(node.name),
        )
        .height(px(200.0));
        let mut handles = stash.borrow_mut();
        handles.expanded = Some(view.expanded());
        handles.selected = Some(view.selected());
        panel().align_items(Align::Start).child(view)
    });
    harness.frame();
    // The rows mount a frame after the viewport is known, so the window
    // settles before the test looks at it.
    harness.settle();
    harness
}

/// The expanded and selected signals the window's build stashed.
fn handles(stash: &Stash) -> Handles {
    stash.borrow().clone_shallow()
}

impl Handles {
    fn clone_shallow(&self) -> Handles {
        Handles {
            expanded: self.expanded.clone(),
            selected: self.selected.clone(),
        }
    }
}

/// The projected nodes with the tree item role, in paint order.
fn tree_items(harness: &mut Harness) -> Vec<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter(|node| node.role == Role::TreeItem)
        .collect()
}

/// The first name or value on `node` or its projected descendants, from the
/// flat projection `nodes`.
fn text_of(nodes: &[SemanticsNode], node: &SemanticsNode) -> Option<String> {
    node.label
        .clone()
        .or_else(|| node.value.clone())
        .or_else(|| {
            node.children.iter().find_map(|child| {
                nodes
                    .iter()
                    .find(|candidate| candidate.id == *child)
                    .and_then(|child| text_of(nodes, child))
            })
        })
}

/// The row names in the order the rows are painted.
fn row_names(harness: &mut Harness) -> Vec<String> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .filter(|node| node.role == Role::TreeItem)
        .map(|item| {
            text_of(&nodes, item).unwrap_or_else(|| panic!("the row of {:?} has a name", item.id))
        })
        .collect()
}

/// The tree item whose row is named `name`.
fn item_named(harness: &mut Harness, name: &str) -> SemanticsNode {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .filter(|node| node.role == Role::TreeItem)
        .find(|item| text_of(&nodes, item).as_deref() == Some(name))
        .cloned()
        .unwrap_or_else(|| panic!("the window shows a row named {name:?}"))
}

/// The name of the row that holds the keyboard focus.
fn focused_row(harness: &mut Harness) -> Option<String> {
    let nodes = harness.app().semantics_nodes();
    nodes
        .iter()
        .find(|node| node.role == Role::TreeItem && node.focused)
        .and_then(|item| text_of(&nodes, item))
}

/// The middle of `node`.
fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: (node.bounds.x0 + node.bounds.x1) / 2.0,
        y: (node.bounds.y0 + node.bounds.y1) / 2.0,
    }
}

/// Wheels the tree by `down` logical pixels, scrolling down when positive.
fn wheel(harness: &mut Harness, down: f32) {
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: down },
    });
}

/// Clears the focus, as a press on the empty part of the window does.
fn clear_focus(harness: &mut Harness) {
    harness.click(Point { x: 290.0, y: 210.0 });
    harness.frame();
}

#[test]
fn a_press_picks_a_row_and_the_arrows_fold_the_tree() {
    let stash: Stash = Rc::new(RefCell::new(Handles::default()));
    let mut harness = window(&stash, Rc::new(Cell::new(0)), model());
    assert_eq!(row_names(&mut harness), vec!["Alpha", "Beta"]);

    let alpha = item_named(&mut harness, "Alpha");
    harness.click(middle(&alpha));
    harness.frame();
    let handles = handles(&stash);
    assert_eq!(
        handles.selected.unwrap().get(),
        Some(1),
        "a press picks the row"
    );
    assert_eq!(item_named(&mut harness, "Alpha").selected, Some(true));

    harness.press_key(Key::ArrowRight);
    harness.settle();
    assert_eq!(
        row_names(&mut harness),
        vec!["Alpha", "Alpha one", "Alpha two", "Beta"],
        "Right opens the picked row"
    );
    assert!(
        handles
            .expanded
            .as_ref()
            .unwrap()
            .with(|set| set.contains(&1))
    );

    // Right on an open row moves to its first child.
    harness.press_key(Key::ArrowRight);
    harness.settle();
    assert_eq!(
        focused_row(&mut harness).as_deref(),
        Some("Alpha one"),
        "Right on an open row moves to its first child"
    );

    // Left on a closed row moves back to its parent.
    harness.press_key(Key::ArrowLeft);
    harness.settle();
    assert_eq!(
        focused_row(&mut harness).as_deref(),
        Some("Alpha"),
        "Left on a closed leaf moves to its parent"
    );

    // Left on an open row closes it.
    harness.press_key(Key::ArrowLeft);
    harness.settle();
    assert_eq!(
        row_names(&mut harness),
        vec!["Alpha", "Beta"],
        "Left closes"
    );
    assert!(
        !handles
            .expanded
            .as_ref()
            .unwrap()
            .with(|set| set.contains(&1))
    );
}

#[test]
fn the_rows_come_out_in_depth_first_order() {
    let stash: Stash = Rc::new(RefCell::new(Handles::default()));
    let mut harness = window(&stash, Rc::new(Cell::new(0)), model());
    let handles = handles(&stash);

    handles.expanded.as_ref().unwrap().update(|set| {
        set.insert(1);
        set.insert(2);
    });
    harness.settle();

    assert_eq!(
        row_names(&mut harness),
        vec!["Alpha", "Alpha one", "Alpha two", "Beta", "Beta one"],
        "a parent comes before its children, depth first"
    );
}

#[test]
fn a_closed_branch_never_asks_for_its_children() {
    let stash: Stash = Rc::new(RefCell::new(Handles::default()));
    let asks = Rc::new(Cell::new(0));
    let mut harness = window(&stash, Rc::clone(&asks), model());
    let handles = handles(&stash);
    assert_eq!(asks.get(), 0, "a closed tree asks for no children");

    handles.expanded.as_ref().unwrap().update(|set| {
        set.insert(1);
    });
    harness.settle();
    assert_eq!(asks.get(), 1, "opening Alpha asks for Alpha's children");

    handles.expanded.as_ref().unwrap().update(|set| {
        set.insert(2);
    });
    harness.settle();
    assert_eq!(
        asks.get(),
        3,
        "the walk asks every open branch again, Alpha's and Beta's"
    );

    handles.expanded.as_ref().unwrap().update(|set| {
        set.remove(&1);
    });
    harness.settle();
    assert_eq!(
        asks.get(),
        4,
        "closing Alpha asks for Beta's children only, never Alpha's"
    );

    let beta = item_named(&mut harness, "Beta");
    harness.click(middle(&beta));
    harness.frame();
    assert_eq!(
        asks.get(),
        4,
        "a pick re-flattens nothing, so it asks for no children"
    );
}

#[test]
fn the_projection_reports_place_and_state() {
    let stash: Stash = Rc::new(RefCell::new(Handles::default()));
    let mut harness = window(&stash, Rc::new(Cell::new(0)), model());
    let handles = handles(&stash);
    handles.expanded.as_ref().unwrap().update(|set| {
        set.insert(1);
    });
    harness.settle();

    let expected = [
        ("Alpha", 1, 0, 2, Some(true)),
        ("Alpha one", 2, 0, 2, Some(false)),
        ("Alpha two", 2, 1, 2, Some(false)),
        ("Beta", 1, 1, 2, Some(false)),
    ];
    let items = tree_items(&mut harness);
    for (item, (name, level, position, set_size, expanded)) in items.iter().zip(expected) {
        let name_shown = text_of(&harness.app().semantics_nodes(), item);
        assert_eq!(name_shown.as_deref(), Some(name), "the row order holds");
        assert_eq!(item.level, Some(level), "{name} sits at level {level}");
        assert_eq!(item.position, Some(position), "{name} sits at {position}");
        assert_eq!(
            item.set_size,
            Some(set_size),
            "{name} has {set_size} siblings"
        );
        assert_eq!(item.expanded, expanded, "{name} reports its fold state");
    }

    let tree = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Tree)
        .expect("the window shows a tree");
    assert_eq!(
        tree.set_size,
        Some(4),
        "the tree reports the rows the whole tree shows, folded ones included"
    );

    let alpha = item_named(&mut harness, "Alpha");
    harness.click(middle(&alpha));
    harness.frame();
    assert_eq!(item_named(&mut harness, "Alpha").selected, Some(true));
    assert_eq!(
        item_named(&mut harness, "Beta").selected,
        Some(false),
        "the unpicked rows report unpicked"
    );
    assert_eq!(handles.selected.unwrap().get(), Some(1));
}

#[test]
fn the_tree_takes_one_tab_stop() {
    let stash: Stash = Rc::new(RefCell::new(Handles::default()));
    let mut harness = window(&stash, Rc::new(Cell::new(0)), model());

    harness.press_key(Key::Tab);
    harness.frame();
    assert_eq!(
        focused_row(&mut harness).as_deref(),
        Some("Alpha"),
        "Tab enters the tree at its first row"
    );

    harness.press_key(Key::ArrowDown);
    harness.settle();
    assert_eq!(
        focused_row(&mut harness).as_deref(),
        Some("Beta"),
        "Down moves along the mounted rows"
    );

    harness.press_key(Key::Home);
    harness.settle();
    assert_eq!(
        focused_row(&mut harness).as_deref(),
        Some("Alpha"),
        "Home goes back to the first row"
    );

    // With a pick, Tab re-enters at the picked row.
    let beta = item_named(&mut harness, "Beta");
    harness.click(middle(&beta));
    harness.frame();
    clear_focus(&mut harness);
    assert_eq!(focused_row(&mut harness), None);
    harness.press_key(Key::Tab);
    harness.frame();
    assert_eq!(
        focused_row(&mut harness).as_deref(),
        Some("Beta"),
        "Tab re-enters at the picked row"
    );
}

/// Ten thousand nodes: five thousand roots, one child each, every row 20
/// pixels tall.
fn big_model() -> Rc<[Node]> {
    Rc::from(
        (0..5_000u64)
            .map(|index| Node {
                id: index,
                name: "",
                children: vec![Node {
                    id: 5_000 + index,
                    name: "",
                    children: Vec::new(),
                }],
            })
            .collect::<Vec<_>>(),
    )
}

/// A window of 300 by 220 showing the ten thousand node tree, whose rows
/// are 20 pixels tall, in a 200 pixel viewport.
fn big_window(stash: &Stash) -> Harness {
    TextSystem::bundled_only().install();
    let stash = Rc::clone(stash);
    let model = big_model();
    let mut harness = Harness::new(300.0, 220.0, 1.0, move || {
        let model = Rc::clone(&model);
        let view = tree_view(
            move || Rc::clone(&model),
            |node| Rc::from(node.children.as_slice()),
            |node| node.id,
            20.0,
            |node, _depth| {
                panel()
                    .height(px(20.0))
                    .child(label(format!("Node {}", node.id)))
            },
        )
        .height(px(200.0));
        stash.borrow_mut().expanded = Some(view.expanded());
        panel().child(view)
    });
    harness.frame();
    harness.settle();
    harness
}

#[test]
fn ten_thousand_nodes_keep_a_window_of_rows_mounted() {
    // Every row is 20 pixels tall and the viewport is 200, so ten rows are
    // visible and the row that begins at the viewport's bottom edge makes
    // eleven; the overscan adds four on each side: nineteen at most.
    const MOUNTED_BOUND: usize = 19;

    let stash: Stash = Rc::new(RefCell::new(Handles::default()));
    let mut harness = big_window(&stash);

    let names = row_names(&mut harness);
    assert!(
        names.len() <= MOUNTED_BOUND,
        "the top mounts {} rows, at most {MOUNTED_BOUND}",
        names.len()
    );
    assert_eq!(names.first().map(String::as_str), Some("Node 0"));

    // The tree reports the five thousand rows it shows, folded children
    // included in neither count nor mount.
    let tree = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Tree)
        .expect("the window shows a tree");
    assert_eq!(
        tree.set_size,
        Some(5_000),
        "the tree reports the rows it shows"
    );

    wheel(&mut harness, 50_000.0);
    harness.settle();
    let names = row_names(&mut harness);
    assert!(
        names.len() <= MOUNTED_BOUND,
        "the middle mounts {} rows, at most {MOUNTED_BOUND}",
        names.len()
    );
    assert_eq!(
        names.first().map(String::as_str),
        Some("Node 2496"),
        "the overscan reaches four rows above the viewport"
    );
    assert!(
        names.iter().any(|name| name == "Node 2500"),
        "the middle of the tree is on screen"
    );

    wheel(&mut harness, 1_000_000.0);
    harness.settle();
    let names = row_names(&mut harness);
    assert!(
        names.len() <= MOUNTED_BOUND,
        "the bottom mounts {} rows, at most {MOUNTED_BOUND}",
        names.len()
    );
    assert_eq!(
        names.last().map(String::as_str),
        Some("Node 4999"),
        "the end of the tree is on screen"
    );
}
