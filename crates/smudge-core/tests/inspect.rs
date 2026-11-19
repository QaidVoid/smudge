//! What the devtools read of a running app: picking a node, its boxes, where
//! it was built and the tree with its hidden nodes.

#![cfg(feature = "devtools")]

use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{InputEvent, PointerButton};
use smudge_core::inspect::SourceKind;
use smudge_core::reactive::{RenderEffect, RwSignal, prelude::*};
use smudge_core::style::px;
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
