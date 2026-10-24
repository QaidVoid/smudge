//! Unit tests for keyboard focus.
//!
//! The integration tests observe the handlers through the harness; these tests
//! also look at the focus bits of the nodes and at the focus order walk, which
//! stay crate-private.

use std::cell::Cell;
use std::rc::Rc;

use crate::geometry::{Point, Size};
use crate::input::{Key, KeyEvent, Modifiers, PointerButton};
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

/// Inserts a focusable child of `parent` and returns its id.
fn focusable_child(tree: &mut WidgetTree, parent: WidgetId) -> WidgetId {
    let id = tree.insert(Some(parent), Box::new(Empty), fixed(20.0, 20.0));
    tree.set_focusable(id, true);
    id
}

/// Presses Tab with `shift` held, as a keyboard would deliver it.
fn press_tab(tree: &mut WidgetTree, shift: bool) {
    tree.key(&KeyEvent {
        key: Key::Tab,
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            shift,
            ..Modifiers::default()
        },
        text: None,
    });
}

/// Moves the pointer to (`x`, `y`) and clicks the primary button there.
fn click(tree: &mut WidgetTree, x: f32, y: f32) {
    tree.pointer_move(Point { x, y });
    tree.pointer_down(Point { x, y }, PointerButton::Primary);
    tree.pointer_up(Point { x, y }, PointerButton::Primary);
}

/// Returns whether the node holds the focused bit.
fn focused_bit(tree: &WidgetTree, id: WidgetId) -> bool {
    tree.nodes[id].interaction.has(Interaction::FOCUSED)
}

/// Returns whether the node holds the keyboard focus bit.
fn keyboard_bit(tree: &WidgetTree, id: WidgetId) -> bool {
    tree.nodes[id].interaction.has(Interaction::KEYBOARD_FOCUS)
}

#[test]
fn tab_moves_focus_in_paint_order_and_wraps() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let b = focusable_child(&mut tree, root);
    let c = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a));
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(b));
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(c));
    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(a),
        "Tab wraps from the last to the first"
    );

    press_tab(&mut tree, true);
    assert_eq!(tree.focused(), Some(c), "Shift+Tab reverses");
    press_tab(&mut tree, true);
    assert_eq!(tree.focused(), Some(b));
    press_tab(&mut tree, true);
    assert_eq!(
        tree.focused(),
        Some(a),
        "Shift+Tab wraps from the first to the last"
    );
}

#[test]
fn focus_order_skips_hidden_and_disabled_nodes() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let hidden = focusable_child(&mut tree, root);
    tree.nodes[hidden].style.layout.hidden = true;
    let disabled = focusable_child(&mut tree, root);
    tree.set_disabled(disabled, true);
    let d = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a));
    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(d),
        "a hidden node and a disabled node are not in the focus order"
    );
    press_tab(&mut tree, true);
    assert_eq!(
        tree.focused(),
        Some(a),
        "backward navigation skips them too"
    );
}

#[test]
fn keyboard_navigation_marks_the_focused_node_until_a_press() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert!(keyboard_bit(&tree, a), "Tab marks the focused node");
    assert!(tree.keyboard_nav());

    click(&mut tree, 10.0, 10.0);
    assert_eq!(
        tree.focused(),
        Some(a),
        "a press on the focused node keeps it"
    );
    assert!(focused_bit(&tree, a));
    assert!(
        !keyboard_bit(&tree, a),
        "the press turns keyboard navigation off"
    );
    assert!(!tree.keyboard_nav());
}

#[test]
fn a_press_outside_a_focusable_node_clears_the_focus() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a));

    click(&mut tree, 90.0, 90.0);
    assert_eq!(tree.focused(), None, "the press landed on a plain node");
    assert!(!focused_bit(&tree, a));
}

#[test]
fn a_sole_focusable_node_keeps_the_focus_without_firing_again() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let fires = Rc::new(Cell::new(0u32));
    let handler = Handlers {
        on_focus: Some({
            let fires = Rc::clone(&fires);
            Box::new(move |_| fires.set(fires.get() + 1))
        }),
        ..Handlers::default()
    };
    tree.set_handlers(a, handler);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(fires.get(), 1);
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a), "Tab wraps onto the node itself");
    assert_eq!(
        fires.get(),
        1,
        "focus that stays put hears no further on_focus"
    );
}

#[test]
fn removing_the_focused_node_clears_the_focus() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a));
    tree.remove(a);
    assert_eq!(tree.focused(), None);
    assert!(tree.keyboard_nav(), "the mode survives the removal");
    tree.debug_assert_consistent();
}
