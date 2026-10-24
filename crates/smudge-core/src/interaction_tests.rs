//! Unit tests for the pointer routing.
//!
//! The integration tests observe the handlers; these tests also look at the
//! interaction bits of the nodes on the hovered chain, which stay
//! crate-private.

use std::cell::Cell;
use std::rc::Rc;

use crate::geometry::{Point, Size};
use crate::input::PointerButton;
use crate::paint::PaintOp;
use crate::style::{Length, Style};
use crate::tree::interaction::{Handlers, Interaction};
use crate::tree::{WidgetId, WidgetTree};
use crate::widget::{PaintCx, Widget};

/// A widget with no behavior, for filling tree nodes.
struct Empty;

impl Widget for Empty {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A style with a fixed size.
fn fixed(width: f32, height: f32) -> Style {
    let mut style = Style::default();
    style.layout.width = Length::px(width);
    style.layout.height = Length::px(height);
    style
}

/// A handler set whose `on_press` counts into `presses`.
fn press_handler(presses: &Rc<Cell<u32>>) -> Handlers {
    let presses = Rc::clone(presses);
    Handlers {
        on_press: Some(Box::new(move || presses.set(presses.get() + 1))),
        ..Handlers::default()
    }
}

/// Returns whether the node carries the hovered bit.
fn hovered(tree: &WidgetTree, id: WidgetId) -> bool {
    tree.nodes[id].interaction.has(Interaction::HOVERED)
}

/// Returns whether the node carries the pressed bit.
fn pressed(tree: &WidgetTree, id: WidgetId) -> bool {
    tree.nodes[id].interaction.has(Interaction::PRESSED)
}

#[test]
fn hovering_a_target_hovers_every_ancestor() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let mid = tree.insert(Some(root), Box::new(Empty), fixed(100.0, 100.0));
    let leaf = tree.insert(Some(mid), Box::new(Empty), fixed(50.0, 50.0));
    tree.set_handlers(leaf, press_handler(&Rc::new(Cell::new(0))));
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    tree.pointer_move(Point { x: 10.0, y: 10.0 });
    assert!(hovered(&tree, leaf));
    assert!(hovered(&tree, mid), "the target's ancestors hover with it");
    assert!(hovered(&tree, root));

    tree.pointer_move(Point { x: 90.0, y: 90.0 });
    assert!(!hovered(&tree, leaf));
    assert!(!hovered(&tree, mid));
    assert!(!hovered(&tree, root), "the pointer left every node");
}

#[test]
fn a_target_without_handlers_lets_its_ancestors_respond() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let mid = tree.insert(Some(root), Box::new(Empty), fixed(100.0, 100.0));
    let leaf = tree.insert(Some(mid), Box::new(Empty), fixed(50.0, 50.0));
    let presses = Rc::new(Cell::new(0u32));
    tree.set_handlers(root, press_handler(&presses));
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    tree.pointer_move(Point { x: 10.0, y: 10.0 });
    assert!(
        hovered(&tree, root),
        "the root is the nearest node that can respond"
    );
    assert!(
        !hovered(&tree, mid) && !hovered(&tree, leaf),
        "hover follows the target chain, which starts at the target"
    );

    tree.pointer_down(Point { x: 10.0, y: 10.0 }, PointerButton::Primary);
    assert!(pressed(&tree, root), "the press captures the ancestor");
    assert_eq!(presses.get(), 0, "the press is not released yet");

    tree.pointer_up(Point { x: 10.0, y: 10.0 }, PointerButton::Primary);
    assert_eq!(presses.get(), 1);
    assert!(!pressed(&tree, root));
}

#[test]
fn a_press_with_no_target_is_outside_every_node() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let popup = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 50.0));
    // The second child gives the press at (75, 25) a node to land on, one
    // without handlers of its own.
    let _elsewhere = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 50.0));
    let closes = Rc::new(Cell::new(0u32));
    let handler = Handlers {
        on_press_outside: Some({
            let closes = Rc::clone(&closes);
            Box::new(move || closes.set(closes.get() + 1))
        }),
        ..Handlers::default()
    };
    tree.set_handlers(popup, handler);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    tree.pointer_down(Point { x: 75.0, y: 25.0 }, PointerButton::Primary);
    assert_eq!(
        closes.get(),
        1,
        "the press landed beyond the popup, on a node without handlers"
    );

    tree.pointer_down(Point { x: 10.0, y: 10.0 }, PointerButton::Primary);
    assert_eq!(
        closes.get(),
        1,
        "a press on the popup itself is not outside"
    );
}

#[test]
fn releasing_a_capture_whose_node_is_removed_does_nothing() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let leaf = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 50.0));
    tree.set_handlers(leaf, press_handler(&Rc::new(Cell::new(0))));
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    tree.pointer_down(Point { x: 10.0, y: 10.0 }, PointerButton::Primary);
    tree.remove(leaf);
    tree.pointer_move(Point { x: 90.0, y: 90.0 });
    tree.pointer_up(Point { x: 90.0, y: 90.0 }, PointerButton::Primary);
    tree.pointer_left();
    tree.debug_assert_consistent();
}

#[test]
fn a_second_primary_down_clears_the_first_capture() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let first = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 40.0));
    let second = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 40.0));
    let presses = Rc::new(Cell::new(0));
    tree.set_handlers(first, press_handler(&presses));
    tree.set_handlers(second, press_handler(&presses));
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0,
    }));

    tree.pointer_down(Point { x: 10.0, y: 10.0 }, PointerButton::Primary);
    tree.pointer_down(Point { x: 50.0, y: 10.0 }, PointerButton::Primary);

    assert!(!pressed(&tree, first), "the first capture was replaced");
    assert!(pressed(&tree, second));
}
