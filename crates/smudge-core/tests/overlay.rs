//! Overlays: popups that escape the clips around them and draw over the
//! window, through the public API.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::element::{Element, Placement, show};
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{Handled, Key};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, Overflow, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// The color of every popup.
const POPUP: Color = Color::rgb(200, 30, 30);

/// A counter shared with a handler.
type Count = Rc<Cell<u32>>;

/// Returns a counter and a handler that increments it.
fn counter() -> (Count, impl FnMut() + 'static) {
    let count = Rc::new(Cell::new(0));
    let bump = Rc::clone(&count);
    (count, move || bump.set(bump.get() + 1))
}

/// A popup of 80 by 50 in the popup color that counts its presses.
fn popup(presses: impl FnMut() + 'static) -> Element<Panel> {
    panel()
        .width(px(80.0))
        .height(px(50.0))
        .background(POPUP)
        .on_press(presses)
}

/// Places a node at `left`, `top` of its parent.
fn at(left: f32, top: f32) -> Edges<Length> {
    Edges {
        left: Length::px(left),
        top: Length::px(top),
        right: Length::Auto,
        bottom: Length::Auto,
    }
}

/// Returns whether the pixel at `x`, `y` shows the popup.
fn popup_at(harness: &Harness, x: u32, y: u32) -> bool {
    harness.pixel(x, y) == Some([POPUP.r, POPUP.g, POPUP.b, 255])
}

/// A window of 200 by 120 with a 100 by 30 clipping box at 10, 10, whose
/// toggle at 0, 0 of it opens a popup below the toggle. A panel under where
/// the popup opens counts its presses.
fn scene(open: RwSignal<bool>, inside: Count, under: Count) -> Harness {
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, toggle) = (open.clone(), open.clone());
        let (inside, under) = (Rc::clone(&inside), Rc::clone(&under));
        panel()
            .child(
                panel()
                    .absolute()
                    .inset(at(10.0, 60.0))
                    .width(px(100.0))
                    .height(px(40.0))
                    .on_press(move || under.set(under.get() + 1)),
            )
            .child(
                panel()
                    .absolute()
                    .inset(at(10.0, 10.0))
                    .width(px(100.0))
                    .height(px(30.0))
                    .overflow(Overflow::Clip)
                    .child(
                        panel()
                            .width(px(40.0))
                            .height(px(20.0))
                            .on_press(move || toggle.update(|open| *open = !*open))
                            .overlay(shown, Placement::Below, move || {
                                let inside = Rc::clone(&inside);
                                popup(move || inside.set(inside.get() + 1))
                            }),
                    ),
            )
    });
    harness.frame();
    harness
}

#[test]
fn a_popup_draws_outside_the_clip_around_it_and_over_the_window() {
    let open = RwSignal::new(true);
    let (inside, _) = counter();
    let (under, _) = counter();
    let harness = scene(open, inside, under);

    assert!(popup_at(&harness, 20, 35), "inside the clipping box");
    assert!(
        popup_at(&harness, 20, 75),
        "far below the clip, over the panel"
    );
    assert!(!popup_at(&harness, 95, 75), "and no further than its size");
}

#[test]
fn a_press_over_the_popup_reaches_it_and_not_the_node_under_it() {
    let open = RwSignal::new(true);
    let (inside, _) = counter();
    let (under, _) = counter();
    let mut harness = scene(open.clone(), Rc::clone(&inside), Rc::clone(&under));

    harness.click(Point { x: 20.0, y: 70.0 });

    assert_eq!(inside.get(), 1);
    assert_eq!(under.get(), 0);
    assert!(open.get(), "a press inside keeps it open");
}

#[test]
fn a_press_outside_closes_the_popup_and_reaches_what_it_lands_on() {
    let open = RwSignal::new(true);
    let (inside, _) = counter();
    let (under, _) = counter();
    let mut harness = scene(open.clone(), inside, Rc::clone(&under));

    harness.click(Point { x: 100.0, y: 80.0 });
    harness.frame();

    assert!(!open.get());
    assert_eq!(under.get(), 1);
    assert!(!popup_at(&harness, 20, 75), "the popup is gone");
    harness.assert_matches_full_repaint();
}

#[test]
fn the_node_that_opened_a_popup_closes_it_with_its_own_press() {
    let open = RwSignal::new(false);
    let (inside, _) = counter();
    let (under, _) = counter();
    let mut harness = scene(open.clone(), inside, under);
    let toggle = Point { x: 20.0, y: 20.0 };

    harness.click(toggle);
    harness.frame();
    assert!(open.get());
    assert!(popup_at(&harness, 20, 75));
    harness.assert_matches_full_repaint();

    harness.click(toggle);
    harness.frame();
    assert!(!open.get(), "the press on the opener is not outside");
    harness.assert_matches_full_repaint();
}

#[test]
fn escape_closes_the_last_opened_popup() {
    let open = RwSignal::new(true);
    let (inside, _) = counter();
    let (under, _) = counter();
    let mut harness = scene(open.clone(), inside, under);

    harness.press_key(Key::Escape);

    assert!(!open.get());
}

#[test]
fn a_popup_with_no_room_below_opens_above_and_follows_a_scroll() {
    let open = RwSignal::new(true);
    let offset = RwSignal::new(Point::default());
    let scrolled = offset.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, scrolled) = (open.clone(), scrolled.clone());
        panel().child(
            panel()
                .absolute()
                .inset(at(10.0, 0.0))
                .width(px(100.0))
                .height(px(120.0))
                .column()
                .overflow(Overflow::Clip)
                .scroll_offset(move || scrolled.get())
                .child(panel().height(px(80.0)).shrink(0.0))
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .shrink(0.0)
                        .overlay(shown, Placement::Below, || popup(|| {})),
                )
                .child(panel().height(px(80.0)).shrink(0.0)),
        )
    });
    harness.frame();
    let popup = |harness: &Harness| {
        let ink = harness.ink_bounds().expect("only the popup draws");
        (ink.y0, ink.y1)
    };
    assert_eq!(popup(&harness), (30.0, 80.0), "above the node at 80");

    offset.set(Point { x: 0.0, y: 60.0 });
    harness.frame();
    assert_eq!(popup(&harness), (40.0, 90.0), "below the node, now at 20");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_trapping_popup_takes_the_keys_bubbles_them_to_its_opener_and_gives_the_focus_back() {
    let open = RwSignal::new(false);
    let heard: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let focused = Rc::new(Cell::new(false));
    let (shown, toggle, keys, opener_focus) = (
        open.clone(),
        open.clone(),
        Rc::clone(&heard),
        Rc::clone(&focused),
    );
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, toggle, keys) = (shown.clone(), toggle.clone(), Rc::clone(&keys));
        let opener_focus = Rc::clone(&opener_focus);
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .focusable()
                .on_focus(move |on| opener_focus.set(on))
                .on_key(move |event| {
                    if event.pressed && event.key == Key::Character(String::from("x")) {
                        keys.borrow_mut().push("opener");
                        return Handled::Yes;
                    }
                    Handled::No
                })
                .on_press(move || toggle.set(true))
                .overlay(shown, Placement::Below, || {
                    panel().trap_focus().child(
                        panel()
                            .width(px(60.0))
                            .height(px(20.0))
                            .focusable()
                            .autofocus(),
                    )
                }),
        )
    });
    harness.frame();
    harness.click(Point { x: 10.0, y: 10.0 });
    harness.frame();
    assert!(!focused.get(), "the focus moved into the popup");

    harness.press_key(Key::Character(String::from("x")));
    assert_eq!(*heard.borrow(), ["opener"], "keys bubble to the opener");

    harness.press_key(Key::Escape);
    harness.frame();
    assert!(!open.get());
    assert!(focused.get(), "the focus is back on the opener");
}

#[test]
fn removing_the_opener_removes_its_popup() {
    let present = RwSignal::new(true);
    let open = RwSignal::new(true);
    let here = present.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (here, shown) = (here.clone(), open.clone());
        panel().child(show(
            move || here.get(),
            move || {
                panel().width(px(40.0)).height(px(20.0)).overlay(
                    shown.clone(),
                    Placement::Below,
                    || popup(|| {}),
                )
            },
        ))
    });
    harness.frame();
    assert!(popup_at(&harness, 10, 30));

    present.set(false);
    harness.frame();
    assert!(!popup_at(&harness, 10, 30));
    harness.assert_matches_full_repaint();
    let mut nodes = 0;
    harness.app().visit_nodes(|_, _, _| nodes += 1);
    assert_eq!(nodes, 2, "the root and the empty slot");
}

#[test]
fn a_popup_reaches_assistive_technology_after_the_window() {
    let open = RwSignal::new(true);
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        panel().child(panel().width(px(40.0)).height(px(20.0)).overlay(
            open.clone(),
            Placement::Below,
            || {
                panel()
                    .width(px(60.0))
                    .height(px(20.0))
                    .role(smudge_core::semantics::Role::Button)
                    .accessible_label("Inside")
            },
        ))
    });
    harness.frame();

    let nodes = harness.app().semantics_nodes();
    assert!(
        nodes
            .iter()
            .any(|node| node.label.as_deref() == Some("Inside")),
        "{nodes:?}"
    );
}

#[test]
fn a_press_on_a_popups_bare_background_presses_neither_its_opener_nor_what_lies_under_it() {
    let open = RwSignal::new(true);
    let (opener, bump_opener) = counter();
    let (under, bump_under) = counter();
    let bump_opener = Rc::new(RefCell::new(bump_opener));
    let bump_under = Rc::new(RefCell::new(bump_under));
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, opener, under) = (
            open.clone(),
            Rc::clone(&bump_opener),
            Rc::clone(&bump_under),
        );
        panel()
            .child(
                panel()
                    .absolute()
                    .inset(at(0.0, 40.0))
                    .width(px(200.0))
                    .height(px(80.0))
                    .on_press(move || (under.borrow_mut())()),
            )
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .on_press(move || (opener.borrow_mut())())
                    .overlay(shown, Placement::Below, || {
                        panel().width(px(80.0)).height(px(50.0)).background(POPUP)
                    }),
            )
    });
    harness.frame();

    harness.click(Point { x: 20.0, y: 50.0 });

    assert_eq!(opener.get(), 0, "the press does not bubble to the opener");
    assert_eq!(under.get(), 0, "nor fall through the popup");
}

#[test]
fn a_fading_popup_lets_the_pointer_through_to_what_lies_under_it() {
    let open = RwSignal::new(true);
    let (under, bump) = counter();
    let bump = Rc::new(RefCell::new(bump));
    let shown = open.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, bump) = (shown.clone(), Rc::clone(&bump));
        panel()
            .child(
                panel()
                    .absolute()
                    .inset(at(0.0, 30.0))
                    .width(px(200.0))
                    .height(px(90.0))
                    .on_press(move || (bump.borrow_mut())()),
            )
            .child(panel().width(px(40.0)).height(px(20.0)).overlay(
                shown,
                Placement::Below,
                || popup(|| {}).exit(smudge_core::animation::Spring::SMOOTH),
            ))
    });
    harness.frame();

    open.set(false);
    harness.frame();
    harness.click(Point { x: 20.0, y: 50.0 });

    assert_eq!(under.get(), 1, "the fade blocks nothing");
}

#[test]
fn escape_closes_the_popup_the_focus_is_in_before_the_nodes_around_its_opener() {
    let open = RwSignal::new(true);
    let (outer, bump) = counter();
    let bump = Rc::new(RefCell::new(bump));
    let shown = open.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, bump) = (shown.clone(), Rc::clone(&bump));
        panel()
            .on_key(move |event| {
                if event.pressed && event.key == Key::Escape {
                    (bump.borrow_mut())();
                    return Handled::Yes;
                }
                Handled::No
            })
            .child(panel().width(px(40.0)).height(px(20.0)).overlay(
                shown,
                Placement::Below,
                || {
                    panel().trap_focus().child(
                        panel()
                            .width(px(60.0))
                            .height(px(20.0))
                            .focusable()
                            .autofocus(),
                    )
                },
            ))
    });
    harness.frame();

    harness.press_key(Key::Escape);

    assert!(!open.get(), "the popup closed");
    assert_eq!(outer.get(), 0, "and the Escape went no further");
}

#[test]
fn a_popup_flipped_above_its_node_keeps_its_margin_clear_of_it() {
    let open = RwSignal::new(true);
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        panel().child(
            panel()
                .absolute()
                .inset(at(10.0, 90.0))
                .width(px(40.0))
                .height(px(20.0))
                .overlay(open.clone(), Placement::Below, || {
                    popup(|| {}).margin(px(4.0))
                }),
        )
    });
    harness.frame();

    let ink = harness.ink_bounds().expect("the popup draws");
    assert_eq!(
        (ink.y0, ink.y1),
        (36.0, 86.0),
        "4 pixels above the node at 90"
    );
}

#[test]
fn closing_a_popup_under_a_still_pointer_hovers_what_it_uncovered() {
    let open = RwSignal::new(true);
    let hovered = Rc::new(Cell::new(false));
    let (shown, seen) = (open.clone(), Rc::clone(&hovered));
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (shown, seen) = (shown.clone(), Rc::clone(&seen));
        panel()
            .child(
                panel()
                    .absolute()
                    .inset(at(0.0, 30.0))
                    .width(px(200.0))
                    .height(px(90.0))
                    .on_hover(move |inside| seen.set(inside)),
            )
            .child(panel().width(px(40.0)).height(px(20.0)).overlay(
                shown,
                Placement::Below,
                || popup(|| {}),
            ))
    });
    harness.frame();
    harness.move_to(Point { x: 20.0, y: 50.0 });
    assert!(!hovered.get(), "the popup covers it");

    open.set(false);
    harness.frame();

    assert!(hovered.get(), "uncovered under the pointer");
}

#[test]
fn a_dialog_gives_the_focus_back_to_its_opener_or_where_it_was_without_one() {
    use smudge_core::element::Placement;
    let (open, keep) = (RwSignal::new(false), RwSignal::new(true));
    let (opened, kept) = (open.clone(), keep.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let (opened, kept) = (opened.clone(), kept.clone());
        panel()
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .role(smudge_core::semantics::Role::TextInput)
                    .accessible_label("Field")
                    .focusable(),
            )
            .child(show(
                move || kept.get(),
                move || {
                    panel()
                        .width(px(40.0))
                        .height(px(20.0))
                        .role(smudge_core::semantics::Role::Button)
                        .accessible_label("Opener")
                        .focusable()
                        .overlay(opened.clone(), Placement::Below, || {
                            panel()
                                .trap_focus()
                                .width(px(60.0))
                                .height(px(30.0))
                                .role(smudge_core::semantics::Role::Button)
                                .accessible_label("Inside")
                                .focusable()
                                .autofocus()
                        })
                },
            ))
    });
    harness.frame();
    let focused = |harness: &mut Harness| {
        harness
            .app()
            .semantics_nodes()
            .into_iter()
            .find(|node| node.focused)
            .and_then(|node| node.label)
    };
    harness.click(Point { x: 5.0, y: 5.0 });
    open.set(true);
    harness.frame();
    harness.frame();
    assert_eq!(focused(&mut harness).as_deref(), Some("Inside"));
    open.set(false);
    harness.frame();
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Opener"),
        "back to what opened it, though the field held the focus before"
    );

    harness.click(Point { x: 5.0, y: 5.0 });
    open.set(true);
    harness.frame();
    harness.frame();
    assert_eq!(focused(&mut harness).as_deref(), Some("Inside"));
    keep.set(false);
    harness.frame();
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Field"),
        "an opener that went with the dialog leaves it where it was"
    );
}
