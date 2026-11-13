//! Unit tests for keyboard focus.
//!
//! The integration tests observe the handlers through the harness; these tests
//! also look at the focus bits of the nodes and at the focus order walk, which
//! stay crate-private.

use std::cell::Cell;
use std::rc::Rc;

use crate::geometry::{Point, Size};
use crate::input::{Handled, Key, KeyEvent, Modifiers, PointerButton};
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

/// Presses Escape, as a keyboard would deliver it.
fn press_escape(tree: &mut WidgetTree) {
    tree.key(&KeyEvent {
        key: Key::Escape,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::default(),
        text: None,
    });
}

/// Sets an `on_key` handler that counts fresh presses of Escape and takes
/// them.
fn escape_counter() -> (Rc<Cell<u32>>, Handlers) {
    let hears = Rc::new(Cell::new(0u32));
    let handlers = Handlers {
        on_key: Some({
            let hears = Rc::clone(&hears);
            Box::new(move |event: &KeyEvent| {
                if event.pressed && event.key == Key::Escape {
                    hears.set(hears.get() + 1);
                    Handled::Yes
                } else {
                    Handled::No
                }
            })
        }),
        ..Handlers::default()
    };
    (hears, handlers)
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
    tree.apply_layout(hidden, |layout| layout.hidden = true);
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

#[test]
fn a_key_with_no_focus_reaches_the_root() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let (hears, handlers) = escape_counter();
    tree.set_handlers(root, handlers);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_escape(&mut tree);
    assert_eq!(
        hears.get(),
        1,
        "a key pressed while nothing holds the focus routes from the root"
    );
    press_escape(&mut tree);
    assert_eq!(hears.get(), 2, "every anchorless key routes from the root");
}

#[test]
fn tab_moves_inside_a_focus_trap_and_never_outside() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    let a = focusable_child(&mut tree, trap);
    let b = focusable_child(&mut tree, trap);
    focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(a),
        "a Tab with nothing focused enters the trap"
    );
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(b));
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a), "the cycle wraps inside the trap");
    press_tab(&mut tree, true);
    assert_eq!(
        tree.focused(),
        Some(b),
        "Shift+Tab reverses inside the trap"
    );
    press_tab(&mut tree, true);
    assert_eq!(tree.focused(), Some(a));
    tree.debug_assert_consistent();
}

#[test]
fn a_press_outside_a_focus_trap_keeps_the_focus_inside() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    let a = focusable_child(&mut tree, trap);
    focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a));

    click(&mut tree, 50.0, 10.0);
    assert_eq!(
        tree.focused(),
        Some(a),
        "a press on a focusable node outside the trap does not move the focus"
    );
    click(&mut tree, 80.0, 80.0);
    assert_eq!(
        tree.focused(),
        Some(a),
        "a press on a plain node outside the trap does not clear the focus"
    );
    tree.debug_assert_consistent();
}

#[test]
fn removing_a_trap_restores_the_previous_one() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let outer = tree.insert(Some(root), Box::new(Empty), fixed(100.0, 100.0));
    tree.push_focus_trap(outer);
    let a = focusable_child(&mut tree, outer);
    let inner = tree.insert(Some(outer), Box::new(Empty), fixed(50.0, 50.0));
    tree.push_focus_trap(inner);
    let x = focusable_child(&mut tree, inner);
    let y = focusable_child(&mut tree, inner);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(x), "the trap mounted last wins");
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(y));

    tree.remove(inner);
    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(a),
        "removing the trap restores the one mounted before it"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_hidden_trap_yields_to_an_earlier_trap() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let outer = tree.insert(Some(root), Box::new(Empty), fixed(100.0, 100.0));
    tree.push_focus_trap(outer);
    let a = focusable_child(&mut tree, outer);
    let inner = tree.insert(Some(outer), Box::new(Empty), fixed(50.0, 50.0));
    tree.push_focus_trap(inner);
    focusable_child(&mut tree, inner);
    tree.apply_layout(inner, |layout| layout.hidden = true);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(a),
        "a hidden trap does not hold the keyboard"
    );
    tree.debug_assert_consistent();
}

#[test]
fn autofocus_takes_the_focus_after_its_first_layout() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    tree.queue_autofocus(a);
    assert_eq!(
        tree.focused(),
        None,
        "the request waits for the layout that lays the node out"
    );
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        Some(a),
        "the request resolves with the layout pass"
    );
    tree.debug_assert_consistent();
}

#[test]
fn the_last_autofocus_node_in_paint_order_wins() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let b = focusable_child(&mut tree, root);
    tree.queue_autofocus(b);
    tree.queue_autofocus(a);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        Some(b),
        "the request order does not matter, the paint order does"
    );
    assert!(!focused_bit(&tree, a));
    tree.debug_assert_consistent();
}

#[test]
fn autofocus_runs_once_per_mount() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let c = focusable_child(&mut tree, root);
    tree.queue_autofocus(a);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(tree.focused(), Some(a));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(c));
    assert!(tree.layout(Size {
        width: 50.0,
        height: 50.0
    }));
    assert_eq!(
        tree.focused(),
        Some(c),
        "a later layout pass does not ask for the focus again"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_disabled_autofocus_node_yields_to_an_earlier_one() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let b = focusable_child(&mut tree, root);
    tree.queue_autofocus(a);
    tree.queue_autofocus(b);
    tree.set_disabled(b, true);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        Some(a),
        "a node that cannot hold the focus is skipped"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_removed_autofocus_node_is_skipped() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    tree.queue_autofocus(a);
    tree.remove(a);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        None,
        "a request whose node is gone resolves to nothing"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_tab_with_an_empty_trap_neither_moves_nor_clears_the_focus() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    assert!(tree.request_focus(a));
    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(a),
        "a trap that holds no focusable node leaves the focus where it is"
    );
    tree.debug_assert_consistent();
}

#[test]
fn an_autofocus_node_hidden_at_mount_takes_the_focus_when_it_shows() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let a = focusable_child(&mut tree, root);
    tree.queue_autofocus(a);
    tree.apply_layout(a, |layout| layout.hidden = true);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(tree.focused(), None, "a hidden node cannot take the focus");

    tree.apply_layout(a, |layout| layout.hidden = false);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        Some(a),
        "the request waits for the first layout that shows the node"
    );
    tree.debug_assert_consistent();
}

#[test]
fn an_autofocus_node_under_a_hidden_ancestor_waits_for_it_to_show() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let dialog = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 50.0));
    tree.apply_layout(dialog, |layout| layout.hidden = true);
    let a = focusable_child(&mut tree, dialog);
    tree.queue_autofocus(a);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(tree.focused(), None);
    assert!(tree.layout(Size {
        width: 80.0,
        height: 80.0
    }));
    assert_eq!(
        tree.focused(),
        None,
        "a pass that leaves the ancestor hidden keeps the request waiting"
    );

    tree.apply_layout(dialog, |layout| layout.hidden = false);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(tree.focused(), Some(a));
    tree.debug_assert_consistent();
}

#[test]
fn a_trap_under_a_hidden_ancestor_does_not_hold_the_keyboard() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let page = tree.insert(Some(root), Box::new(Empty), fixed(100.0, 100.0));
    tree.apply_layout(page, |layout| layout.hidden = true);
    let trap = tree.insert(Some(page), Box::new(Empty), fixed(40.0, 40.0));
    tree.push_focus_trap(trap);
    focusable_child(&mut tree, trap);
    let c = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(
        tree.focused(),
        Some(c),
        "a trap under a hidden ancestor leaves the keyboard to the tree"
    );
    tree.debug_assert_consistent();
}

#[test]
fn an_autofocus_outside_an_active_trap_does_not_take_the_focus() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    let a = focusable_child(&mut tree, trap);
    tree.queue_autofocus(a);
    let b = focusable_child(&mut tree, root);
    tree.queue_autofocus(b);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        Some(a),
        "the autofocus inside the trap wins over the one painted after it"
    );
    assert!(!focused_bit(&tree, b));
    tree.debug_assert_consistent();
}

#[test]
fn an_autofocus_with_no_node_inside_the_trap_takes_nothing() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    focusable_child(&mut tree, trap);
    let b = focusable_child(&mut tree, root);
    tree.queue_autofocus(b);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert_eq!(
        tree.focused(),
        None,
        "a node outside the trap cannot pull the focus out of it"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_press_outside_a_trap_keeps_the_key_anchor_of_a_dropped_focus() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    let a = focusable_child(&mut tree, trap);
    let (root_hears, root_handlers) = escape_counter();
    tree.set_handlers(root, root_handlers);
    let (trap_hears, trap_handlers) = escape_counter();
    tree.set_handlers(trap, trap_handlers);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(a));
    tree.set_disabled(a, true);
    assert_eq!(tree.focused(), None);

    click(&mut tree, 80.0, 80.0);
    press_escape(&mut tree);
    assert_eq!(
        trap_hears.get(),
        1,
        "the key still routes above the node the focus fell off of"
    );
    assert_eq!(root_hears.get(), 0);
    tree.debug_assert_consistent();
}

#[test]
fn a_key_with_the_focus_outside_an_active_trap_starts_at_the_trap() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let opener = focusable_child(&mut tree, root);
    let (opener_hears, opener_handlers) = escape_counter();
    tree.set_handlers(opener, opener_handlers);
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    let (trap_hears, trap_handlers) = escape_counter();
    tree.set_handlers(trap, trap_handlers);
    focusable_child(&mut tree, trap);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert!(tree.request_focus(opener));
    tree.push_focus_trap(trap);

    press_escape(&mut tree);
    assert_eq!(trap_hears.get(), 1, "the trap hears the key first");
    assert_eq!(
        opener_hears.get(),
        0,
        "the focused node behind the trap does not hear it"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_key_with_no_focus_under_an_active_trap_starts_at_the_trap() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let (root_hears, root_handlers) = escape_counter();
    tree.set_handlers(root, root_handlers);
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    let (trap_hears, trap_handlers) = escape_counter();
    tree.set_handlers(trap, trap_handlers);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    press_escape(&mut tree);
    assert_eq!(trap_hears.get(), 1, "the trap's handler takes the key");
    assert_eq!(root_hears.get(), 0);

    tree.remove(trap);
    press_escape(&mut tree);
    assert_eq!(
        root_hears.get(),
        1,
        "with the trap gone the key starts at the root again"
    );
    tree.debug_assert_consistent();
}

#[test]
fn removing_a_trap_returns_the_focus_to_the_node_that_held_it() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let opener = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert!(tree.request_focus(opener));

    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    let field = focusable_child(&mut tree, trap);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(field));

    tree.remove(trap);
    assert_eq!(
        tree.focused(),
        Some(opener),
        "the focus goes back to where it was when the trap mounted"
    );
    assert!(focused_bit(&tree, opener));
    tree.debug_assert_consistent();
}

#[test]
fn removing_nested_traps_together_restores_the_focus_before_the_outer_one() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let opener = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert!(tree.request_focus(opener));

    let outer = tree.insert(Some(root), Box::new(Empty), fixed(80.0, 80.0));
    tree.push_focus_trap(outer);
    let a = focusable_child(&mut tree, outer);
    assert!(tree.request_focus(a));
    let inner = tree.insert(Some(outer), Box::new(Empty), fixed(40.0, 40.0));
    tree.push_focus_trap(inner);
    focusable_child(&mut tree, inner);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));

    tree.remove(outer);
    assert_eq!(tree.focused(), Some(opener));
    tree.debug_assert_consistent();
}

#[test]
fn removing_a_trap_after_its_restore_node_left_leaves_the_focus_empty() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let opener = focusable_child(&mut tree, root);
    assert!(tree.layout(Size {
        width: 100.0,
        height: 100.0
    }));
    assert!(tree.request_focus(opener));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    focusable_child(&mut tree, trap);

    tree.remove(opener);
    tree.remove(trap);
    assert_eq!(tree.focused(), None);
    tree.debug_assert_consistent();
}

#[test]
fn hiding_a_trap_returns_the_focus_held_when_it_last_showed() {
    let viewport = Size {
        width: 100.0,
        height: 100.0,
    };
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let first = focusable_child(&mut tree, root);
    let second = focusable_child(&mut tree, root);
    let mut hidden = fixed(40.0, 100.0);
    hidden.layout.hidden = true;
    assert!(tree.layout(viewport));
    assert!(tree.request_focus(first));
    let trap = tree.insert(Some(root), Box::new(Empty), hidden);
    tree.push_focus_trap(trap);
    let field = focusable_child(&mut tree, trap);
    assert!(tree.layout(viewport));
    assert!(tree.request_focus(second));

    tree.apply_layout(trap, |layout| layout.hidden = false);
    assert!(tree.layout(viewport));
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(field));

    tree.apply_layout(trap, |layout| layout.hidden = true);
    assert!(tree.layout(viewport));
    assert_eq!(
        tree.focused(),
        Some(second),
        "the focus goes back to where it was when the trap showed, not \
         where it was when the trap mounted"
    );

    tree.apply_layout(trap, |layout| layout.hidden = false);
    assert!(tree.layout(viewport));
    press_tab(&mut tree, false);
    assert_eq!(tree.focused(), Some(field));
    tree.remove(trap);
    assert_eq!(
        tree.focused(),
        Some(second),
        "removing a trap that showed again returns the focus too"
    );
    tree.debug_assert_consistent();
}

#[test]
fn hiding_a_trap_leaves_a_focus_outside_it_alone() {
    let viewport = Size {
        width: 100.0,
        height: 100.0,
    };
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(100.0, 100.0));
    let first = focusable_child(&mut tree, root);
    let second = focusable_child(&mut tree, root);
    assert!(tree.layout(viewport));
    assert!(tree.request_focus(first));
    let trap = tree.insert(Some(root), Box::new(Empty), fixed(40.0, 100.0));
    tree.push_focus_trap(trap);
    focusable_child(&mut tree, trap);
    assert!(tree.layout(viewport));
    assert!(tree.request_focus(second));

    tree.apply_layout(trap, |layout| layout.hidden = true);
    assert!(tree.layout(viewport));
    assert_eq!(tree.focused(), Some(second));
    tree.debug_assert_consistent();
}
