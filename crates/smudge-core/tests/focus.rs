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
use smudge_core::reactive::{self, RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::{PaintCx, Widget, panel};
use smudge_core::window;
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

#[test]
fn activate_on_keys_runs_the_press_handler_for_enter_and_space() {
    let (presses, on_press) = counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .activate_on_keys()
                .on_press(on_press),
        )
    });
    harness.frame();
    harness.press_key(Key::Tab);

    harness.press_key(Key::Enter);
    harness.press_key(Key::Space);

    assert_eq!(
        presses.get(),
        2,
        "Enter and Space each fire the press handler once"
    );
}

#[test]
fn activate_on_keys_fires_once_per_press_and_not_per_repeat() {
    let (presses, on_press) = counter();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .activate_on_keys()
                .on_press(on_press),
        )
    });
    harness.frame();
    harness.press_key(Key::Tab);

    let repeat = KeyEvent {
        key: Key::Enter,
        pressed: true,
        repeat: true,
        modifiers: Modifiers::default(),
        text: None,
    };
    harness.event(InputEvent::Key(repeat.clone()));
    harness.event(InputEvent::Key(KeyEvent {
        pressed: false,
        ..repeat
    }));
    harness.press_key(Key::Enter);

    assert_eq!(
        presses.get(),
        1,
        "the repeat fires nothing and the fresh press fires once"
    );
}

#[test]
fn activate_on_keys_without_a_press_handler_lets_the_key_bubble() {
    let (enters, on_key) = key_counter(|event| event.key == Key::Enter);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .on_key(on_key)
                .child(panel().width(px(40.0)).height(px(20.0)).activate_on_keys()),
        )
    });
    harness.frame();
    // A press on the inner panel focuses it, because it can take the focus.
    harness.click(Point { x: 20.0, y: 10.0 });

    harness.press_key(Key::Enter);

    assert_eq!(
        enters.get(),
        1,
        "without a press handler the key is not taken and reaches the ancestor"
    );
}

/// A log of committed texts with a handler closure that appends to it.
fn commit_log(name: &'static str) -> (Rc<RefCell<Vec<String>>>, impl FnMut(&str)) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let handler = {
        let log = Rc::clone(&log);
        move |text: &str| log.borrow_mut().push(format!("{name}:{text}"))
    };
    (log, handler)
}

#[test]
fn an_ime_commit_goes_to_the_deepest_handler_in_the_focused_chain() {
    let (outer_log, on_outer) = commit_log("outer");
    let (inner_log, on_inner) = commit_log("inner");
    let (plain_log, on_plain) = commit_log("plain");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .column()
                .on_ime_commit(on_outer)
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .focusable()
                        .on_ime_commit(on_inner),
                )
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .focusable()
                        .child(panel().on_ime_commit(on_plain)),
                ),
        )
    });
    harness.frame();

    let commit = |harness: &mut Harness, text: &str| {
        harness.event(InputEvent::ImeCommit {
            text: text.to_owned(),
        });
    };
    commit(&mut harness, "none");
    harness.click(Point { x: 10.0, y: 5.0 });
    commit(&mut harness, "a");
    harness.click(Point { x: 10.0, y: 25.0 });
    commit(&mut harness, "b");

    assert_eq!(
        *inner_log.borrow(),
        ["inner:a"],
        "the focused node takes the commit and stops it"
    );
    assert_eq!(
        *outer_log.borrow(),
        ["outer:b"],
        "a focused node without a handler passes the commit to its parent"
    );
    assert!(
        plain_log.borrow().is_empty(),
        "a commit never goes down to a child, and nothing hears it while \
         nothing is focused"
    );
}

/// A log of composing texts and the handler that appends to it.
type PreeditLog = (
    Rc<RefCell<Vec<String>>>,
    Box<dyn FnMut(&str, Option<(usize, usize)>)>,
);

/// A log of composing texts with a handler closure that appends to it.
fn preedit_log(name: &'static str) -> PreeditLog {
    let log = Rc::new(RefCell::new(Vec::new()));
    let handler = {
        let log = Rc::clone(&log);
        move |text: &str, cursor: Option<(usize, usize)>| {
            log.borrow_mut().push(format!("{name}:{text}:{cursor:?}"))
        }
    };
    (log, Box::new(handler))
}

#[test]
fn an_ime_preedit_goes_to_the_deepest_handler_in_the_focused_chain() {
    let (outer_log, on_outer) = preedit_log("outer");
    let (inner_log, on_inner) = preedit_log("inner");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .column()
                .on_ime_preedit(on_outer)
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .focusable()
                        .on_ime_preedit(on_inner),
                )
                .child(panel().width(px(40.0)).height(px(20.0)).focusable()),
        )
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 5.0 });
    harness.event(InputEvent::ImePreedit {
        text: String::from("か"),
        cursor: Some((3, 3)),
    });
    harness.event(InputEvent::ImePreedit {
        text: String::new(),
        cursor: None,
    });

    assert_eq!(
        *inner_log.borrow(),
        ["inner:か:Some((3, 3))", "inner::None"],
        "the focused input hears the composing text and the clear"
    );
    assert!(
        outer_log.borrow().is_empty(),
        "a preedit stops at the deepest handler, like a commit"
    );
}

#[test]
fn losing_the_window_focus_clears_the_composition() {
    let (log, on_preedit) = preedit_log("input");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .focusable()
                .on_ime_preedit(on_preedit),
        )
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 5.0 });
    harness.event(InputEvent::ImePreedit {
        text: String::from("にほん"),
        cursor: Some((9, 9)),
    });
    harness.event(InputEvent::WindowFocus(false));

    let last = log.borrow().last().cloned();
    assert_eq!(
        last,
        Some(String::from("input::None")),
        "a window that lost its focus clears the preedit, so a composition \
         never outlives the focus"
    );

    harness.event(InputEvent::WindowFocus(true));
    assert_eq!(
        log.borrow().len(),
        2,
        "gaining the window focus sends nothing"
    );
}

#[test]
fn a_key_with_no_focus_reaches_a_root_on_key() {
    let (escapes, on_root_escape) = key_counter(|event| event.key == Key::Escape);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || panel().on_key(on_root_escape));
    harness.frame();
    assert!(harness.app().focused().is_none());

    harness.press_key(Key::Escape);
    assert_eq!(
        escapes.get(),
        1,
        "a key pressed while nothing holds the focus reaches the root"
    );

    harness.press_key(Key::Escape);
    assert_eq!(
        escapes.get(),
        2,
        "every key pressed without a focused node routes from the root"
    );
}

#[test]
fn an_autofocused_element_takes_the_focus_when_a_show_mounts_it() {
    let (visible, set_visible) = reactive::signal_local(false);
    let hears = Rc::new(Cell::new(0u32));
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let hears = Rc::clone(&hears);
        let log = Rc::clone(&log);
        move || {
            let hears = Rc::clone(&hears);
            let log = Rc::clone(&log);
            scene(panel().width(px(40.0)).height(px(20.0)).child(show(
                move || visible.get(),
                move || {
                    let hears = Rc::clone(&hears);
                    let log = Rc::clone(&log);
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .autofocus()
                        .on_focus(move |focused: bool| log.borrow_mut().push(("input", focused)))
                        .on_key(move |event: &KeyEvent| {
                            if event.pressed && event.key == Key::Escape {
                                hears.set(hears.get() + 1);
                                Handled::Yes
                            } else {
                                Handled::No
                            }
                        })
                },
            )))
        }
    });
    harness.frame();
    assert!(harness.app().focused().is_none());
    assert!(log.borrow().is_empty());

    set_visible.set(true);
    harness.frame();
    assert_eq!(
        *log.borrow(),
        [("input", true)],
        "the autofocus takes the focus in the frame the element mounts"
    );

    harness.press_key(Key::Escape);
    assert_eq!(hears.get(), 1, "the focused element hears the next key");
}

#[test]
fn autofocus_prefers_the_last_node_in_paint_order() {
    let (first_log, first_on_focus) = focus_log("first");
    let (second_log, second_on_focus) = focus_log("second");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .autofocus()
                        .on_focus(first_on_focus),
                )
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .autofocus()
                        .on_focus(second_on_focus),
                ),
        )
    });
    harness.frame();
    assert!(
        first_log.borrow().is_empty(),
        "the node painted first loses"
    );
    assert_eq!(
        *second_log.borrow(),
        [("second", true)],
        "the node painted last wins"
    );
}

#[test]
fn tab_cycles_inside_a_focus_trap_and_never_reaches_nodes_outside_it() {
    let (a_log, a_on_focus) = focus_log("a");
    let (b_log, b_on_focus) = focus_log("b");
    let (c_log, c_on_focus) = focus_log("c");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .child(
                    panel()
                        .trap_focus()
                        .child(
                            panel()
                                .width(px(20.0))
                                .height(px(20.0))
                                .focusable()
                                .on_focus(a_on_focus),
                        )
                        .child(
                            panel()
                                .width(px(20.0))
                                .height(px(20.0))
                                .focusable()
                                .on_focus(b_on_focus),
                        ),
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

    harness.press_key(Key::Tab);
    assert_eq!(
        *a_log.borrow(),
        [("a", true)],
        "a Tab with nothing focused enters the trap"
    );
    harness.press_key(Key::Tab);
    assert_eq!(*b_log.borrow(), [("b", true)]);
    harness.press_key(Key::Tab);
    assert_eq!(
        *a_log.borrow(),
        [("a", true), ("a", false), ("a", true)],
        "the cycle wraps inside the trap"
    );

    shift_tab(&mut harness);
    assert_eq!(
        *b_log.borrow(),
        [("b", true), ("b", false), ("b", true)],
        "Shift+Tab reverses inside the trap"
    );
    assert!(
        c_log.borrow().is_empty(),
        "no Tab press moves the focus outside the trap"
    );
}

#[test]
fn a_nested_focus_trap_wins_and_removal_restores_the_outer_one() {
    let (a_log, a_on_focus) = focus_log("a");
    let (b_log, b_on_focus) = focus_log("b");
    let (x_log, _) = focus_log("x");
    let (y_log, _) = focus_log("y");
    let (c_log, c_on_focus) = focus_log("c");
    let (visible, set_visible) = reactive::signal_local(true);
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let x_log = Rc::clone(&x_log);
        let y_log = Rc::clone(&y_log);
        move || {
            scene(
                panel()
                    .child(
                        panel()
                            .trap_focus()
                            .child(
                                panel()
                                    .width(px(20.0))
                                    .height(px(20.0))
                                    .focusable()
                                    .on_focus(a_on_focus),
                            )
                            .child(show(
                                move || visible.get(),
                                move || {
                                    let x_log = Rc::clone(&x_log);
                                    let y_log = Rc::clone(&y_log);
                                    panel()
                                        .trap_focus()
                                        .child(
                                            panel()
                                                .width(px(20.0))
                                                .height(px(20.0))
                                                .focusable()
                                                .on_focus({
                                                    let x_log = Rc::clone(&x_log);
                                                    move |focused: bool| {
                                                        x_log.borrow_mut().push(("x", focused))
                                                    }
                                                }),
                                        )
                                        .child(
                                            panel()
                                                .width(px(20.0))
                                                .height(px(20.0))
                                                .focusable()
                                                .on_focus({
                                                    let y_log = Rc::clone(&y_log);
                                                    move |focused: bool| {
                                                        y_log.borrow_mut().push(("y", focused))
                                                    }
                                                }),
                                        )
                                },
                            ))
                            .child(
                                panel()
                                    .width(px(20.0))
                                    .height(px(20.0))
                                    .focusable()
                                    .on_focus(b_on_focus),
                            ),
                    )
                    .child(
                        panel()
                            .width(px(20.0))
                            .height(px(20.0))
                            .focusable()
                            .on_focus(c_on_focus),
                    ),
            )
        }
    });
    harness.frame();

    harness.press_key(Key::Tab);
    assert_eq!(
        *x_log.borrow(),
        [("x", true)],
        "the nested trap mounted last wins"
    );
    harness.press_key(Key::Tab);
    assert_eq!(*y_log.borrow(), [("y", true)]);
    harness.press_key(Key::Tab);
    assert_eq!(
        *x_log.borrow(),
        [("x", true), ("x", false), ("x", true)],
        "the cycle wraps inside the nested trap"
    );
    assert!(a_log.borrow().is_empty() && b_log.borrow().is_empty());
    assert!(c_log.borrow().is_empty());

    set_visible.set(false);
    harness.frame();
    assert_eq!(
        *x_log.borrow(),
        [("x", true), ("x", false), ("x", true)],
        "a removed node hears no on_focus: its handlers go with it"
    );
    harness.press_key(Key::Tab);
    assert_eq!(
        *a_log.borrow(),
        [("a", true)],
        "removing the nested trap restores the outer one"
    );
    harness.press_key(Key::Tab);
    assert_eq!(*b_log.borrow(), [("b", true)]);
    harness.press_key(Key::Tab);
    assert_eq!(
        *a_log.borrow(),
        [("a", true), ("a", false), ("a", true)],
        "the outer trap cycles"
    );
    assert!(
        c_log.borrow().is_empty(),
        "the focus never leaves the outer trap"
    );
}

#[test]
fn a_press_outside_a_focus_trap_does_not_move_the_focus() {
    let (a_log, a_on_focus) = focus_log("a");
    let (c_log, c_on_focus) = focus_log("c");
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        scene(
            panel()
                .child(
                    panel().width(px(40.0)).height(px(60.0)).trap_focus().child(
                        panel()
                            .width(px(20.0))
                            .height(px(20.0))
                            .focusable()
                            .on_focus(a_on_focus),
                    ),
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

    harness.press_key(Key::Tab);
    assert_eq!(*a_log.borrow(), [("a", true)]);

    harness.click(Point { x: 50.0, y: 10.0 });
    assert_eq!(
        *a_log.borrow(),
        [("a", true)],
        "a press on a focusable node outside the trap does not move the focus"
    );
    assert!(c_log.borrow().is_empty());

    harness.click(Point { x: 80.0, y: 50.0 });
    assert_eq!(
        *a_log.borrow(),
        [("a", true)],
        "a press on a plain node outside the trap does not clear the focus"
    );
}

#[test]
fn a_dialog_trap_hears_keys_before_its_opener_and_returns_the_focus_on_close() {
    let open = RwSignal::new(false);
    let (opener_log, opener_on_focus) = focus_log("opener");
    let (opener_enters, opener_on_key) = key_counter(|event| event.key == Key::Enter);
    let (dialog_enters, dialog_on_enter) = key_counter(|event| event.key == Key::Enter);
    let dialog_on_enter = Rc::new(RefCell::new(dialog_on_enter));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let open = open.clone();
        move || {
            let shown = open.clone();
            let open = open.clone();
            let dialog_on_enter = Rc::clone(&dialog_on_enter);
            scene(
                panel()
                    .child(
                        panel()
                            .width(px(20.0))
                            .height(px(20.0))
                            .on_focus(opener_on_focus)
                            .on_key(opener_on_key),
                    )
                    .child(show(
                        move || shown.get(),
                        move || {
                            let open = open.clone();
                            let dialog_on_enter = Rc::clone(&dialog_on_enter);
                            panel()
                                .width(px(40.0))
                                .height(px(40.0))
                                .trap_focus()
                                .on_key(move |event: &KeyEvent| {
                                    if event.pressed && event.key == Key::Escape {
                                        open.set(false);
                                        Handled::Yes
                                    } else {
                                        (dialog_on_enter.borrow_mut())(event)
                                    }
                                })
                                .child(panel().width(px(20.0)).height(px(20.0)).focusable())
                        },
                    )),
            )
        }
    });
    harness.frame();
    harness.press_key(Key::Tab);
    assert_eq!(*opener_log.borrow(), [("opener", true)]);

    open.set(true);
    harness.frame();
    harness.press_key(Key::Enter);
    assert_eq!(
        opener_enters.get(),
        0,
        "a key pressed while the dialog is open never reaches the control behind it"
    );
    assert_eq!(dialog_enters.get(), 1, "the dialog hears it instead");

    harness.press_key(Key::Tab);
    assert_eq!(
        *opener_log.borrow(),
        [("opener", true), ("opener", false)],
        "Tab moves the focus into the dialog"
    );
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(!open.get(), "the dialog heard Escape and closed");
    assert_eq!(
        *opener_log.borrow(),
        [("opener", true), ("opener", false), ("opener", true)],
        "closing the dialog returns the focus to its opener"
    );
}

#[test]
fn an_ime_commit_with_the_focus_behind_an_active_trap_starts_at_the_trap() {
    let area = Rect {
        x0: 2.0,
        y0: 3.0,
        x1: 30.0,
        y1: 12.0,
    };
    let (editor_log, on_editor) = commit_log("editor");
    let (dialog_log, on_dialog) = commit_log("dialog");
    let on_dialog = Rc::new(RefCell::new(on_dialog));
    let open = RwSignal::new(false);
    let opened = open.clone();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let on_dialog = Rc::clone(&on_dialog);
        let open = opened.clone();
        scene(
            panel()
                .column()
                .child(
                    Element::new(ImeReporter(Some(area)))
                        .width(px(40.0))
                        .height(px(20.0))
                        .focusable()
                        .on_ime_commit(on_editor),
                )
                .child(show(
                    move || open.get(),
                    move || {
                        let on_dialog = Rc::clone(&on_dialog);
                        panel()
                            .width(px(40.0))
                            .height(px(20.0))
                            .trap_focus()
                            .on_ime_commit(move |text| (on_dialog.borrow_mut())(text))
                    },
                )),
        )
    });
    harness.frame();
    harness.click(Point { x: 10.0, y: 5.0 });
    assert_eq!(harness.app().ime_area(), Some(area));

    open.set(true);
    harness.frame();
    harness.event(InputEvent::ImeCommit {
        text: String::from("x"),
    });
    assert_eq!(*dialog_log.borrow(), ["dialog:x"]);
    assert!(
        editor_log.borrow().is_empty(),
        "the editor behind the trap hears no text"
    );
    assert_eq!(
        harness.app().ime_area(),
        None,
        "the input method does not anchor to the editor behind the trap"
    );
}

#[test]
fn window_activation_leaves_the_focus_and_runs_no_focus_handler() {
    let log: FocusLog = Rc::new(RefCell::new(Vec::new()));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let log = Rc::clone(&log);
        move || {
            let first = Rc::clone(&log);
            let second = Rc::clone(&log);
            scene(
                panel()
                    .child(
                        panel()
                            .width(px(20.0))
                            .height(px(20.0))
                            .focusable()
                            .on_focus(move |focused| first.borrow_mut().push(("a", focused))),
                    )
                    .child(
                        panel()
                            .width(px(20.0))
                            .height(px(20.0))
                            .focusable()
                            .on_focus(move |focused| second.borrow_mut().push(("b", focused))),
                    ),
            )
        }
    });
    harness.frame();
    harness.press_key(Key::Tab);
    let focused = harness.app().focused();

    harness.event(InputEvent::WindowFocus(false));
    harness.frame();
    harness.event(InputEvent::WindowFocus(true));
    harness.frame();
    assert_eq!(harness.app().focused(), focused, "the node keeps the focus");
    assert_eq!(*log.borrow(), [("a", true)], "no handler hears the window");

    harness.event(InputEvent::WindowFocus(false));
    harness.press_key(Key::Tab);
    assert_eq!(
        *log.borrow(),
        [("a", true), ("a", false), ("b", true)],
        "a focus move in the background is told at once"
    );
}

#[test]
fn window_focused_reruns_its_readers_on_a_real_change() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut harness = Harness::new(100.0, 60.0, 1.0, {
        let seen = Rc::clone(&seen);
        move || {
            let seen = Rc::clone(&seen);
            let _reader = reactive::RenderEffect::new(move || {
                seen.borrow_mut().push(window::focused());
            });
            panel()
        }
    });
    harness.frame();
    assert_eq!(
        *seen.borrow(),
        [true],
        "an app with no report counts as focused"
    );

    harness.event(InputEvent::WindowFocus(false));
    harness.frame();
    harness.event(InputEvent::WindowFocus(false));
    harness.frame();
    harness.event(InputEvent::WindowFocus(true));
    harness.frame();
    assert_eq!(
        *seen.borrow(),
        [true, false, true],
        "each change reruns the reader once, and a repeated report does not"
    );
}

#[test]
fn a_press_on_a_node_that_keeps_focus_leaves_the_focus_where_it_is() {
    let presses = RwSignal::new(0u32);
    let counted = presses.clone();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        let counted = counted.clone();
        panel()
            .row()
            .child(panel().width(px(40.0)).height(px(40.0)).focusable())
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(40.0))
                    .keeps_focus()
                    .on_press(move || counted.update(|count| *count += 1)),
            )
            .child(panel().width(px(40.0)).height(px(40.0)).on_press(|| {}))
    });
    harness.frame();
    harness.click(Point { x: 20.0, y: 20.0 });
    harness.frame();
    let field = harness
        .app()
        .focused()
        .expect("the press focused the field");

    harness.click(Point { x: 60.0, y: 20.0 });
    harness.frame();
    assert_eq!(presses.get(), 1, "the node heard its press");
    assert_eq!(
        harness.app().focused(),
        Some(field),
        "the focus stayed on the field"
    );

    harness.click(Point { x: 100.0, y: 20.0 });
    harness.frame();
    assert_eq!(
        harness.app().focused(),
        None,
        "a plain node's press clears it"
    );
}

#[test]
fn a_press_on_a_disabled_node_that_keeps_focus_leaves_the_focus_too() {
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        panel()
            .row()
            .child(panel().width(px(40.0)).height(px(40.0)).focusable())
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(40.0))
                    .keeps_focus()
                    .disabled(true)
                    .on_press(|| {}),
            )
    });
    harness.frame();
    harness.click(Point { x: 20.0, y: 20.0 });
    harness.frame();
    let field = harness
        .app()
        .focused()
        .expect("the press focused the field");
    harness.click(Point { x: 60.0, y: 20.0 });
    harness.frame();
    assert_eq!(
        harness.app().focused(),
        Some(field),
        "the disabled button left the focus"
    );
}

#[test]
fn the_node_that_takes_a_press_decides_whether_the_focus_stays() {
    let downs = RwSignal::new(0u32);
    let counted = downs.clone();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        let counted = counted.clone();
        panel()
            .row()
            .child(panel().width(px(40.0)).height(px(40.0)).focusable())
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(40.0))
                    .keeps_focus()
                    .on_pointer_down(move |_| counted.update(|count| *count += 1))
                    .child(
                        panel()
                            .width(px(40.0))
                            .height(px(40.0))
                            .disabled(true)
                            .on_press(|| {}),
                    ),
            )
    });
    harness.frame();
    harness.click(Point { x: 20.0, y: 20.0 });
    harness.frame();
    let field = harness
        .app()
        .focused()
        .expect("the press focused the field");
    harness.click(Point { x: 60.0, y: 20.0 });
    harness.frame();
    assert_eq!(
        downs.get(),
        1,
        "the container took the press over its disabled child"
    );
    assert_eq!(
        harness.app().focused(),
        Some(field),
        "and its flag kept the focus"
    );
}

/// A column of five focusable items named Apple, Banana, Cherry, Date and
/// Elder, 20 pixels tall each, that navigates with the arrow keys along
/// `axis`, with Date disabled. Returns the harness and the name of each item
/// by its index.
fn arrow_list(
    axis: smudge_core::style::Direction,
    focus_log: Rc<RefCell<Vec<&'static str>>>,
) -> smudge_test::Harness {
    let mut harness = smudge_test::Harness::new(100.0, 120.0, 1.0, move || {
        let names = ["Apple", "Banana", "Cherry", "Date", "Elder"];
        names
            .into_iter()
            .fold(panel().column().navigate_with_arrows(axis), |list, name| {
                let log = Rc::clone(&focus_log);
                list.child(
                    panel()
                        .height(px(20.0))
                        .focusable()
                        .disabled(name == "Date")
                        .accessible_label(name)
                        .on_focus(move |on| {
                            if on {
                                log.borrow_mut().push(name);
                            }
                        }),
                )
            })
    });
    harness.frame();
    harness
}

#[test]
fn arrows_step_through_a_column_wrapping_and_passing_over_disabled_items() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut harness = arrow_list(smudge_core::style::Direction::Column, Rc::clone(&log));
    harness.click(Point { x: 10.0, y: 50.0 });

    for key in [
        Key::ArrowDown,
        Key::ArrowDown,
        Key::ArrowUp,
        Key::Home,
        Key::ArrowUp,
        Key::End,
        Key::ArrowLeft,
    ] {
        harness.press_key(key);
    }

    assert_eq!(
        *log.borrow(),
        ["Cherry", "Elder", "Apple", "Elder", "Apple", "Elder"],
        "End stays on Elder, and Left does nothing in a column"
    );
}

#[test]
fn a_letter_goes_to_the_next_item_whose_name_starts_with_it() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut harness = arrow_list(smudge_core::style::Direction::Column, Rc::clone(&log));
    harness.click(Point { x: 10.0, y: 10.0 });

    harness.press_key(Key::Character(String::from("c")));
    harness.press_key(Key::Character(String::from("E")));
    harness.press_key(Key::Character(String::from("d")));

    assert_eq!(
        *log.borrow(),
        ["Apple", "Cherry", "Elder"],
        "Date is disabled, so d finds nothing"
    );
}

#[test]
fn a_row_steps_with_left_and_right() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut harness = arrow_list(smudge_core::style::Direction::Row, Rc::clone(&log));
    harness.click(Point { x: 10.0, y: 10.0 });

    harness.press_key(Key::ArrowRight);
    harness.press_key(Key::ArrowDown);
    harness.press_key(Key::ArrowLeft);

    assert_eq!(*log.borrow(), ["Apple", "Banana", "Apple"]);
}

#[test]
fn focus_within_hears_the_focus_enter_and_leave_but_not_move_inside() {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let present = RwSignal::new(true);
    let (log, here) = (Rc::clone(&heard), present.clone());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        let (log, here) = (Rc::clone(&log), here.clone());
        panel()
            .child(
                panel()
                    .column()
                    .on_focus_within(move |inside| log.borrow_mut().push(inside))
                    .child(panel().width(px(40.0)).height(px(20.0)).focusable())
                    .child(show(
                        move || here.get(),
                        || panel().width(px(40.0)).height(px(20.0)).focusable(),
                    )),
            )
            .child(panel().width(px(40.0)).height(px(20.0)).focusable())
    });
    harness.frame();

    harness.click(Point { x: 10.0, y: 10.0 });
    harness.click(Point { x: 10.0, y: 30.0 });
    assert_eq!(*heard.borrow(), [true], "moving inside runs nothing");
    harness.click(Point { x: 60.0, y: 10.0 });
    assert_eq!(*heard.borrow(), [true, false]);

    harness.click(Point { x: 10.0, y: 30.0 });
    present.set(false);
    harness.frame();
    assert_eq!(
        *heard.borrow(),
        [true, false, true, false],
        "removing the focused node inside takes the focus out"
    );
}

/// Returns the accessible name of the node that holds the focus.
fn focused_name(harness: &mut Harness) -> Option<String> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
        .and_then(|node| node.label)
}

/// A focusable item 20 pixels square, named `name`, with a role so a screen
/// reader, and the test, sees it.
fn item(name: &'static str) -> Element<smudge_core::widget::Panel> {
    panel()
        .width(px(20.0))
        .height(px(20.0))
        .role(smudge_core::semantics::Role::Button)
        .accessible_label(name)
        .focusable()
}

#[test]
fn a_nested_arrow_container_keeps_the_outer_one_from_stepping_into_it() {
    use smudge_core::style::Direction;
    let mut harness = Harness::new(200.0, 120.0, 1.0, || {
        panel()
            .navigate_with_arrows(Direction::Row)
            .child(item("Cut"))
            .child(
                panel()
                    .column()
                    .navigate_with_arrows(Direction::Column)
                    .child(item("Red"))
                    .child(item("Green"))
                    .child(item("Blue")),
            )
            .child(item("Paste"))
    });
    harness.frame();
    harness.click(Point { x: 30.0, y: 30.0 });
    assert_eq!(focused_name(&mut harness).as_deref(), Some("Green"));

    harness.press_key(Key::ArrowRight);
    assert_eq!(
        focused_name(&mut harness).as_deref(),
        Some("Green"),
        "Right is the inner column's to leave alone"
    );
    harness.press_key(Key::ArrowDown);
    assert_eq!(focused_name(&mut harness).as_deref(), Some("Blue"));
}

#[test]
fn a_shortcut_above_an_arrow_container_outside_an_overlay_hears_its_letter_first() {
    use smudge_core::style::Direction;
    let heard = Rc::new(Cell::new(0u32));
    let count = Rc::clone(&heard);
    let mut harness = Harness::new(200.0, 60.0, 1.0, move || {
        let count = Rc::clone(&count);
        panel()
            .on_key(move |event| {
                if event.pressed && event.key == Key::Character(String::from("h")) {
                    count.set(count.get() + 1);
                    return Handled::Yes;
                }
                Handled::No
            })
            .child(
                panel()
                    .navigate_with_arrows(Direction::Row)
                    .child(item("Files"))
                    .child(item("History")),
            )
    });
    harness.frame();
    harness.click(Point { x: 5.0, y: 5.0 });

    harness.press_key(Key::Character(String::from("h")));
    assert_eq!(heard.get(), 1, "the shortcut took it");
    assert_eq!(focused_name(&mut harness).as_deref(), Some("Files"));
    harness.press_key(Key::ArrowRight);
    assert_eq!(focused_name(&mut harness).as_deref(), Some("History"));
}

#[test]
fn tab_leaves_an_arrow_container_from_a_node_nested_inside_one_of_its_items() {
    use smudge_core::style::Direction;
    let mut harness = Harness::new(200.0, 60.0, 1.0, || {
        panel()
            .child(
                panel()
                    .navigate_with_arrows(Direction::Row)
                    .child(item("First"))
                    .child(panel().child(item("Inner"))),
            )
            .child(item("After"))
    });
    harness.frame();
    harness.click(Point { x: 25.0, y: 5.0 });
    assert_eq!(focused_name(&mut harness).as_deref(), Some("Inner"));

    harness.press_key(Key::Tab);
    assert_eq!(focused_name(&mut harness).as_deref(), Some("After"));
    shift_tab(&mut harness);
    assert_eq!(
        focused_name(&mut harness).as_deref(),
        Some("First"),
        "coming back, with nothing selected, the stop is the first"
    );
}

#[test]
fn a_trap_that_navigates_with_the_arrows_takes_them_with_the_focus_outside() {
    use smudge_core::style::Direction;
    let shown = RwSignal::new(false);
    let open = shown.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let open = open.clone();
        panel().child(item("Behind")).child(show(
            move || open.get(),
            || {
                panel()
                    .column()
                    .trap_focus()
                    .navigate_with_arrows(Direction::Column)
                    .child(item("One"))
                    .child(item("Two"))
            },
        ))
    });
    harness.frame();
    harness.click(Point { x: 5.0, y: 5.0 });
    shown.set(true);
    harness.frame();
    assert_eq!(focused_name(&mut harness).as_deref(), Some("Behind"));

    harness.press_key(Key::ArrowDown);
    assert_eq!(focused_name(&mut harness).as_deref(), Some("One"));
}

#[test]
fn a_value_step_behind_a_trap_reaches_nothing() {
    let shown = RwSignal::new(true);
    let open = shown.clone();
    let heard = Rc::new(Cell::new(0u32));
    let count = Rc::clone(&heard);
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (open, count) = (open.clone(), Rc::clone(&count));
        panel()
            .child(item("Level").on_key(move |event| {
                if event.pressed {
                    count.set(count.get() + 1);
                }
                Handled::Yes
            }))
            .child(show(
                move || open.get(),
                || panel().trap_focus().child(item("Dialog").autofocus()),
            ))
    });
    harness.frame();
    let level = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some("Level"))
        .expect("the level shows")
        .id;

    assert!(!harness.app().step_value(level, true));
    assert_eq!(heard.get(), 0);
    shown.set(false);
    harness.frame();
    assert!(harness.app().step_value(level, true));
    assert_eq!(heard.get(), 1, "the press, not the release, counts");
}
