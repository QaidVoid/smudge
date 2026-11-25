//! Editor and scroll state after a retained panel changes placement.

use smudge_core::element::{KeepAlive, show};
use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::text_input;
use smudge_widgets::{column, row, scroll};

fn field(harness: &mut Harness) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::TextInput)
        .expect("the field projects")
}

fn scroll_mark(harness: &mut Harness) -> f32 {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some("Scroll marker"))
        .expect("the scroll marker projects")
        .bounds
        .y0
}

fn command() -> Modifiers {
    Modifiers {
        control: cfg!(not(target_os = "macos")),
        meta: cfg!(target_os = "macos"),
        ..Modifiers::default()
    }
}

#[test]
fn moving_a_panel_preserves_scroll_caret_composition_and_undo() {
    let right = RwSignal::new(false);
    let right_slot = right.clone();
    let left_slot = right.clone();
    let text = RwSignal::new(String::new());
    let edit = text.clone();
    let mut harness = Harness::new(240.0, 90.0, 1.0, move || {
        let store = KeepAlive::new(move |_| {
            scroll(
                column()
                    .child(
                        text_input(edit.clone())
                            .width(px(106.0))
                            .height(px(32.0))
                            .shrink(0.0),
                    )
                    .child(
                        panel()
                            .height(px(90.0))
                            .shrink(0.0)
                            .role(Role::Group)
                            .accessible_label("Scroll marker"),
                    ),
            )
            .width(px(120.0))
            .height(px(65.0))
        });
        let other = store.clone();
        row()
            .child(
                column()
                    .width(px(120.0))
                    .shrink(0.0)
                    .child(show(move || !left_slot.get(), move || store.slot("editor"))),
            )
            .child(
                column()
                    .width(px(120.0))
                    .shrink(0.0)
                    .child(show(move || right_slot.get(), move || other.slot("editor"))),
            )
    });
    harness.frame();
    harness.click(Point { x: 15.0, y: 16.0 });
    harness.type_text("hello world");
    assert_eq!(text.get(), "hello world");
    harness.press_key(Key::Home);
    harness.press_key(Key::ArrowRight);
    harness.press_key(Key::ArrowRight);
    let caret = field(&mut harness).text_selection;
    let unscrolled = scroll_mark(&mut harness);
    harness.move_to(Point { x: 75.0, y: 52.0 });
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: 12.0 },
    });
    harness.settle();
    let scrolled = scroll_mark(&mut harness);
    assert!(scrolled < unscrolled);

    harness.event(InputEvent::ImePreedit {
        text: String::from("ka"),
        cursor: Some((2, 2)),
    });
    harness.settle();
    let composing = field(&mut harness);
    let runs = composing
        .text_runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    assert!(runs.contains("ka"));
    let old_area = harness
        .app()
        .ime_area()
        .expect("the composing field has an IME anchor");

    right.set(true);
    harness.settle();
    let moved = field(&mut harness);
    assert_eq!(moved.id, composing.id);
    assert_eq!(harness.app().focused(), Some(composing.id));
    assert_eq!(scroll_mark(&mut harness), scrolled);
    assert_eq!(
        moved
            .text_runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        runs
    );
    assert_eq!(moved.text_selection, composing.text_selection);
    let new_area = harness
        .app()
        .ime_area()
        .expect("the moved field has an IME anchor");
    assert_eq!(new_area.x0 - old_area.x0, 120.0);
    assert_eq!(new_area.y0, old_area.y0);
    harness.assert_matches_full_repaint();

    harness.event(InputEvent::ImeCommit {
        text: String::from("X"),
    });
    harness.settle();
    assert_eq!(text.get(), "heXllo world");
    let undo = Key::Character(String::from("z"));
    harness.press_key_with(undo.clone(), command());
    assert_eq!(text.get(), "hello world");
    assert_eq!(field(&mut harness).text_selection, caret);
    harness.press_key_with(undo, command());
    assert_eq!(text.get(), "hello");
}
