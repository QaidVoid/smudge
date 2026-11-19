//! What the devtools read of a running app: picking a node, its boxes, where
//! it was built and the tree with its hidden nodes.

#![cfg(feature = "devtools")]

use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{InputEvent, PointerButton};
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
