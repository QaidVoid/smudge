//! Moving keyed content between slots through the public element API.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use proptest::prelude::*;
use smudge_core::animation::Spring;
use smudge_core::element::{Element, IntoElement, KeepAlive, Layer, Placement, show};
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{Drag, Handled};
use smudge_core::reactive::{Owner, RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

const LOCATIONS: [f32; 3] = [4.0, 51.5, 101.0];
const PANEL: Color = Color::rgb(20, 70, 160);
const POPUP: Color = Color::rgb(210, 40, 30);

fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

fn host(index: usize, child: impl IntoElement) -> Element<Panel> {
    panel()
        .absolute()
        .inset(Edges {
            left: px(LOCATIONS[index]),
            top: px(4.0),
            right: Length::Auto,
            bottom: Length::Auto,
        })
        .width(px(42.0))
        .height(px(56.0))
        .column()
        .role(Role::Group)
        .accessible_label(format!("Placement {index}"))
        .child(child)
}

fn placements(location: RwSignal<usize>, store: &KeepAlive) -> Element<Panel> {
    panel()
        .role(Role::Group)
        .children((0..LOCATIONS.len()).map(|index| {
            let location = location.clone();
            let store = store.clone();
            host(
                index,
                show(move || location.get() == index, move || store.slot("panel")),
            )
        }))
}

fn named(harness: &mut Harness, name: &str) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some(name))
        .expect("the named node projects")
}

#[test]
fn a_panel_keeps_its_build_state_and_focus_in_both_move_orders() {
    let location = RwSignal::new(0);
    let place = location.clone();
    let builds = Rc::new(Cell::new(0));
    let count = Rc::clone(&builds);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(move |_| {
            count.set(count.get() + 1);
            let value = RwSignal::new(0_u32);
            let pressed = value.clone();
            panel()
                .width(px(36.0))
                .height(px(24.0))
                .background(PANEL)
                .focusable()
                .role(Role::Button)
                .accessible_label("Panel")
                .accessible_value(move || value.get().to_string())
                .on_press(move || pressed.update(|count| *count += 1))
        });
        placements(place, &store)
    });
    harness.frame();
    harness.click(point(10.0, 10.0));
    harness.frame();
    let first = named(&mut harness, "Panel");
    assert_eq!(first.value.as_deref(), Some("1"));
    assert_eq!(harness.app().focused(), Some(first.id));

    for position in [1, 0, 2, 1] {
        let old = named(&mut harness, "Panel")
            .parent
            .expect("the panel has a placement");
        harness.app().take_a11y_changes();
        location.set(position);
        harness.frame();
        let node = named(&mut harness, "Panel");
        let new = named(&mut harness, &format!("Placement {position}")).id;
        assert_eq!(node.parent, Some(new));
        assert_eq!(node.id, first.id);
        assert_eq!(node.value.as_deref(), Some("1"));
        assert_eq!(harness.app().focused(), Some(first.id));
        let changes = harness.app().take_a11y_changes();
        assert!(!changes.reset);
        assert!(changes.changed.contains(&old) && changes.changed.contains(&new));
        harness.assert_matches_full_repaint();
    }
    let bounds = named(&mut harness, "Panel").bounds;
    harness.click(point(bounds.x0 + 5.0, bounds.y0 + 5.0));
    harness.frame();
    assert_eq!(named(&mut harness, "Panel").value.as_deref(), Some("2"));
    assert_eq!(builds.get(), 1);
}

#[test]
fn a_panel_without_a_slot_is_disposed_and_its_key_builds_again() {
    let location = RwSignal::new(0);
    let place = location.clone();
    let builds = Rc::new(Cell::new(0));
    let disposals = Rc::new(Cell::new(0));
    let count = Rc::clone(&builds);
    let cleaned = Rc::clone(&disposals);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(move |_| {
            count.set(count.get() + 1);
            let cleanup = Rc::clone(&cleaned);
            Owner::on_cleanup(move || cleanup.set(cleanup.get() + 1));
            panel()
                .height(px(18.0))
                .role(Role::Group)
                .accessible_label("Panel")
        });
        placements(place, &store)
    });
    harness.frame();
    let old = named(&mut harness, "Panel").id;

    location.set(LOCATIONS.len());
    harness.frame();
    assert!(harness.app().semantics_node(old).is_none());
    assert_eq!(disposals.get(), 1);

    location.set(2);
    harness.frame();
    assert_ne!(named(&mut harness, "Panel").id, old);
    assert_eq!(builds.get(), 2);
    assert_eq!(disposals.get(), 1);
}

#[test]
fn a_held_press_and_focus_follow_a_panel_out_of_a_fading_host() {
    let right = RwSignal::new(false);
    let left_slot = right.clone();
    let right_slot = right.clone();
    let released = Rc::new(Cell::new(None));
    let release_at = Rc::clone(&released);
    let focus_log = Rc::new(RefCell::new(Vec::new()));
    let left_log = Rc::clone(&focus_log);
    let right_log = Rc::clone(&focus_log);
    let hovered = Rc::new(RefCell::new(Vec::new()));
    let left_hover = Rc::clone(&hovered);
    let right_hover = Rc::clone(&hovered);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(move |_| {
            let release_at = Rc::clone(&release_at);
            panel()
                .width(px(36.0))
                .height(px(24.0))
                .background(PANEL)
                .focusable()
                .role(Role::Button)
                .accessible_label("Panel")
                .on_pointer_down(|_| {})
                .on_pointer_up(move |at| release_at.set(Some(at)))
        });
        let moved = store.clone();
        panel()
            .child(show(
                move || !left_slot.get(),
                move || {
                    let left_log = Rc::clone(&left_log);
                    let left_hover = Rc::clone(&left_hover);
                    host(0, store.slot("panel"))
                        .exit(Spring::SMOOTH)
                        .on_focus_within(move |inside| left_log.borrow_mut().push((0, inside)))
                        .on_hover(move |inside| left_hover.borrow_mut().push((0, inside)))
                },
            ))
            .child(show(
                move || right_slot.get(),
                move || {
                    let right_log = Rc::clone(&right_log);
                    let right_hover = Rc::clone(&right_hover);
                    host(1, moved.slot("panel"))
                        .on_focus_within(move |inside| right_log.borrow_mut().push((1, inside)))
                        .on_hover(move |inside| right_hover.borrow_mut().push((1, inside)))
                },
            ))
    });
    harness.frame();
    let id = named(&mut harness, "Panel").id;
    harness.button_down(point(10.0, 10.0));
    assert_eq!(harness.app().focused(), Some(id));

    right.set(true);
    harness.frame();
    assert_eq!(harness.app().focused(), Some(id));
    assert_eq!(released.get(), None);
    assert_eq!(*focus_log.borrow(), [(0, true), (0, false), (1, true)]);
    assert_eq!(*hovered.borrow(), [(0, true), (0, false), (1, true)]);

    let bounds = named(&mut harness, "Panel").bounds;
    let local = point(7.0, 5.0);
    harness.button_up(point(bounds.x0 + local.x, bounds.y0 + local.y));
    assert_eq!(released.get(), Some(local));
    harness.assert_matches_full_repaint();

    right.set(false);
    harness.frame();
    assert_eq!(named(&mut harness, "Panel").id, id);
    assert_eq!(harness.app().focused(), Some(id));
    assert_eq!(named(&mut harness, "Panel").bounds.x0, LOCATIONS[0]);
    harness.assert_matches_full_repaint();
}

#[test]
fn moving_a_panel_ends_only_the_departed_hosts_drag_observation() {
    for remove_origin in [false, true] {
        let location = RwSignal::new(0);
        let at = location.clone();
        let child = RwSignal::new(true);
        let show_child = child.clone();
        let events = Rc::new(RefCell::new(Vec::new()));
        let panel_events = Rc::clone(&events);
        let host_events = Rc::clone(&events);
        let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
            let store = KeepAlive::new(move |_| {
                let show_child = show_child.clone();
                let panel_events = Rc::clone(&panel_events);
                panel()
                    .width(px(36.0))
                    .height(px(24.0))
                    .background(PANEL)
                    .on_pointer_down(|_| {})
                    .on_drag(move |step| {
                        panel_events.borrow_mut().push((1, step));
                        Handled::No
                    })
                    .child(show(
                        move || show_child.get(),
                        || panel().width(px(12.0)).height(px(10.0)).background(POPUP),
                    ))
            });
            let other = store.clone();
            let left = at.clone();
            panel()
                .child(
                    host(
                        0,
                        show(move || left.get() == 0, move || store.slot("panel")),
                    )
                    .on_drag(move |step| {
                        host_events.borrow_mut().push((0, step));
                        Handled::No
                    }),
                )
                .child(host(
                    1,
                    show(move || at.get() == 1, move || other.slot("panel")),
                ))
        });
        harness.frame();
        harness.button_down(point(6.0, 6.0));
        harness.move_to(point(8.0, 8.0));
        assert_eq!(events.borrow().len(), 2);
        if remove_origin {
            child.set(false);
            harness.frame();
        }
        events.borrow_mut().clear();

        location.set(1);
        harness.frame();
        assert_eq!(
            *events.borrow(),
            [(0, Drag::Ended)],
            "remove_origin={remove_origin}"
        );
        events.borrow_mut().clear();
        harness.move_to(point(60.0, 10.0));
        assert_eq!(events.borrow()[0].0, 1);
        assert_eq!(events.borrow().len(), 1);
        harness.button_up(point(60.0, 10.0));
        assert_eq!(events.borrow().last(), Some(&(1, Drag::Ended)));
    }
}

#[test]
fn a_hidden_slot_first_mounted_in_a_fade_can_claim_when_its_host_reopens() {
    let outer = RwSignal::new(true);
    let inner = RwSignal::new(false);
    let hidden = RwSignal::new(true);
    let show_outer = outer.clone();
    let show_inner = inner.clone();
    let hide_slot = hidden.clone();
    let builds = Rc::new(Cell::new(0));
    let count = Rc::clone(&builds);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(move |_| {
            count.set(count.get() + 1);
            painted_panel()
        });
        panel().child(show(
            move || show_outer.get(),
            move || {
                let show_inner = show_inner.clone();
                let store = store.clone();
                let hide_slot = hide_slot.clone();
                panel()
                    .width(px(42.0))
                    .height(px(30.0))
                    .background(POPUP)
                    .exit(Spring::SMOOTH)
                    .child(show(
                        move || show_inner.get(),
                        move || {
                            let hide_slot = hide_slot.clone();
                            store.slot("panel").hidden(move || hide_slot.get())
                        },
                    ))
            },
        ))
    });
    harness.frame();
    outer.set(false);
    harness.frame();
    inner.set(true);
    harness.frame();
    assert_eq!(builds.get(), 0, "the fading host has made no claim");

    outer.set(true);
    harness.frame();
    assert_eq!(builds.get(), 1);
    hidden.set(false);
    harness.frame();
    assert!(named(&mut harness, "Panel").bounds.width() > 0.0);
    assert_eq!(builds.get(), 1);
    harness.assert_matches_full_repaint();
}

#[test]
fn a_nested_slot_claims_only_if_its_parked_panel_reattaches() {
    for (next, expected_parent) in [(1, "Outer"), (3, "Placement 2")] {
        let location = RwSignal::new(0);
        let at = location.clone();
        let nested = RwSignal::new(false);
        let show_nested = nested.clone();
        let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
            let shared = KeepAlive::new(|_| painted_panel().accessible_label("Shared"));
            let for_nested = shared.clone();
            let outer = KeepAlive::new(move |_| {
                let show_nested = show_nested.clone();
                let for_nested = for_nested.clone();
                panel()
                    .role(Role::Group)
                    .accessible_label("Outer")
                    .child(show(
                        move || show_nested.get(),
                        move || for_nested.slot("shared"),
                    ))
            });
            let right = outer.clone();
            let left_at = at.clone();
            panel()
                .child(host(
                    0,
                    show(move || left_at.get() == 0, move || outer.slot("outer")),
                ))
                .child(host(
                    1,
                    show(move || at.get() == 1, move || right.slot("outer")),
                ))
                .child(host(2, shared.slot("shared")))
        });
        harness.frame();
        let id = named(&mut harness, "Shared").id;
        assert_eq!(
            named(&mut harness, "Shared").parent,
            Some(named(&mut harness, "Placement 2").id)
        );

        nested.set(true);
        location.set(next);
        harness.frame();
        let shared = named(&mut harness, "Shared");
        assert_eq!(shared.id, id);
        assert_eq!(shared.parent, Some(named(&mut harness, expected_parent).id));
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn a_pending_nested_slot_claims_when_its_panel_moves_out_of_a_fade() {
    let left = RwSignal::new(true);
    let right = RwSignal::new(false);
    let nested = RwSignal::new(false);
    let show_left = left.clone();
    let show_right = right.clone();
    let show_nested = nested.clone();
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let shared = KeepAlive::new(|_| painted_panel().accessible_label("Shared"));
        let for_nested = shared.clone();
        let outer = KeepAlive::new(move |_| {
            let show_nested = show_nested.clone();
            let for_nested = for_nested.clone();
            panel()
                .role(Role::Group)
                .accessible_label("Outer")
                .child(show(
                    move || show_nested.get(),
                    move || for_nested.slot("shared"),
                ))
        });
        let other = outer.clone();
        panel()
            .child(show(
                move || show_left.get(),
                move || host(0, outer.slot("outer")).exit(Spring::SMOOTH),
            ))
            .child(host(
                1,
                show(move || show_right.get(), move || other.slot("outer")),
            ))
            .child(host(2, shared.slot("shared")))
    });
    harness.frame();
    let id = named(&mut harness, "Shared").id;

    left.set(false);
    harness.frame();
    nested.set(true);
    harness.frame();
    assert_eq!(
        named(&mut harness, "Shared").parent,
        Some(named(&mut harness, "Placement 2").id)
    );

    right.set(true);
    harness.frame();
    let shared = named(&mut harness, "Shared");
    assert_eq!(shared.id, id);
    assert_eq!(shared.parent, Some(named(&mut harness, "Outer").id));
    harness.assert_matches_full_repaint();
}

#[test]
fn an_uncaptured_move_does_not_hover_the_destination_away_from_the_pointer() {
    let right = RwSignal::new(false);
    let left_slot = right.clone();
    let right_slot = right.clone();
    let destination_events = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::clone(&destination_events);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(|_| painted_panel().on_hover(|_| {}));
        let other = store.clone();
        panel()
            .child(host(
                0,
                show(move || !left_slot.get(), move || store.slot("panel")),
            ))
            .child(
                host(
                    2,
                    show(move || right_slot.get(), move || other.slot("panel")),
                )
                .on_hover(move |inside| events.borrow_mut().push(inside)),
            )
    });
    harness.frame();
    harness.move_to(point(10.0, 10.0));

    right.set(true);
    harness.frame();
    assert_eq!(named(&mut harness, "Panel").bounds.x0, LOCATIONS[2]);
    assert!(destination_events.borrow().is_empty());

    harness.move_to(point(LOCATIONS[2] + 4.0, 10.0));
    assert_eq!(*destination_events.borrow(), [true]);
}

#[test]
fn a_layout_changing_hover_stays_stable_across_moves_under_a_resting_pointer() {
    let right = RwSignal::new(false);
    let left_slot = right.clone();
    let right_slot = right.clone();
    let events = Rc::new(RefCell::new(Vec::new()));
    let left_events = Rc::clone(&events);
    let right_events = Rc::clone(&events);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(|_| {
            panel()
                .width(px(36.0))
                .height(px(24.0))
                .background(PANEL)
                .hover(|patch| patch.width(px(12.0)))
                .role(Role::Group)
                .accessible_label("Panel")
        });
        let other = store.clone();
        panel()
            .child(show(
                move || !left_slot.get(),
                move || {
                    let events = Rc::clone(&left_events);
                    host(0, store.slot("panel"))
                        .on_hover(move |inside| events.borrow_mut().push((false, inside)))
                },
            ))
            .child(show(
                move || right_slot.get(),
                move || {
                    let events = Rc::clone(&right_events);
                    host(0, other.slot("panel"))
                        .on_hover(move |inside| events.borrow_mut().push((true, inside)))
                },
            ))
    });
    harness.frame();
    harness.move_to(point(30.0, 10.0));
    harness.frame();
    assert_eq!(named(&mut harness, "Panel").bounds.width(), 12.0);
    assert_eq!(*events.borrow(), [(false, true)]);

    for destination in [true, false, true, false] {
        events.borrow_mut().clear();
        right.set(destination);
        harness.frame();
        assert_eq!(named(&mut harness, "Panel").bounds.width(), 12.0);
        assert_eq!(*events.borrow(), [(destination, true)]);
        harness.assert_matches_full_repaint();
    }

    harness.move_to(point(145.0, 65.0));
    harness.frame();
    assert_eq!(named(&mut harness, "Panel").bounds.width(), 36.0);
}

#[test]
fn closing_an_unrelated_overlay_during_a_move_keeps_continuous_hover() {
    let right = RwSignal::new(false);
    let left_slot = right.clone();
    let right_slot = right.clone();
    let open = RwSignal::new(true);
    let popup_open = open.clone();
    let log = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::clone(&log);
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(move |_| {
            let events = Rc::clone(&events);
            painted_panel().on_hover(move |inside| events.borrow_mut().push(inside))
        });
        let other = store.clone();
        panel()
            .child(show(
                move || !left_slot.get(),
                move || host(0, store.slot("panel")),
            ))
            .child(show(
                move || right_slot.get(),
                move || host(0, other.slot("panel")),
            ))
            .child(host(
                2,
                panel().overlay(popup_open, Placement::Below, || {
                    panel()
                        .width(px(20.0))
                        .height(px(10.0))
                        .background(POPUP)
                        .role(Role::Group)
                        .accessible_label("Unrelated popup")
                }),
            ))
    });
    harness.frame();
    let popup = named(&mut harness, "Unrelated popup").id;
    harness.move_to(point(10.0, 10.0));
    assert_eq!(*log.borrow(), [true]);

    right.set(true);
    open.set(false);
    harness.frame();
    assert!(harness.app().semantics_node(popup).is_none());
    assert_eq!(*log.borrow(), [true]);
}

#[test]
fn a_layout_triggered_hover_that_hides_a_retained_panel_drops_focus() {
    let left = RwSignal::new(4.0);
    let position = left.clone();
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(|_| painted_panel().focusable());
        panel().child(
            panel()
                .absolute()
                .inset(move || Edges {
                    left: px(position.get()),
                    top: px(4.0),
                    right: Length::Auto,
                    bottom: Length::Auto,
                })
                .width(px(38.0))
                .height(px(30.0))
                .hover(|style| style.hidden(true))
                .child(store.slot("panel")),
        )
    });
    harness.frame();
    let id = named(&mut harness, "Panel").id;
    assert!(harness.app().request_focus(id));
    harness.move_to(point(90.0, 12.0));
    assert_eq!(harness.app().focused(), Some(id));

    left.set(80.0);
    harness.frame();
    assert!(harness.app().semantics_node(id).is_none());
    assert_eq!(harness.app().focused(), None);
}

#[test]
fn an_open_overlay_keeps_its_id_and_moves_with_the_host() {
    let location = RwSignal::new(0);
    let place = location.clone();
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(|_| {
            let open = RwSignal::new(true);
            panel()
                .height(px(20.0))
                .overlay(open, Placement::Below, || {
                    panel()
                        .width(px(20.0))
                        .height(px(10.0))
                        .background(POPUP)
                        .role(Role::Group)
                        .accessible_label("Popup")
                })
        });
        placements(place, &store)
    });
    harness.frame();
    let old = named(&mut harness, "Popup");
    assert_eq!(
        harness.pixel(10, 28),
        Some([POPUP.r, POPUP.g, POPUP.b, 255])
    );

    location.set(2);
    harness.frame();
    let moved = named(&mut harness, "Popup");
    assert_eq!(moved.id, old.id);
    assert!(moved.bounds.x0 > old.bounds.x0);
    assert_eq!(
        harness.pixel(110, 28),
        Some([POPUP.r, POPUP.g, POPUP.b, 255])
    );
    assert_ne!(
        harness.pixel(10, 28),
        Some([POPUP.r, POPUP.g, POPUP.b, 255])
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_moved_popup_joins_and_leaves_its_destinations_dialog_layer() {
    let right = RwSignal::new(false);
    let left_slot = right.clone();
    let right_slot = right.clone();
    let mut harness = Harness::new(150.0, 72.0, 1.0, move || {
        let store = KeepAlive::new(|_| {
            panel()
                .height(px(20.0))
                .overlay(RwSignal::new(true), Placement::Below, || {
                    panel()
                        .width(px(20.0))
                        .height(px(10.0))
                        .background(POPUP)
                        .role(Role::Group)
                        .accessible_label("Popup")
                })
        });
        let other = store.clone();
        panel()
            .child(host(
                0,
                show(move || !left_slot.get(), move || store.slot("panel")),
            ))
            .child(panel().overlay_in(
                Layer::Dialog,
                RwSignal::new(true),
                Placement::Cover,
                move || {
                    let right_slot = right_slot.clone();
                    let other = other.clone();
                    panel()
                        .width(px(150.0))
                        .height(px(72.0))
                        .background(PANEL)
                        .child(host(
                            1,
                            show(move || right_slot.get(), move || other.slot("panel")),
                        ))
                },
            ))
    });
    harness.frame();
    assert_eq!(
        harness.pixel(10, 28),
        Some([PANEL.r, PANEL.g, PANEL.b, 255])
    );

    right.set(true);
    harness.frame();
    let id = named(&mut harness, "Popup").id;
    assert_eq!(
        harness.pixel(60, 28),
        Some([POPUP.r, POPUP.g, POPUP.b, 255])
    );
    harness.assert_matches_full_repaint();

    right.set(false);
    harness.frame();
    assert_eq!(
        harness.pixel(10, 28),
        Some([PANEL.r, PANEL.g, PANEL.b, 255])
    );
    harness.assert_matches_full_repaint();

    right.set(true);
    harness.frame();
    assert_eq!(named(&mut harness, "Popup").id, id);
    harness.assert_matches_full_repaint();
}

fn painted_panel() -> Element<Panel> {
    panel()
        .height(px(20.0))
        .background(PANEL)
        .role(Role::Group)
        .accessible_label("Panel")
        .child(
            panel()
                .width(px(9.5))
                .height(px(8.0))
                .background(POPUP)
                .role(Role::Group)
                .accessible_label("Child"),
        )
}

fn fresh_placement(at: usize) -> Harness {
    let mut harness = Harness::new(150.0, 72.0, 1.25, move || {
        panel()
            .role(Role::Group)
            .children((0..LOCATIONS.len()).map(|index| {
                host(
                    index,
                    panel().child(if index == at {
                        painted_panel()
                    } else {
                        panel()
                    }),
                )
            }))
    });
    harness.frame();
    harness
}

proptest! {
    #[test]
    fn moving_matches_fresh_paint_and_accessibility_after_every_placement(
        locations in prop::collection::vec(0usize..LOCATIONS.len(), 1..16),
    ) {
        let location = RwSignal::new(0);
        let place = location.clone();
        let mut moved = Harness::new(150.0, 72.0, 1.25, move || {
            let store = KeepAlive::new(|_| painted_panel());
            placements(place, &store)
        });
        moved.frame();
        let id = named(&mut moved, "Panel").id;

        for at in locations {
            location.set(at);
            moved.frame();
            moved.assert_matches_full_repaint();
            let mut fresh = fresh_placement(at);
            moved.assert_matches(&fresh);

            let mut actual = moved.app().semantics_nodes();
            let expected = fresh.app().semantics_nodes();
            prop_assert_eq!(actual.len(), expected.len());
            prop_assert_eq!(named(&mut moved, "Panel").id, id);
            let ids: HashMap<_, _> = actual.iter().zip(&expected).map(|(moved, fresh)| (moved.id, fresh.id)).collect();
            for node in &mut actual {
                node.id = ids[&node.id];
                node.parent = node.parent.map(|parent| ids[&parent]);
                for child in &mut node.children {
                    *child = ids[child];
                }
            }
            prop_assert_eq!(actual, expected);
        }
    }
}
