//! Behavior tests for every pointer button and the context menu.
//!
//! Every test drives the same `App::handle_event` a window does, through the
//! harness, and observes the handlers the tree runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::element::{IntoElement, show};
use smudge_core::geometry::Point;
use smudge_core::input::{ButtonEvent, Handled, InputEvent, Key, Modifiers, PointerButton};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;

/// A log of the events a handler heard.
type Log<T> = Rc<RefCell<Vec<T>>>;

/// Returns an empty log.
fn log<T>() -> Log<T> {
    Rc::new(RefCell::new(Vec::new()))
}

/// Wraps a node under test in a plain parent, padded so its rectangle does
/// not start at the window's corner.
fn scene(child: impl IntoElement) -> impl IntoElement {
    panel().child(panel().padding(px(10.0)).child(child))
}

/// Presses `button` at `position`.
fn down(harness: &mut Harness, position: Point, button: PointerButton) {
    harness.event(InputEvent::PointerDown { position, button });
}

/// Releases `button` at `position`.
fn up(harness: &mut Harness, position: Point, button: PointerButton) {
    harness.event(InputEvent::PointerUp { position, button });
}

/// The modifiers with Shift held.
fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

#[test]
fn a_right_and_a_middle_press_reach_the_button_handler_and_never_press() {
    let heard: Log<ButtonEvent> = log();
    let presses = Rc::new(Cell::new(0u32));
    let downs = Rc::new(Cell::new(0u32));
    let (record, pressed, down_count) = (Rc::clone(&heard), Rc::clone(&presses), Rc::clone(&downs));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (record, pressed, down_count) = (
            Rc::clone(&record),
            Rc::clone(&pressed),
            Rc::clone(&down_count),
        );
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_press(move || pressed.set(pressed.get() + 1))
                .on_pointer_down(move |_| down_count.set(down_count.get() + 1))
                .on_button_down(move |event| {
                    record.borrow_mut().push(event);
                    Handled::Yes
                }),
        )
    });
    harness.frame();
    harness.event(InputEvent::ModifiersChanged(shift()));

    let at = Point { x: 15.0, y: 13.0 };
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        down(&mut harness, at, button);
        up(&mut harness, at, button);
    }

    let heard = heard.borrow();
    assert_eq!(
        heard.iter().map(|event| event.button).collect::<Vec<_>>(),
        [PointerButton::Secondary, PointerButton::Middle]
    );
    for event in heard.iter() {
        assert_eq!(
            event.position,
            Point { x: 5.0, y: 3.0 },
            "relative to the node"
        );
        assert!(event.modifiers.shift, "with the modifiers held");
    }
    assert_eq!(presses.get(), 0, "no other button presses the node");
    assert_eq!(downs.get(), 0, "nor reaches its primary pointer down");
}

#[test]
fn a_primary_press_reaches_the_button_handler_and_still_presses() {
    let heard: Log<PointerButton> = log();
    let presses = Rc::new(Cell::new(0u32));
    let (record, pressed) = (Rc::clone(&heard), Rc::clone(&presses));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (record, pressed) = (Rc::clone(&record), Rc::clone(&pressed));
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_press(move || pressed.set(pressed.get() + 1))
                .on_button_down(move |event| {
                    record.borrow_mut().push(event.button);
                    Handled::Yes
                }),
        )
    });
    harness.frame();

    harness.click(Point { x: 15.0, y: 13.0 });

    assert_eq!(*heard.borrow(), [PointerButton::Primary]);
    assert_eq!(presses.get(), 1, "taking the press stops nothing it runs");
}

#[test]
fn a_press_nobody_takes_bubbles_to_the_ancestor_that_does() {
    let heard: Log<&'static str> = log();
    let (inner, outer) = (Rc::clone(&heard), Rc::clone(&heard));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (inner, outer) = (Rc::clone(&inner), Rc::clone(&outer));
        scene(
            panel()
                .padding(px(5.0))
                .on_button_down(move |_| {
                    outer.borrow_mut().push("outer");
                    Handled::Yes
                })
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .on_button_down(move |_| {
                            inner.borrow_mut().push("inner");
                            Handled::No
                        }),
                ),
        )
    });
    harness.frame();

    down(
        &mut harness,
        Point { x: 20.0, y: 20.0 },
        PointerButton::Middle,
    );

    assert_eq!(*heard.borrow(), ["inner", "outer"]);
}

#[test]
fn the_node_that_took_a_middle_press_hears_its_moves_and_release_anywhere() {
    let moves: Log<Point> = log();
    let releases: Log<Point> = log();
    let (moved, released) = (Rc::clone(&moves), Rc::clone(&releases));
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        let (moved, released) = (Rc::clone(&moved), Rc::clone(&released));
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_button_down(|event| {
                    if event.button == PointerButton::Middle {
                        Handled::Yes
                    } else {
                        Handled::No
                    }
                })
                .on_button_move(move |event| moved.borrow_mut().push(event.position))
                .on_button_up(move |event| released.borrow_mut().push(event.position)),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 15.0, y: 15.0 });
    assert!(moves.borrow().is_empty(), "no button is held yet");
    down(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Middle,
    );
    harness.move_to(Point { x: 150.0, y: 80.0 });
    up(
        &mut harness,
        Point { x: 150.0, y: 80.0 },
        PointerButton::Middle,
    );
    harness.move_to(Point { x: 20.0, y: 15.0 });

    assert_eq!(
        *moves.borrow(),
        [Point { x: 140.0, y: 70.0 }],
        "a move outside the node, while held"
    );
    assert_eq!(*releases.borrow(), [Point { x: 140.0, y: 70.0 }]);

    down(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Secondary,
    );
    harness.move_to(Point { x: 30.0, y: 15.0 });
    up(
        &mut harness,
        Point { x: 30.0, y: 15.0 },
        PointerButton::Secondary,
    );
    assert_eq!(moves.borrow().len(), 1, "a declined press brings no moves");
    assert_eq!(releases.borrow().len(), 1, "and no release");
}

#[test]
fn a_second_press_makes_the_button_it_replaces_hear_a_release() {
    let downs: Log<Point> = log();
    let ups: Log<Point> = log();
    let (pressed, released) = (Rc::clone(&downs), Rc::clone(&ups));
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        let (pressed, released) = (Rc::clone(&pressed), Rc::clone(&released));
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_button_down(move |event| {
                    if event.button == PointerButton::Primary {
                        pressed.borrow_mut().push(event.position);
                        Handled::Yes
                    } else {
                        Handled::No
                    }
                })
                .on_button_up(move |event| released.borrow_mut().push(event.position)),
        )
    });
    harness.frame();

    down(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Primary,
    );
    down(
        &mut harness,
        Point { x: 100.0, y: 50.0 },
        PointerButton::Primary,
    );
    up(
        &mut harness,
        Point { x: 100.0, y: 50.0 },
        PointerButton::Primary,
    );

    assert_eq!(
        *downs.borrow(),
        [Point { x: 5.0, y: 5.0 }],
        "the second press lands outside the node, so it takes no press"
    );
    assert_eq!(
        *ups.borrow(),
        [Point { x: 90.0, y: 40.0 }],
        "the replaced press hears its release when the next press comes"
    );
}

#[test]
fn a_window_that_loses_the_focus_ends_a_held_press() {
    let downs: Log<Point> = log();
    let ups: Log<Point> = log();
    let (pressed, released) = (Rc::clone(&downs), Rc::clone(&ups));
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        let (pressed, released) = (Rc::clone(&pressed), Rc::clone(&released));
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_button_down(move |event| {
                    if event.button == PointerButton::Primary {
                        pressed.borrow_mut().push(event.position);
                        Handled::Yes
                    } else {
                        Handled::No
                    }
                })
                .on_button_up(move |event| released.borrow_mut().push(event.position)),
        )
    });
    harness.frame();

    down(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Primary,
    );
    harness.event(InputEvent::WindowFocus(false));
    up(
        &mut harness,
        Point { x: 20.0, y: 20.0 },
        PointerButton::Primary,
    );

    assert_eq!(
        *downs.borrow(),
        [Point { x: 5.0, y: 5.0 }],
        "the focus loss ends the press the release never reached"
    );
    assert_eq!(
        *ups.borrow(),
        [Point { x: 5.0, y: 5.0 }],
        "a release that comes after the focus loss reaches no handler"
    );
}

#[test]
fn a_right_press_asks_the_nearest_context_menu_at_the_pointer() {
    let menus: Log<(&'static str, Point)> = log();
    let (inner, outer) = (Rc::clone(&menus), Rc::clone(&menus));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (inner, outer) = (Rc::clone(&inner), Rc::clone(&outer));
        scene(
            panel()
                .padding(px(5.0))
                .on_context_menu(move |at| outer.borrow_mut().push(("outer", at)))
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .on_context_menu(move |at| inner.borrow_mut().push(("inner", at))),
                ),
        )
    });
    harness.frame();

    down(
        &mut harness,
        Point { x: 20.0, y: 18.0 },
        PointerButton::Secondary,
    );
    down(
        &mut harness,
        Point { x: 20.0, y: 18.0 },
        PointerButton::Middle,
    );
    harness.click(Point { x: 20.0, y: 18.0 });

    assert_eq!(
        *menus.borrow(),
        [("inner", Point { x: 5.0, y: 3.0 })],
        "only the deepest menu, only for the secondary button"
    );
}

#[test]
fn a_right_press_a_button_handler_takes_asks_for_no_context_menu() {
    let menus = Rc::new(Cell::new(0u32));
    let asked = Rc::clone(&menus);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let asked = Rc::clone(&asked);
        scene(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_context_menu(move |_| asked.set(asked.get() + 1))
                .on_button_down(|_| Handled::Yes),
        )
    });
    harness.frame();

    down(
        &mut harness,
        Point { x: 15.0, y: 13.0 },
        PointerButton::Secondary,
    );

    assert_eq!(menus.get(), 0);
}

#[test]
fn the_menu_key_and_shift_f10_ask_the_focused_nodes_context_menu() {
    let menus: Log<Point> = log();
    let asked = Rc::clone(&menus);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let asked = Rc::clone(&asked);
        scene(
            panel()
                .padding(px(5.0))
                .on_context_menu(move |at| asked.borrow_mut().push(at))
                .child(panel().width(px(20.0)).height(px(20.0)).focusable()),
        )
    });
    harness.frame();
    harness.click(Point { x: 20.0, y: 20.0 });
    menus.borrow_mut().clear();

    harness.press_key(Key::ContextMenu);
    harness.press_key_with(Key::Function(10), shift());
    harness.press_key(Key::Function(10));

    assert_eq!(
        *menus.borrow(),
        [Point { x: 5.0, y: 5.0 }, Point { x: 5.0, y: 5.0 }],
        "at the focused node's corner, and F10 alone asks nothing"
    );
}

#[test]
fn a_key_handler_that_takes_the_menu_key_keeps_the_context_menu_shut() {
    let menus = Rc::new(Cell::new(0u32));
    let asked = Rc::clone(&menus);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let asked = Rc::clone(&asked);
        scene(
            panel()
                .width(px(20.0))
                .height(px(20.0))
                .focusable()
                .on_key(|_| Handled::Yes)
                .on_context_menu(move |_| asked.set(asked.get() + 1)),
        )
    });
    harness.frame();
    harness.click(Point { x: 15.0, y: 15.0 });

    harness.press_key(Key::ContextMenu);

    assert_eq!(menus.get(), 0);
}

#[test]
fn a_right_press_takes_no_focus() {
    let focused: Log<bool> = log();
    let record = Rc::clone(&focused);
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let record = Rc::clone(&record);
        scene(
            panel()
                .width(px(20.0))
                .height(px(20.0))
                .focusable()
                .on_focus(move |on| record.borrow_mut().push(on))
                .on_context_menu(|_| {}),
        )
    });
    harness.frame();

    down(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Secondary,
    );
    up(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Secondary,
    );

    assert!(focused.borrow().is_empty());
}

#[test]
fn a_child_with_only_a_context_menu_leaves_primary_presses_to_its_parent() {
    let presses = Rc::new(Cell::new(0u32));
    let menus = Rc::new(Cell::new(0u32));
    let (pressed, asked) = (Rc::clone(&presses), Rc::clone(&menus));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (pressed, asked) = (Rc::clone(&pressed), Rc::clone(&asked));
        scene(
            panel()
                .padding(px(5.0))
                .on_press(move || pressed.set(pressed.get() + 1))
                .child(
                    panel()
                        .width(px(20.0))
                        .height(px(20.0))
                        .on_context_menu(move |_| asked.set(asked.get() + 1)),
                ),
        )
    });
    harness.frame();
    let inside = Point { x: 20.0, y: 20.0 };

    harness.click(inside);
    down(&mut harness, inside, PointerButton::Secondary);

    assert_eq!(presses.get(), 1, "the parent hears the primary press");
    assert_eq!(menus.get(), 1, "the child hears the secondary one");
}

#[test]
fn a_node_removed_while_it_holds_a_button_hears_the_release() {
    let present = RwSignal::new(true);
    let releases = Rc::new(Cell::new(0u32));
    let (here, released) = (present.clone(), Rc::clone(&releases));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (here, released) = (here.clone(), Rc::clone(&released));
        scene(show(
            move || here.get(),
            move || {
                let released = Rc::clone(&released);
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .on_button_down(|_| Handled::Yes)
                    .on_button_up(move |_| released.set(released.get() + 1))
            },
        ))
    });
    harness.frame();
    down(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Middle,
    );

    present.set(false);
    harness.frame();
    assert_eq!(releases.get(), 1, "the removal ends the hold");

    up(
        &mut harness,
        Point { x: 15.0, y: 15.0 },
        PointerButton::Middle,
    );
    assert_eq!(releases.get(), 1, "and the late release reaches nothing");
}
