//! Whole-frame behavior of nodes that fade in when they mount and out when
//! [`show`] or [`for_each`] removes them, driven through the headless
//! harness on its virtual clock.

use std::time::Duration;

use smudge_core::animation::Spring;
use smudge_core::element::Element;
use smudge_core::element::{for_each, show};
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{Handled, Key};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// The color the shown square paints.
const SQUARE: Color = Color::rgb(200, 40, 40);

/// A point inside the square.
const INSIDE: Point = Point { x: 20.0, y: 20.0 };

/// The interval between frames, about one at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

/// The surface pixel at `point`.
fn pixel(harness: &Harness, point: Point) -> [u8; 4] {
    harness
        .pixel(point.x as u32, point.y as u32)
        .expect("the point is inside the surface")
}

/// How many squares the layout holds, fading ones included.
fn squares(harness: &Harness) -> usize {
    harness
        .layout_dump()
        .lines()
        .filter(|line| line.contains("[0, 0, 40, 40]"))
        .count()
}

/// A harness showing a square while `open` reads `true`, which fades in
/// and out and counts its presses in `presses`.
fn toggled(open: RwSignal<bool>, presses: RwSignal<u32>) -> Harness {
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let open = open.clone();
        let presses = presses.clone();
        panel().child(show(
            move || open.get(),
            move || {
                let presses = presses.clone();
                panel()
                    .width(px(40.0))
                    .height(px(40.0))
                    .background(SQUARE)
                    .role(Role::Button)
                    .focusable()
                    .on_press(move || presses.update(|count| *count += 1))
                    .enter(Spring::SMOOTH)
                    .exit(Spring::SMOOTH)
            },
        ))
    });
    harness.frame();
    harness
}

#[test]
fn a_node_fades_in_when_it_mounts() {
    let open = RwSignal::new(false);
    let mut harness = toggled(open.clone(), RwSignal::new(0));
    open.set(true);
    harness.frame();
    let mut between = 0;
    for _ in 0..60 {
        harness.advance(FRAME);
        let shown = pixel(&harness, INSIDE);
        if shown != [255, 255, 255, 255] && shown != [200, 40, 40, 255] {
            between += 1;
        }
        harness.assert_matches_full_repaint();
    }
    assert!(between >= 5, "only {between} frames showed the fade");
    assert_eq!(pixel(&harness, INSIDE), [200, 40, 40, 255]);
}

#[test]
fn an_exiting_node_takes_no_input_and_goes_once_its_fade_ends() {
    let open = RwSignal::new(true);
    let presses = RwSignal::new(0u32);
    let mut harness = toggled(open.clone(), presses.clone());
    harness.settle();
    for _ in 0..60 {
        harness.advance(FRAME);
    }
    open.set(false);
    harness.frame();
    assert_ne!(
        pixel(&harness, INSIDE),
        [255, 255, 255, 255],
        "the node still shows while it fades"
    );
    harness.click(INSIDE);
    harness.frame();
    assert_eq!(presses.get(), 0, "the fading node took no press");

    for _ in 0..90 {
        harness.advance(FRAME);
        harness.assert_matches_full_repaint();
    }
    assert_eq!(pixel(&harness, INSIDE), [255, 255, 255, 255]);
    assert_eq!(squares(&harness), 0, "the node is gone");
    assert!(!harness.app().is_animating());
}

#[test]
fn rapid_toggles_turn_the_fade_around_and_keep_one_node() {
    let open = RwSignal::new(true);
    let mut harness = toggled(open.clone(), RwSignal::new(0));
    for _ in 0..60 {
        harness.advance(FRAME);
    }
    let mut previous = pixel(&harness, INSIDE);
    for toggle in 0..6 {
        open.set(toggle % 2 == 1);
        for _ in 0..4 {
            harness.advance(FRAME);
            let shown = pixel(&harness, INSIDE);
            let step = (i16::from(shown[1]) - i16::from(previous[1])).abs();
            assert!(step <= 60, "the fade jumped from {previous:?} to {shown:?}");
            previous = shown;
            assert!(squares(&harness) <= 1, "two nodes for one condition");
        }
    }
}

#[test]
fn an_exiting_node_loses_the_focus_and_leaves_the_projection() {
    let open = RwSignal::new(true);
    let mut harness = toggled(open.clone(), RwSignal::new(0));
    harness.press_key(Key::Tab);
    harness.frame();
    assert!(harness.app().focused().is_some(), "Tab focused the square");
    assert!(
        harness
            .app()
            .semantics_nodes()
            .iter()
            .any(|node| node.role == Role::Button)
    );

    open.set(false);
    harness.frame();
    assert!(harness.app().focused().is_none());
    assert!(
        !harness
            .app()
            .semantics_nodes()
            .iter()
            .any(|node| node.role == Role::Button),
        "the projection dropped the fading node in the frame of its removal"
    );
}

#[test]
fn a_row_fading_out_of_a_list_leaves_its_place_at_once_and_comes_back_in_it() {
    let items = RwSignal::new(vec![1u32, 2, 3]);
    let read = items.clone();
    let mut harness = Harness::new(100.0, 200.0, 1.0, move || {
        let read = read.clone();
        panel().child(
            for_each(
                move || read.get(),
                |item| *item,
                |item| {
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .background(Color::rgb(40 * item as u8, 40, 40))
                        .exit(Spring::SMOOTH)
                },
            )
            .column(),
        )
    });
    harness.frame();
    let third = Point { x: 10.0, y: 50.0 };
    let second = Point { x: 10.0, y: 30.0 };
    assert_eq!(pixel(&harness, third), [120, 40, 40, 255]);

    items.set(vec![1, 3, 4]);
    harness.frame();
    assert_eq!(
        pixel(&harness, Point { x: 10.0, y: 70.0 }),
        [255, 255, 255, 255],
        "nothing below the third row, which moved up"
    );
    assert_ne!(
        pixel(&harness, second),
        [255, 255, 255, 255],
        "the second place shows the third row with the fading row over it"
    );
    harness.assert_matches_full_repaint();

    items.set(vec![1, 2, 3, 4]);
    harness.frame();
    for _ in 0..90 {
        harness.advance(FRAME);
    }
    assert_eq!(pixel(&harness, second), [80, 40, 40, 255]);
    assert_eq!(pixel(&harness, third), [120, 40, 40, 255]);
    assert_eq!(
        pixel(&harness, Point { x: 10.0, y: 70.0 }),
        [160, 40, 40, 255]
    );
    harness.assert_matches_full_repaint();
}

/// A harness with a focusable control above a slot that shows `child`
/// while `open` reads `true`, laid out in a column.
fn beside(open: RwSignal<bool>, child: impl Fn() -> Element<Panel> + 'static) -> Harness {
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let open = open.clone();
        let child = std::rc::Rc::new(child);
        panel()
            .column()
            .child(panel().width(px(20.0)).height(px(20.0)).focusable())
            .child(show(move || open.get(), {
                let child = child.clone();
                move || child()
            }))
    });
    harness.frame();
    harness
}

#[test]
fn a_node_whose_hold_changes_no_layout_still_fades_out() {
    let open = RwSignal::new(true);
    let read = open.clone();
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let read = read.clone();
        panel().child(show(
            move || read.get(),
            || {
                panel()
                    .absolute()
                    .inset(Edges {
                        left: Length::Px(10.0),
                        top: Length::Px(10.0),
                        right: Length::Auto,
                        bottom: Length::Auto,
                    })
                    .width(px(40.0))
                    .height(px(40.0))
                    .background(SQUARE)
                    .exit(Spring::SMOOTH)
            },
        ))
    });
    harness.frame();
    assert_eq!(pixel(&harness, INSIDE), [200, 40, 40, 255]);
    open.set(false);
    for _ in 0..90 {
        harness.advance(FRAME);
    }
    assert_eq!(pixel(&harness, INSIDE), [255, 255, 255, 255]);
    assert!(!harness.layout_dump().contains("[10, 10, 50, 50]"));
    assert!(!harness.app().is_animating());
}

#[test]
fn an_exiting_node_takes_no_keyboard_input() {
    let open = RwSignal::new(true);
    let presses = RwSignal::new(0u32);
    let counted = presses.clone();
    let mut harness = beside(open.clone(), move || {
        let counted = counted.clone();
        panel()
            .width(px(40.0))
            .height(px(40.0))
            .focusable()
            .activate_on_keys()
            .on_press(move || counted.update(|count| *count += 1))
            .exit(Spring::SMOOTH)
    });
    open.set(false);
    harness.frame();
    for _ in 0..4 {
        harness.press_key(Key::Tab);
        harness.press_key(Key::Enter);
        harness.frame();
    }
    assert_eq!(presses.get(), 0, "the fading node heard a key");
}

#[test]
fn a_focus_trap_that_comes_back_keeps_the_keyboard() {
    let open = RwSignal::new(true);
    let mut harness = beside(open.clone(), || {
        panel()
            .trap_focus()
            .exit(Spring::SMOOTH)
            .child(panel().width(px(20.0)).height(px(20.0)).focusable())
    });
    harness.press_key(Key::Tab);
    harness.frame();
    let field = harness.app().focused().expect("Tab focused the field");

    open.set(false);
    harness.advance(FRAME);
    open.set(true);
    harness.advance(FRAME);
    for _ in 0..4 {
        harness.press_key(Key::Tab);
        harness.frame();
        let focused = harness.app().focused();
        assert!(
            focused.is_none() || focused == Some(field),
            "the focus left the trap that came back"
        );
    }
    assert_eq!(harness.app().focused(), Some(field));
}

#[test]
fn a_node_that_comes_back_does_not_keep_the_focus_it_lost() {
    let open = RwSignal::new(true);
    let blurs = RwSignal::new(0u32);
    let counted = blurs.clone();
    let mut harness = beside(open.clone(), move || {
        let counted = counted.clone();
        panel()
            .width(px(40.0))
            .height(px(40.0))
            .background(SQUARE)
            .focusable()
            .focused(|patch| patch.background(Color::rgb(40, 40, 200)))
            .on_focus(move |focused| {
                if !focused {
                    counted.update(|count| *count += 1);
                }
            })
            .exit(Spring::SMOOTH)
    });
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.frame();
    assert!(harness.app().focused().is_some());

    open.set(false);
    harness.frame();
    assert_eq!(blurs.get(), 1, "the exit blurred the node");
    open.set(true);
    for _ in 0..90 {
        harness.advance(FRAME);
    }
    assert!(harness.app().focused().is_none());
    assert_eq!(
        pixel(&harness, Point { x: 10.0, y: 40.0 }),
        [200, 40, 40, 255]
    );
}

#[test]
fn a_node_that_comes_back_takes_the_layout_its_bindings_wrote_meanwhile() {
    let open = RwSignal::new(true);
    let width = RwSignal::new(40.0f32);
    let read = width.clone();
    let mut harness = beside(open.clone(), move || {
        let read = read.clone();
        panel()
            .width(move || px(read.get()))
            .height(px(20.0))
            .background(SQUARE)
            .exit(Spring::SMOOTH)
    });
    open.set(false);
    harness.frame();
    width.set(80.0);
    harness.advance(FRAME);
    assert!(
        harness.layout_dump().contains("[0, 20, 40, 40]"),
        "the fading node holds its size"
    );
    open.set(true);
    for _ in 0..90 {
        harness.advance(FRAME);
    }
    assert!(harness.layout_dump().contains("[0, 20, 80, 40]"));
}

#[test]
fn a_hover_patch_does_not_move_an_exiting_node() {
    let open = RwSignal::new(true);
    let mut harness = beside(open.clone(), || {
        panel()
            .width(px(40.0))
            .height(px(40.0))
            .hover(|patch| patch.margin(px(10.0)))
            .exit(Spring::SMOOTH)
    });
    harness.move_to(Point { x: 20.0, y: 40.0 });
    harness.frame();
    assert!(harness.layout_dump().contains("[10, 30, 50, 70]"));
    open.set(false);
    harness.frame();
    harness.move_to(Point { x: 90.0, y: 90.0 });
    harness.frame();
    assert!(
        harness.layout_dump().contains("[10, 30, 50, 70]"),
        "the node stays where it was when its exit began"
    );
}

#[test]
fn the_first_key_after_an_exit_reaches_no_handler_inside_the_exiting_node() {
    let open = RwSignal::new(true);
    let keys = RwSignal::new(0u32);
    let counted = keys.clone();
    let mut harness = beside(open.clone(), move || {
        let counted = counted.clone();
        panel().exit(Spring::SMOOTH).child(
            panel()
                .on_key(move |_| {
                    counted.update(|count| *count += 1);
                    Handled::No
                })
                .child(panel().width(px(40.0)).height(px(40.0)).focusable()),
        )
    });
    harness.click(Point { x: 20.0, y: 40.0 });
    harness.frame();
    assert!(
        harness.app().focused().is_some(),
        "the click focused the field"
    );
    open.set(false);
    harness.frame();
    harness.press_key(Key::Escape);
    harness.frame();
    assert_eq!(
        keys.get(),
        0,
        "a handler inside the fading node heard a key"
    );
}

#[test]
fn a_press_held_inside_an_exiting_node_ends_with_its_release() {
    let open = RwSignal::new(true);
    let ups = RwSignal::new(0u32);
    let counted = ups.clone();
    let mut harness = beside(open.clone(), move || {
        let counted = counted.clone();
        panel()
            .width(px(40.0))
            .height(px(40.0))
            .on_pointer_up(move |_| counted.update(|count| *count += 1))
            .exit(Spring::SMOOTH)
    });
    harness.button_down(Point { x: 20.0, y: 40.0 });
    harness.frame();
    open.set(false);
    harness.frame();
    assert_eq!(ups.get(), 1, "the pressed node heard its release");
    harness.button_up(Point { x: 20.0, y: 40.0 });
    harness.frame();
    assert_eq!(ups.get(), 1);
}

#[test]
fn an_exiting_node_loses_the_hover_even_when_its_hover_moves_it() {
    let open = RwSignal::new(true);
    let hovers = RwSignal::new(Vec::new());
    let logged = hovers.clone();
    let mut harness = beside(open.clone(), move || {
        let logged = logged.clone();
        panel()
            .width(px(40.0))
            .height(px(40.0))
            .background(SQUARE)
            .hover(|patch| patch.margin(px(2.0)).background(Color::rgb(40, 40, 200)))
            .on_hover(move |hovered| logged.update(|log| log.push(hovered)))
            .exit(Spring::SMOOTH)
    });
    harness.move_to(Point { x: 20.0, y: 40.0 });
    harness.frame();
    assert_eq!(hovers.get(), vec![true]);
    open.set(false);
    harness.frame();
    assert_eq!(hovers.get(), vec![true, false], "the exit ended the hover");
}

#[test]
fn an_ancestor_whose_hover_moves_it_keeps_the_hover_when_a_hovered_child_exits() {
    let open = RwSignal::new(true);
    let outer = RwSignal::new(Vec::new());
    let inner = RwSignal::new(Vec::new());
    let (outer_log, inner_log) = (outer.clone(), inner.clone());
    let read = open.clone();
    let mut harness = Harness::new(100.0, 100.0, 1.0, move || {
        let open = read.clone();
        let (outer_log, inner_log) = (outer_log.clone(), inner_log.clone());
        panel().child(
            panel()
                .width(px(100.0))
                .height(px(40.0))
                .hover(|patch| patch.width(px(20.0)))
                .on_hover(move |hovered| outer_log.update(|log| log.push(hovered)))
                .child(show(
                    move || open.get(),
                    move || {
                        let inner_log = inner_log.clone();
                        panel()
                            .absolute()
                            .inset(Edges {
                                left: Length::Px(40.0),
                                top: Length::Px(0.0),
                                right: Length::Auto,
                                bottom: Length::Auto,
                            })
                            .width(px(50.0))
                            .height(px(40.0))
                            .on_hover(move |hovered| inner_log.update(|log| log.push(hovered)))
                            .exit(Spring::SMOOTH)
                    },
                )),
        )
    });
    harness.frame();
    harness.move_to(Point { x: 70.0, y: 20.0 });
    harness.frame();
    assert_eq!(inner.get(), vec![true]);
    assert_eq!(outer.get(), vec![true]);
    open.set(false);
    for _ in 0..4 {
        harness.advance(FRAME);
    }
    assert_eq!(
        inner.get(),
        vec![true, false],
        "the exit ended the child's hover"
    );
    assert_eq!(
        outer.get(),
        vec![true],
        "the ancestor kept its hover throughout"
    );
}
