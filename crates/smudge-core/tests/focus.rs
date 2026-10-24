//! Behavior tests for keyboard focus and key routing.
//!
//! Every test drives the same `App::handle_event` a window does, through the
//! harness, and observes the focus handlers the tree runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::element::{Element, IntoElement, show};
use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{Handled, InputEvent, Key, KeyEvent, Modifiers};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::{PaintCx, Widget, panel};
use smudge_test::Harness;

/// A counter shared with a handler.
type Count = Rc<Cell<u32>>;

/// A counter with a handler closure that increments it.
fn counter() -> (Count, impl FnMut()) {
    let count = Rc::new(Cell::new(0u32));
    let handler = {
        let count = Rc::clone(&count);
        move || count.set(count.get() + 1)
    };
    (count, handler)
}

/// A key handler that counts the pressed keys `matches` accepts and takes
/// them, letting every other event bubble on.
fn key_counter(
    matches: impl Fn(&KeyEvent) -> bool + 'static,
) -> (Count, impl FnMut(&KeyEvent) -> Handled) {
    let count = Rc::new(Cell::new(0u32));
    let handler = {
        let count = Rc::clone(&count);
        move |event: &KeyEvent| {
            if event.pressed && matches(event) {
                count.set(count.get() + 1);
                Handled::Yes
            } else {
                Handled::No
            }
        }
    };
    (count, handler)
}

/// A log of focus changes, shared between a test and its handler.
type FocusLog = Rc<RefCell<Vec<(&'static str, bool)>>>;

/// A log of focus changes with a handler closure that appends to it.
fn focus_log(name: &'static str) -> (FocusLog, impl FnMut(bool)) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let handler = {
        let log = Rc::clone(&log);
        move |focused: bool| log.borrow_mut().push((name, focused))
    };
    (log, handler)
}

/// Wraps a node under test in a plain parent.
///
/// The layout pass sizes the root to the whole viewport, whatever its style
/// says, so a node with a size of its own has to sit under the root.
fn scene(child: impl IntoElement) -> impl IntoElement {
    panel().child(child)
}

/// Injects a press and release of Tab with Shift held.
fn shift_tab(harness: &mut Harness) {
    let press = KeyEvent {
        key: Key::Tab,
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
        text: None,
    };
    harness.event(InputEvent::Key(press.clone()));
    harness.event(InputEvent::Key(KeyEvent {
        pressed: false,
        ..press
    }));
}

/// A panel-shaped widget that reports an input method area.
struct ImeReporter(Option<Rect>);

impl Widget for ImeReporter {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn ime_area(&self) -> Option<Rect> {
        self.0
    }
}

#[test]
fn tab_cycles_through_focusable_nodes_and_shift_tab_reverses() {
    let (a_log, a_on_focus) = focus_log("a");
    let (b_log, b_on_focus) = focus_log("b");
    let (c_log, c_on_focus) = focus_log("c");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(20.0))
                .height(px(20.0))
                .focusable()
                .on_focus(a_on_focus)
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .focusable()
                        .on_focus(b_on_focus),
                )
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .focusable()
                        .on_focus(c_on_focus),
                ),
        )
    });
    harness.frame();
    assert!(harness.app().focused().is_none());

    harness.press_key(Key::Tab);
    assert_eq!(*a_log.borrow(), [("a", true)]);
    assert!(b_log.borrow().is_empty() && c_log.borrow().is_empty());

    harness.press_key(Key::Tab);
    assert_eq!(*a_log.borrow(), [("a", true), ("a", false)]);
    assert_eq!(*b_log.borrow(), [("b", true)]);

    harness.press_key(Key::Tab);
    assert_eq!(*b_log.borrow(), [("b", true), ("b", false)]);
    assert_eq!(*c_log.borrow(), [("c", true)]);

    harness.press_key(Key::Tab);
    assert_eq!(*c_log.borrow(), [("c", true), ("c", false)]);
    assert_eq!(
        *a_log.borrow(),
        [("a", true), ("a", false), ("a", true)],
        "Tab wraps from the last focusable node to the first"
    );

    shift_tab(&mut harness);
    assert_eq!(a_log.borrow().len(), 4);
    assert_eq!(
        *c_log.borrow(),
        [("c", true), ("c", false), ("c", true)],
        "Shift+Tab moves back"
    );

    shift_tab(&mut harness);
    assert_eq!(*b_log.borrow(), [("b", true), ("b", false), ("b", true)]);
}

#[test]
fn a_key_bubbles_from_the_focused_child_to_a_parent() {
    let (escapes, on_child_escape) = key_counter(|event| event.key == Key::Escape);
    let (enters, on_parent_enter) = key_counter(|event| event.key == Key::Enter);
    let (characters, on_root_character) =
        key_counter(|event| matches!(event.key, Key::Character(_)));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(60.0))
                .on_key(on_root_character)
                .child(
                    panel()
                        .width(px(30.0))
                        .height(px(30.0))
                        .on_key(on_parent_enter)
                        .child(
                            panel()
                                .width(px(10.0))
                                .height(px(10.0))
                                .on_key(on_child_escape),
                        ),
                ),
        )
    });
    harness.frame();

    // The leaf hears its own key first, so a click puts the focus on it.
    harness.click(Point { x: 5.0, y: 5.0 });
    assert!(harness.app().focused().is_some());

    harness.press_key(Key::Escape);
    assert_eq!(
        escapes.get(),
        1,
        "the focused child handles its own key, and it stops there"
    );
    assert_eq!(enters.get(), 0);

    harness.press_key(Key::Enter);
    assert_eq!(
        enters.get(),
        1,
        "the key the child drops bubbles to the parent"
    );
    assert_eq!(characters.get(), 0, "a handled key stops bubbling");

    harness.type_text("x");
    assert_eq!(
        characters.get(),
        1,
        "a key no handler takes reaches the root"
    );
    assert_eq!(enters.get(), 1);
}

#[test]
fn removing_the_focused_node_clears_the_focus() {
    let (visible, set_visible) = reactive::signal_local(true);
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || {
            scene(
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .child(panel().width(px(20.0)).height(px(20.0)).focusable())
                    .child({
                        let log = Rc::clone(&log);
                        show(
                            move || visible.get(),
                            move || {
                                let on_focus = {
                                    let log = Rc::clone(&log);
                                    move |focused: bool| {
                                        log.borrow_mut().push(("target", focused));
                                    }
                                };
                                panel()
                                    .width(px(20.0))
                                    .height(px(20.0))
                                    .focusable()
                                    .on_focus(on_focus)
                            },
                        )
                    }),
            )
        }
    });
    harness.frame();

    harness.click(Point { x: 30.0, y: 10.0 });
    assert!(harness.app().focused().is_some());
    assert_eq!(*log.borrow(), [("target", true)]);

    set_visible.set(false);
    harness.frame();
    assert!(harness.app().focused().is_none());
    assert_eq!(
        *log.borrow(),
        [("target", true)],
        "a removed node hears no on_focus: its handlers go with it"
    );

    harness.press_key(Key::Tab);
    assert!(
        harness.app().focused().is_some(),
        "focus moves again after the focused node was removed"
    );
}

#[test]
fn a_tab_turns_keyboard_navigation_on_and_a_press_turns_it_off() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(panel().width(px(40.0)).height(px(20.0)).focusable())
    });
    harness.frame();
    assert!(!harness.app().keyboard_navigation());

    harness.press_key(Key::Tab);
    assert!(harness.app().keyboard_navigation());
    assert!(harness.app().focused().is_some());

    harness.click(Point { x: 80.0, y: 50.0 });
    assert!(!harness.app().keyboard_navigation());
    assert!(
        harness.app().focused().is_none(),
        "the press cleared the focus"
    );
}

#[test]
fn an_arrow_key_turns_keyboard_navigation_on() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(panel().width(px(40.0)).height(px(20.0)).focusable())
    });
    harness.frame();

    harness.press_key(Key::ArrowDown);
    assert!(harness.app().keyboard_navigation());
    assert!(
        harness.app().focused().is_none(),
        "arrows turn the mode on without moving the focus"
    );
}

#[test]
fn a_press_on_a_focusable_node_focuses_it_and_a_press_elsewhere_clears_it() {
    let (log, on_focus) = focus_log("a");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .focusable()
                .on_focus(on_focus),
        )
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 5.0 });
    assert_eq!(*log.borrow(), [("a", true)]);
    assert!(harness.app().focused().is_some());

    harness.click(Point { x: 10.0, y: 5.0 });
    assert_eq!(
        *log.borrow(),
        [("a", true)],
        "a press on the focused node keeps it without a second on_focus"
    );

    harness.click(Point { x: 80.0, y: 50.0 });
    assert_eq!(*log.borrow(), [("a", true), ("a", false)]);
    assert!(harness.app().focused().is_none());
}

#[test]
fn a_handled_tab_keeps_the_focus_where_it_is() {
    let (tabs, mut on_tab) = counter();
    let (a_log, a_on_focus) = focus_log("a");
    let (b_log, b_on_focus) = focus_log("b");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(80.0))
                .height(px(20.0))
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .on_focus(a_on_focus)
                        .on_key(move |event| {
                            if event.key == Key::Tab {
                                if event.pressed {
                                    on_tab();
                                }
                                Handled::Yes
                            } else {
                                Handled::No
                            }
                        }),
                )
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .focusable()
                        .on_focus(b_on_focus),
                ),
        )
    });
    harness.frame();

    harness.press_key(Key::Tab);
    assert_eq!(
        *a_log.borrow(),
        [("a", true)],
        "the first Tab takes the focus"
    );
    assert_eq!(tabs.get(), 0, "no node held the focus for the first Tab");

    harness.press_key(Key::Tab);
    assert_eq!(tabs.get(), 1);
    assert_eq!(
        *a_log.borrow(),
        [("a", true)],
        "the focused node took the Tab key, so the focus stays"
    );
    assert!(b_log.borrow().is_empty());
    assert!(harness.app().keyboard_navigation());
}

#[test]
fn ime_area_reports_the_focused_widgets_area() {
    let area = Rect {
        x0: 2.0,
        y0: 3.0,
        x1: 30.0,
        y1: 12.0,
    };
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel().child(
                Element::new(ImeReporter(Some(area)))
                    .width(px(20.0))
                    .height(px(10.0))
                    .focusable(),
            ),
        )
    });
    harness.frame();
    assert!(harness.app().ime_area().is_none());

    harness.click(Point { x: 5.0, y: 5.0 });
    assert_eq!(harness.app().ime_area(), Some(area));

    harness.click(Point { x: 80.0, y: 50.0 });
    assert_eq!(harness.app().ime_area(), None);
}

#[test]
fn a_focused_node_that_becomes_disabled_hears_no_keys() {
    let (disabled, set_disabled) = reactive::signal_local(false);
    let (enters, on_key) = key_counter(|event| event.key == Key::Enter);
    let (log, on_focus) = focus_log("button");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_key(on_key)
                .on_focus(on_focus)
                .disabled(move || disabled.get()),
        )
    });
    harness.frame();
    harness.press_key(Key::Tab);
    harness.press_key(Key::Enter);
    assert_eq!(enters.get(), 1, "the focused node takes Enter");

    set_disabled.set(true);
    harness.frame();
    harness.press_key(Key::Enter);

    assert_eq!(enters.get(), 1, "a disabled node hears no key");
    assert!(harness.app().focused().is_none());
    assert_eq!(*log.borrow(), [("button", true), ("button", false)]);
}

#[test]
fn a_press_on_the_content_of_a_disabled_focusable_node_focuses_nothing() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(40.0))
                .focusable()
                .disabled(true)
                .child(panel().width(px(10.0)).height(px(10.0))),
        )
    });
    harness.frame();

    harness.click(Point { x: 5.0, y: 5.0 });

    assert!(harness.app().focused().is_none());
}

#[test]
fn a_key_bubbles_past_a_disabled_ancestor_without_running_its_handler() {
    let (inner, on_inner_key) = key_counter(|event| event.key == Key::Enter);
    let (outer, on_outer_key) = key_counter(|event| event.key == Key::Enter);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .on_key(on_outer_key)
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(30.0))
                        .on_key(on_inner_key)
                        .disabled(true)
                        .child(panel().width(px(20.0)).height(px(20.0)).focusable()),
                ),
        )
    });
    harness.frame();
    harness.click(Point { x: 5.0, y: 5.0 });
    harness.press_key(Key::Enter);

    assert_eq!(
        inner.get(),
        0,
        "the disabled container's handler never runs"
    );
    assert_eq!(
        outer.get(),
        1,
        "the key bubbles past it to the enabled ancestor"
    );
}

#[test]
fn a_key_from_a_focused_node_that_became_disabled_reaches_its_ancestors() {
    let (disabled, set_disabled) = reactive::signal_local(false);
    let (outer, on_outer_key) = key_counter(|event| event.key == Key::Enter);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .on_key(on_outer_key)
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .focusable()
                        .disabled(move || disabled.get()),
                ),
        )
    });
    harness.frame();
    harness.click(Point { x: 5.0, y: 5.0 });
    set_disabled.set(true);
    harness.frame();

    harness.press_key(Key::Enter);

    assert_eq!(
        outer.get(),
        1,
        "the shortcut above the disabled node still works"
    );
    assert!(harness.app().focused().is_none());
}

#[test]
fn a_control_disabled_and_enabled_again_does_not_take_keys_without_focus() {
    let (disabled, set_disabled) = reactive::signal_local(false);
    let (enters, on_key) = key_counter(|event| event.key == Key::Enter);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_key(on_key)
                .disabled(move || disabled.get()),
        )
    });
    harness.frame();
    harness.press_key(Key::Tab);
    set_disabled.set(true);
    harness.frame();
    set_disabled.set(false);
    harness.frame();

    harness.press_key(Key::Enter);

    assert!(harness.app().focused().is_none());
    assert_eq!(enters.get(), 0, "a control without the focus takes no key");
}
