//! Window commands, window state and cursors, through the public API.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::input::{InputEvent, PointerButton};
use smudge_core::reactive::prelude::*;
use smudge_core::reactive::{Memo, Owner, RwSignal};
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::panel;
use smudge_core::window::{self, Cursor, ResizeEdge, WindowState};
use smudge_test::Harness;

/// The config of a headless app that changes nothing on its own.
fn config() -> AppConfig {
    AppConfig {
        viewport: Size {
            width: 200.0,
            height: 200.0,
        },
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint: RepaintMode::Damage,
    }
}

#[test]
fn a_press_handler_that_moves_the_window_queues_one_command() {
    let mut app = App::new(config(), || {
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .on_press(window::start_move),
        )
    });
    app.frame();

    let press = Point { x: 10.0, y: 5.0 };
    app.handle_event(InputEvent::PointerMoved { position: press });
    app.handle_event(InputEvent::PointerDown {
        position: press,
        button: PointerButton::Primary,
    });
    app.handle_event(InputEvent::PointerUp {
        position: press,
        button: PointerButton::Primary,
    });

    assert_eq!(
        app.take_window_commands(),
        vec![window::WindowCommand::Move],
        "the press that released over the panel queued exactly one command"
    );
}

#[test]
fn taking_the_commands_empties_the_queue() {
    let mut app = App::new(config(), || {
        panel()
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .on_pointer_down(|_| window::start_resize(ResizeEdge::NorthEast)),
            )
            .child(panel().width(px(40.0)).height(px(20.0)).on_press(|| {
                window::toggle_maximize();
                window::close();
            }))
    });
    app.frame();

    let first = Point { x: 10.0, y: 5.0 };
    app.handle_event(InputEvent::PointerDown {
        position: first,
        button: PointerButton::Primary,
    });
    let second = Point { x: 50.0, y: 10.0 };
    app.handle_event(InputEvent::PointerMoved { position: second });
    app.handle_event(InputEvent::PointerDown {
        position: second,
        button: PointerButton::Primary,
    });
    app.handle_event(InputEvent::PointerUp {
        position: second,
        button: PointerButton::Primary,
    });

    assert_eq!(
        app.take_window_commands(),
        vec![
            window::WindowCommand::Resize(ResizeEdge::NorthEast),
            window::WindowCommand::ToggleMaximize,
            window::WindowCommand::Close,
        ],
        "the commands queue in the order their handlers ran"
    );
    assert!(
        app.take_window_commands().is_empty(),
        "taking the commands empties the queue, so a headless app keeps it \
         only until a test inspects it"
    );
}

#[test]
fn maximized_reruns_a_binding_when_the_state_changes() {
    let mut app = App::new(config(), || {
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .hidden(window::maximized),
        )
    });
    app.frame();

    let mut visible = 0;
    app.visit_nodes(|_, _, _| visible += 1);
    assert_eq!(
        visible, 2,
        "the window starts unmaximized, so the node shows"
    );

    app.set_window_state(WindowState { maximized: true });
    app.frame();

    visible = 0;
    app.visit_nodes(|_, _, _| visible += 1);
    assert_eq!(
        visible, 1,
        "the binding hid the node when the state changed"
    );
}

#[test]
fn an_equal_window_state_report_reruns_no_binding() {
    let runs = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&runs);
    let mut app = App::new(config(), move || {
        let counted = Rc::clone(&counted);
        panel().child(panel().width(px(40.0)).height(px(20.0)).hidden(move || {
            counted.set(counted.get() + 1);
            window::maximized()
        }))
    });
    app.frame();

    app.set_window_state(WindowState { maximized: true });
    app.frame();
    let after_change = runs.get();

    app.set_window_state(WindowState { maximized: true });
    app.frame();
    assert_eq!(
        runs.get(),
        after_change,
        "an equal report wakes no binding that reads the state"
    );
}

#[test]
fn a_memo_that_reads_maximized_follows_every_change() {
    let mut app = App::new(config(), || {
        let maximized = Memo::new(window::maximized);
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .hidden(move || maximized.get()),
        )
    });
    app.frame();

    let mut visible = 0;
    for maximized in [true, false, true] {
        app.set_window_state(WindowState { maximized });
        app.frame();
        visible = 0;
        app.visit_nodes(|_, _, _| visible += 1);
        assert_eq!(
            visible,
            if maximized { 1 } else { 2 },
            "the memo recomputed while no owner was current and kept \
             reading the state"
        );
    }
    assert_eq!(visible, 1);
}

#[test]
fn a_window_command_with_no_live_application_does_nothing() {
    window::close();
    window::start_move();

    let kept: Rc<RefCell<Option<Owner>>> = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&kept);
    let app = App::new(config(), move || {
        slot.replace(Some(Owner::new()));
        panel()
    });
    drop(app);
    let owner = kept.take().expect("the build closure stored its owner");
    owner.with(|| {
        window::toggle_maximize();
        window::start_resize(ResizeEdge::South);
    });
    assert!(
        !owner.with(window::maximized),
        "an owner whose application is gone reads an unmaximized window"
    );
}

#[test]
fn releasing_a_press_restores_hover_without_running_on_press() {
    let presses = Rc::new(Cell::new(0_u32));
    let pressed = Rc::clone(&presses);
    let hovered = Rc::new(Cell::new(false));
    let hover = Rc::clone(&hovered);
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        let pressed = Rc::clone(&pressed);
        let hover = Rc::clone(&hover);
        panel()
            .child(
                panel()
                    .width(px(100.0))
                    .height(px(40.0))
                    .on_pointer_down(|_| window::start_move())
                    .on_press(move || pressed.set(pressed.get() + 1)),
            )
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .cursor(Cursor::Pointer)
                    .on_hover(move |on| hover.set(on)),
            )
    });
    harness.frame();

    let header = Point { x: 20.0, y: 20.0 };
    harness.move_to(header);
    harness.button_down(header);
    assert_eq!(
        harness.app().take_window_commands(),
        vec![window::WindowCommand::Move]
    );
    // The window system took the press for the move, so its release never
    // reaches the app.
    harness.app().release_press();
    harness.event(InputEvent::PointerLeft);

    harness.move_to(Point { x: 120.0, y: 10.0 });
    assert!(
        hovered.get(),
        "hover follows the pointer again once the press is released"
    );
    assert_eq!(harness.app().cursor(), Cursor::Pointer);

    harness.button_up(Point { x: 20.0, y: 20.0 });
    assert_eq!(
        presses.get(),
        0,
        "a press that the window system took does not count as a press"
    );
}

#[test]
fn cursor_follows_the_hovered_chain_and_a_nested_setter_wins() {
    let mut harness = Harness::new(200.0, 200.0, 1.0, || {
        panel().child(
            panel()
                .width(px(200.0))
                .height(px(40.0))
                .cursor(Cursor::Pointer)
                .child(
                    panel()
                        .absolute()
                        .inset(Edges {
                            top: px(0.0),
                            right: px(50.0),
                            bottom: px(0.0),
                            left: px(100.0),
                        })
                        .cursor(Cursor::Text),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 125.0, y: 20.0 });
    assert_eq!(
        harness.app().cursor(),
        Cursor::Text,
        "the deepest setter under the pointer wins"
    );

    harness.move_to(Point { x: 50.0, y: 20.0 });
    assert_eq!(
        harness.app().cursor(),
        Cursor::Pointer,
        "over the parent alone, the parent's cursor shows"
    );

    harness.move_to(Point { x: 150.0, y: 150.0 });
    assert_eq!(
        harness.app().cursor(),
        Cursor::Default,
        "over no node that sets one, the default shows"
    );
}

#[test]
fn a_bound_cursor_follows_its_signal() {
    let wide = RwSignal::new(true);
    let wide_for_width = wide.clone();
    let wide_for_cursor = wide.clone();
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().child(
            panel()
                .width(move || {
                    if wide_for_width.get() {
                        Length::px(200.0)
                    } else {
                        Length::px(40.0)
                    }
                })
                .height(px(40.0))
                .cursor(move || {
                    if wide_for_cursor.get() {
                        Cursor::Resize(ResizeEdge::East)
                    } else {
                        Cursor::Pointer
                    }
                }),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 150.0, y: 20.0 });
    assert_eq!(harness.app().cursor(), Cursor::Resize(ResizeEdge::East));

    wide.set(false);
    harness.frame();
    assert!(
        harness.app().cursor() == Cursor::Default,
        "the node shrank away from the pointer, so nothing under it sets a \
         cursor"
    );

    harness.move_to(Point { x: 20.0, y: 20.0 });
    assert_eq!(harness.app().cursor(), Cursor::Pointer);
}

#[test]
fn a_cursor_only_child_leaves_the_press_to_its_parent() {
    let presses = Rc::new(Cell::new(0_u32));
    let pressed = Rc::clone(&presses);
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        let pressed = Rc::clone(&pressed);
        panel().child(
            panel()
                .width(px(120.0))
                .height(px(40.0))
                .on_press(move || pressed.set(pressed.get() + 1))
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .cursor(Cursor::Text),
                ),
        )
    });
    harness.frame();

    let over_child = Point { x: 20.0, y: 10.0 };
    harness.move_to(over_child);
    assert_eq!(
        harness.app().cursor(),
        Cursor::Text,
        "the child that sets no handler still shows its cursor"
    );
    harness.click(over_child);
    assert_eq!(
        presses.get(),
        1,
        "a press on a child that only sets a cursor reaches the parent's \
         handler"
    );
}

#[test]
fn a_cursor_only_child_keeps_the_focus_on_its_focusable_parent() {
    let focused = Rc::new(Cell::new(false));
    let focus = Rc::clone(&focused);
    let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
        panel().child(
            panel()
                .width(px(120.0))
                .height(px(40.0))
                .focusable()
                .on_focus(move |on| focus.set(on))
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .cursor(Cursor::Text),
                ),
        )
    });
    harness.frame();

    harness.click(Point { x: 20.0, y: 10.0 });
    assert!(
        focused.get(),
        "a press on a child that only sets a cursor focuses the parent"
    );
}

#[test]
fn a_disabled_node_shows_no_cursor_of_its_own() {
    let mut harness = Harness::new(200.0, 200.0, 1.0, || {
        panel().child(
            panel()
                .width(px(120.0))
                .height(px(40.0))
                .cursor(Cursor::Pointer)
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .disabled(true)
                        .cursor(Cursor::Text),
                ),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 20.0, y: 10.0 });
    assert_eq!(
        harness.app().cursor(),
        Cursor::Pointer,
        "a disabled node passes the cursor choice to its ancestors"
    );
}

#[test]
fn the_cursor_resets_when_the_pointer_leaves() {
    let mut harness = Harness::new(200.0, 200.0, 1.0, || {
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .cursor(Cursor::Pointer),
        )
    });
    harness.frame();

    harness.move_to(Point { x: 20.0, y: 10.0 });
    assert_eq!(harness.app().cursor(), Cursor::Pointer);
    harness.event(InputEvent::PointerLeft);
    assert_eq!(harness.app().cursor(), Cursor::Default);
}
