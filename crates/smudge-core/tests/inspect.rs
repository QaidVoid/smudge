//! What the devtools read of a running app: picking a node, its boxes, where
//! it was built and the tree with its hidden nodes.

#![cfg(feature = "devtools")]

use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{InputEvent, PointerButton};
use smudge_core::inspect::{A11yWarningKind, SourceKind};
use smudge_core::reactive::{RenderEffect, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::px;
use smudge_core::tree::WidgetId;
use smudge_core::widget::panel;
use smudge_test::Harness;

#[test]
fn picking_a_point_finds_the_topmost_node_with_its_four_boxes() {
    let line = line!() + 3;
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(
            panel()
                .width(px(60.0))
                .height(px(40.0))
                .margin(px(5.0))
                .border(2.0)
                .padding(px(3.0)),
        )
    });
    harness.frame();

    let picked = harness
        .app()
        .node_at(Point { x: 30.0, y: 20.0 })
        .expect("the child is there");
    let details = harness
        .app()
        .node_details(picked)
        .expect("the node is live");
    assert!(details.type_name.ends_with("Panel"));
    assert_eq!(details.boxes.border, Rect::new(5.0, 5.0, 65.0, 45.0));
    assert_eq!(details.boxes.margin, Rect::new(0.0, 0.0, 70.0, 50.0));
    assert_eq!(details.boxes.padding, Rect::new(7.0, 7.0, 63.0, 43.0));
    assert_eq!(details.boxes.content, Rect::new(10.0, 10.0, 60.0, 40.0));
    let at = details.created_at.expect("an element built it");
    assert_eq!(
        (at.file(), at.line()),
        (file!(), line),
        "the call in this test"
    );

    let outside = harness
        .app()
        .node_at(Point { x: 150.0, y: 80.0 })
        .expect("the root is there");
    assert_ne!(outside, picked, "outside the child, the root is topmost");
}

#[test]
fn the_details_follow_the_nodes_states() {
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(panel().width(px(60.0)).height(px(40.0)).on_press(|| {}))
    });
    harness.frame();
    let inside = Point { x: 30.0, y: 20.0 };
    let picked = harness.app().node_at(inside).expect("the child is there");
    harness
        .app()
        .handle_event(InputEvent::PointerMoved { position: inside });
    harness.app().handle_event(InputEvent::PointerDown {
        position: inside,
        button: PointerButton::Primary,
    });
    let states = harness
        .app()
        .node_details(picked)
        .expect("the node is live")
        .states;
    assert!(states.hovered && states.pressed && !states.disabled);
}

#[test]
fn the_tree_lists_hidden_nodes_and_marks_them() {
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        panel()
            .child(panel().hidden(true).child(panel()))
            .child(panel())
    });
    harness.frame();
    let mut entries = Vec::new();
    harness
        .app()
        .visit_tree(|entry| entries.push((entry.depth, entry.hidden)));
    assert_eq!(
        entries,
        [(0, false), (1, true), (2, true), (1, false)],
        "a hidden node and its subtree are listed, marked hidden"
    );
}

#[test]
fn a_binding_names_the_signal_it_read_and_the_write_that_woke_it() {
    let defined = line!() + 3;
    let written = line!() + 9;
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        let width = RwSignal::new(20.0_f32);
        let (read, write) = (width.clone(), width);
        panel()
            .child(
                panel()
                    .width(px(60.0))
                    .height(px(40.0))
                    .on_press(move || write.set(90.0)),
            )
            .child(panel().width(move || px(read.get())).height(px(10.0)))
    });
    harness.frame();
    harness.click(Point { x: 30.0, y: 20.0 });
    harness.frame();

    let bound = harness
        .app()
        .node_at(Point { x: 65.0, y: 5.0 })
        .expect("the bound node is there");
    let details = harness.app().node_details(bound).expect("the node is live");
    let binding = details
        .bindings
        .iter()
        .find(|binding| binding.woken_by.is_some())
        .expect("a write woke the width binding");
    let source = binding.sources.first().expect("it read the signal");
    assert_eq!(source.kind, SourceKind::Signal);
    let at = source.defined_at.expect("the signal's place is known");
    assert_eq!((at.file(), at.line()), (file!(), defined));
    let wake = binding.woken_by.expect("checked above");
    assert_eq!(
        (wake.written_at.file(), wake.written_at.line()),
        (file!(), written),
        "the press handler's write"
    );
    assert_eq!(wake.source_defined_at, Some(at));
}

#[test]
fn a_write_cycle_is_reported_with_both_places() {
    let created = line!() + 5;
    let written = line!() + 6;
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        let count = RwSignal::new(0_u32);
        let feed = count.clone();
        std::mem::forget(RenderEffect::new(move || {
            let next = feed.get() + 1;
            feed.set(next);
        }));
        panel().on_press(move || count.set(1))
    });
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.frame();
        harness.click(Point { x: 10.0, y: 10.0 });
        harness.frame();
    }));
    let cycle = harness
        .app()
        .cycles()
        .into_iter()
        .last()
        .expect("the cycle is recorded");
    let effect = cycle
        .effect_defined_at
        .expect("the effect's place is known");
    assert_eq!((effect.file(), effect.line()), (file!(), created));
    let write = cycle.written_at.expect("the write's place is known");
    assert_eq!((write.file(), write.line()), (file!(), written));
}

#[test]
fn the_busiest_effects_and_the_owner_tree_count_the_apps_own() {
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        let count = RwSignal::new(0_u32);
        let (seen, press) = (count.clone(), count);
        std::mem::forget(RenderEffect::new(move || {
            seen.get();
        }));
        panel().on_press(move || press.update(|count| *count += 1))
    });
    harness.frame();
    for _ in 0..3 {
        harness.click(Point { x: 10.0, y: 10.0 });
        harness.frame();
    }
    let busiest = harness.app().busiest_effects(1);
    assert_eq!(busiest.len(), 1);
    assert!(
        busiest[0].runs >= 4,
        "created once and woken three times: {busiest:?}"
    );
    assert!(busiest[0].defined_at.is_some());

    let (mut signals, mut effects) = (0, 0);
    harness.app().visit_owners(|owner| {
        signals += owner.signals;
        effects += owner.effects;
    });
    assert!(
        signals >= 1 && effects >= 1,
        "{signals} signals, {effects} effects"
    );
    let (mut more_signals, mut more_effects) = (0, 0);
    harness.app().remount(|| {
        let count = RwSignal::new(0_u32);
        let _ = RwSignal::new(0_u32);
        let (seen, press) = (count.clone(), count);
        std::mem::forget(RenderEffect::new(move || {
            seen.get();
        }));
        panel().on_press(move || press.update(|count| *count += 1))
    });
    harness.app().visit_owners(|owner| {
        more_signals += owner.signals;
        more_effects += owner.effects;
    });
    assert_eq!(
        (more_signals, more_effects),
        (signals + 1, effects),
        "the same build with one more signal counts one more signal"
    );
}

#[test]
fn updates_that_never_settle_are_reported_with_the_last_write() {
    let first = line!() + 9;
    let second = line!() + 15;
    let (flag, set_flag) = smudge_core::reactive::signal_local(true);
    let (first_read, first_write, second_write) = (flag.clone(), set_flag.clone(), set_flag);
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .child(smudge_core::element::show(
                move || first_read.get(),
                move || {
                    first_write.set(false);
                    panel()
                },
            ))
            .child(smudge_core::element::show(
                move || !flag.get(),
                move || {
                    second_write.set(true);
                    panel()
                },
            ))
    });
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| harness.frame()));
    let cycle = harness
        .app()
        .cycles()
        .into_iter()
        .last()
        .expect("the frame's cycle is recorded");
    assert_eq!(
        cycle.effect_defined_at, None,
        "no single effect ran too often"
    );
    let write = cycle.written_at.expect("the last write's place is known");
    assert!(
        write.file() == file!() && [first, second].contains(&write.line()),
        "{write:?}"
    );
}

#[test]
fn the_tab_order_is_the_order_tab_moves_the_focus_in() {
    let mut harness = Harness::new(300.0, 100.0, 1.0, || {
        panel()
            .child(panel().width(px(40.0)).height(px(20.0)).focusable())
            .child(panel().hidden(true).child(panel().focusable()))
            .child(
                panel()
                    .child(panel().width(px(40.0)).height(px(20.0)).focusable())
                    .child(panel().width(px(40.0)).height(px(20.0)).focusable()),
            )
    });
    harness.frame();
    let order: Vec<WidgetId> = harness
        .app()
        .focus_info()
        .order
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(order.len(), 3, "the hidden node is left out");
    let mut visited = Vec::new();
    for _ in 0..order.len() {
        harness.press_key(smudge_core::input::Key::Tab);
        harness.frame();
        visited.extend(harness.app().focused());
    }
    assert_eq!(visited, order);
    assert!(
        harness.app().focus_info().keyboard,
        "Tab turns keyboard navigation on"
    );
}

#[test]
fn an_unnamed_icon_button_and_an_unprojected_pressable_node_are_warned_about() {
    let mut harness = Harness::new(300.0, 100.0, 1.0, || {
        panel()
            .child(
                panel()
                    .width(px(30.0))
                    .height(px(30.0))
                    .role(Role::Button)
                    .focusable()
                    .on_press(|| {}),
            )
            .child(panel().width(px(30.0)).height(px(30.0)).on_press(|| {}))
            .child(
                panel()
                    .width(px(30.0))
                    .height(px(30.0))
                    .role(Role::Button)
                    .accessible_label("Save")
                    .focusable()
                    .on_press(|| {}),
            )
    });
    harness.frame();
    let kinds: Vec<(A11yWarningKind, f32)> = harness
        .app()
        .accessibility_warnings()
        .into_iter()
        .map(|warning| (warning.kind, warning.rect.x0))
        .collect();
    assert_eq!(
        kinds,
        [
            (A11yWarningKind::FocusableWithoutName, 0.0),
            (A11yWarningKind::PressableWithoutRole, 30.0),
        ],
        "the named button is fine"
    );
}

#[test]
fn an_unnamed_password_field_is_warned_about_as_a_text_input_is() {
    let mut harness = Harness::new(300.0, 100.0, 1.0, || {
        panel().child(
            panel()
                .width(px(30.0))
                .height(px(30.0))
                .role(Role::PasswordInput)
                .accessible_value("\u{2022}")
                .focusable(),
        )
    });
    harness.frame();
    let kinds: Vec<A11yWarningKind> = harness
        .app()
        .accessibility_warnings()
        .into_iter()
        .map(|warning| warning.kind)
        .collect();
    assert_eq!(kinds, [A11yWarningKind::TextInputWithoutName]);
}

#[test]
fn the_pointer_view_follows_hover_and_a_held_press() {
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(panel().width(px(60.0)).height(px(40.0)).on_press(|| {}))
    });
    harness.frame();
    let inside = Point { x: 30.0, y: 20.0 };
    harness
        .app()
        .handle_event(InputEvent::PointerMoved { position: inside });
    harness.app().handle_event(InputEvent::PointerDown {
        position: inside,
        button: PointerButton::Primary,
    });
    let pointer = harness.app().pointer_info();
    let child = Rect::new(0.0, 0.0, 60.0, 40.0);
    assert_eq!(pointer.hovered.first(), Some(&child), "deepest first");
    assert_eq!(pointer.hovered.len(), 2, "the child and the root");
    assert_eq!(pointer.pressed, Some(child));
    assert_eq!(pointer.pressable, [child]);
}

#[test]
fn the_owner_walk_lets_its_callback_use_signals() {
    let mut harness = Harness::new(100.0, 50.0, 1.0, || {
        let _ = RwSignal::new(1_u8);
        panel()
    });
    harness.frame();
    let count = RwSignal::new(0_usize);
    let mut made = Vec::new();
    harness.app().visit_owners(|owner| {
        count.update(|count| *count += owner.signals);
        made.push(RwSignal::new(owner.depth));
    });
    assert!(count.get() >= 1, "the build's signal is counted");
    assert!(!made.is_empty());
}

#[test]
fn a_cycle_is_reported_by_the_app_it_happened_in() {
    let mut quiet = Harness::new(100.0, 50.0, 1.0, panel);
    quiet.frame();
    let mut cycling = Harness::new(100.0, 50.0, 1.0, || {
        let count = RwSignal::new(0_u32);
        let feed = count.clone();
        std::mem::forget(RenderEffect::new(move || {
            let next = feed.get() + 1;
            feed.set(next);
        }));
        panel().on_press(move || count.set(1))
    });
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cycling.frame();
        cycling.click(Point { x: 10.0, y: 10.0 });
        cycling.frame();
    }));
    assert!(!cycling.app().cycles().is_empty());
    assert!(quiet.app().cycles().is_empty(), "the other app had none");
}
