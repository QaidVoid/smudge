//! Whole-frame behavior of a selection scope over labels, in a plain column,
//! in a scroll view and in a keyed virtual list, driven through the headless
//! harness with the bundled fonts.
//!
//! These tests live with the widgets because they need the scroll view and
//! the virtual list, which depend on the text crate.

use std::rc::Rc;
use std::time::Duration;

use smudge_core::clipboard::{Clipboard, MemoryClipboard};
use smudge_core::element::show;
use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_test::Harness;
use smudge_text::{
    LabelExt, SelectableExt, Span, SpanStyle, TextSystem, label, rich_text, selection_scope,
};
use smudge_widgets::{ScrollExt, column, scroll, virtual_list_keyed};

/// The height of every block, in logical pixels.
const ROW: f32 = 24.0;

/// The text of the block of `key`.
fn text(key: u32) -> String {
    format!("line {key} of the transcript")
}

/// The modifiers of the copy shortcut on this platform.
fn command() -> Modifiers {
    Modifiers {
        control: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Modifiers::default()
    }
}

/// Copies the selection with the shortcut and returns what the clipboard
/// holds.
fn copy(harness: &mut Harness, clipboard: &mut MemoryClipboard) -> Option<String> {
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    clipboard.get()
}

/// The point at `x` on the block of the visible row `row`, counted from the
/// top of the window.
fn on_row(row: u32, x: f32) -> Point {
    Point {
        x,
        y: row as f32 * ROW + ROW / 2.0,
    }
}

/// Builds a scope over `count` labels in a plain column, with a memory
/// clipboard installed.
fn column_of(count: u32) -> (Harness, MemoryClipboard) {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        selection_scope(
            move || (0..count).collect(),
            |key: &u32| text(*key),
            move || {
                column().children(
                    (0..count).map(|key| label(text(key)).height(px(ROW)).selectable(key)),
                )
            },
        )
    });
    let clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();
    (harness, clipboard)
}

#[test]
fn a_drag_from_one_block_to_another_selects_and_copies_the_blocks_between() {
    let (mut harness, mut clipboard) = column_of(5);

    harness.button_down(on_row(1, 1.0));
    harness.move_to(on_row(2, 200.0));
    harness.move_to(on_row(3, 390.0));
    harness.button_up(on_row(3, 390.0));
    harness.frame();
    harness.assert_matches_full_repaint();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}\n{}", text(1), text(2), text(3)).as_str()),
    );
}

#[test]
fn a_drag_up_selects_the_same_text_as_a_drag_down() {
    let (mut harness, mut clipboard) = column_of(5);

    harness.button_down(on_row(3, 390.0));
    harness.move_to(on_row(1, 1.0));
    harness.button_up(on_row(1, 1.0));
    harness.frame();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}\n{}", text(1), text(2), text(3)).as_str()),
    );
}

#[test]
fn a_double_click_selects_the_word_under_the_pointer() {
    let (mut harness, mut clipboard) = column_of(3);

    harness.click(on_row(0, 2.0));
    harness.click(on_row(0, 2.0));
    harness.frame();

    assert_eq!(copy(&mut harness, &mut clipboard).as_deref(), Some("line"));
}

#[test]
fn a_press_beside_the_blocks_clears_the_selection() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        selection_scope(
            || (0..2u32).collect(),
            |key: &u32| text(*key),
            || {
                column()
                    .child(label(text(0)).height(px(ROW)).selectable(0u32))
                    .child(label(text(1)).height(px(ROW)).selectable(1u32))
                    .child(column().height(px(200.0)))
            },
        )
    });
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();

    harness.button_down(on_row(0, 1.0));
    harness.button_up(on_row(1, 390.0));
    harness.frame();
    clipboard.set(String::from("before"));
    harness.click(Point { x: 50.0, y: 250.0 });
    harness.frame();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("before"),
        "nothing was selected to copy"
    );
}

#[test]
fn a_drag_held_below_a_scroll_view_scrolls_and_extends_the_selection() {
    TextSystem::bundled_only().install();
    let offset = RwSignal::new(0.0f32);
    let bound = offset.clone();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let bound = bound.clone();
        selection_scope(
            || (0..40u32).collect(),
            |key: &u32| text(*key),
            move || {
                scroll(column().children(
                    (0..40u32).map(|key| label(text(key)).height(px(ROW)).selectable(key)),
                ))
                .offset(bound.clone())
                .height(px(5.0 * ROW))
            },
        )
    });
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();
    harness.settle();

    harness.button_down(on_row(0, 1.0));
    harness.move_to(Point { x: 200.0, y: 250.0 });
    for _ in 0..30 {
        harness.advance(Duration::from_millis(16));
    }
    harness.assert_matches_full_repaint();
    let scrolled = offset.get();
    assert!(
        scrolled > 2.0 * ROW,
        "the view scrolled under the held drag"
    );

    harness.move_to(on_row(2, 200.0));
    for _ in 0..5 {
        harness.advance(Duration::from_millis(16));
    }
    let returned = offset.get();
    assert_eq!(returned, scrolled, "the pointer came back, so it stopped");

    harness.move_to(Point { x: 200.0, y: 250.0 });
    harness.advance(Duration::from_millis(16));
    harness.button_up(Point { x: 200.0, y: 250.0 });
    let released = offset.get();
    for _ in 0..5 {
        harness.advance(Duration::from_millis(16));
    }
    assert_eq!(offset.get(), released, "the release stopped the scroll");

    let copied = copy(&mut harness, &mut clipboard).unwrap_or_default();
    let lines = copied.lines().count();
    assert!(
        lines > 6,
        "the selection followed the rows that scrolled past the edge: {copied:?}"
    );
    assert!(copied.starts_with(&text(0)), "from where the press began");
}

#[test]
fn a_selection_survives_its_blocks_leaving_a_virtual_list_and_coming_back() {
    TextSystem::bundled_only().install();
    let items: Rc<[u32]> = (0..200).collect();
    let keys = Rc::clone(&items);
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let items = Rc::clone(&items);
        let keys = Rc::clone(&keys);
        selection_scope(
            move || Rc::clone(&keys),
            |key: &u32| text(*key),
            move || {
                let items = Rc::clone(&items);
                virtual_list_keyed(
                    move || Rc::clone(&items),
                    |key| *key,
                    ROW,
                    |key| label(text(*key)).height(px(ROW)).selectable(*key),
                )
                .height(px(10.0 * ROW))
            },
        )
    });
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();
    harness.settle();
    let unselected = harness.pixel(4, (2.0 * ROW + ROW / 2.0) as u32);

    harness.button_down(on_row(1, 1.0));
    harness.move_to(on_row(3, 390.0));
    harness.button_up(on_row(3, 390.0));
    harness.frame();
    let highlighted = harness.pixel(4, (2.0 * ROW + ROW / 2.0) as u32);
    assert_ne!(highlighted, unselected, "the selected row is highlighted");

    // Far enough down that the selected rows unmount.
    for _ in 0..10 {
        harness.event(InputEvent::Wheel {
            delta: Point { x: 0.0, y: 400.0 },
        });
        harness.frame();
    }
    harness.settle();
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}\n{}", text(1), text(2), text(3)).as_str()),
        "copy reads the blocks that are not mounted"
    );

    for _ in 0..12 {
        harness.event(InputEvent::Wheel {
            delta: Point { x: 0.0, y: -400.0 },
        });
        harness.frame();
    }
    harness.settle();
    assert_eq!(
        harness.pixel(4, (2.0 * ROW + ROW / 2.0) as u32),
        highlighted,
        "the block that mounted again shows its part of the selection"
    );
    harness.assert_matches_full_repaint();
}

/// Builds a scope over three paragraphs whose texts are those of the first
/// three blocks, the middle one a link that counts its clicks.
fn paragraphs(clicks: RwSignal<u32>) -> (Harness, MemoryClipboard) {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let clicks = clicks.clone();
        selection_scope(
            || (0..3u32).collect(),
            |key: &u32| text(*key),
            move || {
                let clicks = clicks.clone();
                column()
                    .child(
                        rich_text(Rc::from([Span::new(text(0), SpanStyle::default())]))
                            .height(px(ROW))
                            .selectable(0u32),
                    )
                    .child(
                        rich_text(Rc::from([Span::link(text(1), move || {
                            clicks.update(|count| *count += 1)
                        })]))
                        .height(px(ROW))
                        .selectable(1u32),
                    )
                    .child(
                        rich_text(Rc::from([Span::new(text(2), SpanStyle::default())]))
                            .height(px(ROW))
                            .selectable(2u32),
                    )
                    .child(column().height(px(100.0)))
            },
        )
    });
    let clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();
    (harness, clipboard)
}

#[test]
fn a_drag_across_rich_text_selects_and_copies_it() {
    let (mut harness, mut clipboard) = paragraphs(RwSignal::new(0));

    harness.button_down(on_row(0, 1.0));
    harness.move_to(on_row(2, 390.0));
    harness.button_up(on_row(2, 390.0));
    harness.frame();
    harness.assert_matches_full_repaint();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}\n{}", text(0), text(1), text(2)).as_str()),
    );
}

#[test]
fn a_drag_that_began_on_a_link_leaves_nothing_for_the_next_press() {
    let clicks = RwSignal::new(0u32);
    let (mut harness, mut clipboard) = paragraphs(clicks.clone());

    // A drag that began on the link, which takes the press, over a block.
    harness.button_down(on_row(1, 2.0));
    harness.move_to(on_row(2, 30.0));
    harness.button_up(on_row(2, 30.0));
    // A press on the blank space below the blocks, dragged onto a block.
    harness.button_down(Point { x: 50.0, y: 120.0 });
    harness.move_to(on_row(2, 60.0));
    harness.button_up(on_row(2, 60.0));
    harness.frame();
    clipboard.set(String::from("before"));
    harness.click(Point { x: 50.0, y: 120.0 });
    harness.frame();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("before"),
        "no selection came of the drag that began on the link"
    );
}

#[test]
fn trimming_the_first_blocks_keeps_the_rest_of_the_selection() {
    TextSystem::bundled_only().install();
    let first = RwSignal::new(0u32);
    let bound = first.clone();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let keys = bound.clone();
        let rows = bound.clone();
        selection_scope(
            move || (keys.get()..6).collect(),
            |key: &u32| text(*key),
            move || {
                let rows = rows.clone();
                smudge_core::element::for_each(
                    move || (rows.get()..6).collect::<Vec<u32>>(),
                    |key| *key,
                    |key| label(text(key)).height(px(ROW)).selectable(key),
                )
                .column()
            },
        )
    });
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();

    harness.button_down(on_row(1, 1.0));
    harness.move_to(on_row(3, 390.0));
    harness.button_up(on_row(3, 390.0));
    harness.frame();
    // The two oldest lines go, the start's among them.
    first.set(2);
    harness.frame();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}", text(2), text(3)).as_str()),
    );
}

#[test]
fn a_block_mounted_again_under_its_key_keeps_its_highlight() {
    TextSystem::bundled_only().install();
    let streaming = RwSignal::new(true);
    let bound = streaming.clone();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let first = bound.clone();
        let second = bound.clone();
        selection_scope(
            || (0..2u32).collect(),
            |key: &u32| text(*key),
            move || {
                let first = first.clone();
                let second = second.clone();
                column()
                    .child(
                        // The replacement's slot comes first, so it mounts its
                        // block before the old block's cleanup runs.
                        show(
                            move || !second.get(),
                            || label(text(0)).height(px(ROW)).selectable(0u32),
                        )
                        .column(),
                    )
                    .child(
                        show(
                            move || first.get(),
                            || label(text(0)).height(px(ROW)).selectable(0u32),
                        )
                        .column(),
                    )
                    .child(label(text(1)).height(px(ROW)).selectable(1u32))
            },
        )
    });
    harness.frame();
    // The finished block replaces the streaming one under the same key.
    streaming.set(false);
    harness.frame();
    let unselected = harness.pixel(4, (ROW / 2.0) as u32);

    harness.button_down(on_row(0, 1.0));
    harness.move_to(on_row(1, 390.0));
    harness.button_up(on_row(1, 390.0));
    harness.frame();

    assert_ne!(
        harness.pixel(4, (ROW / 2.0) as u32),
        unselected,
        "the block that replaced the old one under its key is highlighted"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_truncated_block_paints_its_highlight_inside_itself() {
    TextSystem::bundled_only().install();
    let long = "a long line ".repeat(20);
    let shown = long.clone();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let shown = shown.clone();
        let read = shown.clone();
        selection_scope(
            || (0..2u32).collect(),
            move |key: &u32| if *key == 0 { read.clone() } else { text(*key) },
            move || {
                column()
                    .child(
                        label(shown.clone())
                            .max_lines(Some(1))
                            .height(px(ROW))
                            .selectable(0u32),
                    )
                    .child(column().height(px(ROW)))
                    .child(label(text(1)).height(px(ROW)).selectable(1u32))
            },
        )
    });
    harness.frame();
    let gap = (300u32, (ROW * 1.5) as u32);
    let before = harness.pixel(gap.0, gap.1);

    harness.button_down(on_row(0, 1.0));
    harness.move_to(on_row(2, 390.0));
    harness.button_up(on_row(2, 390.0));
    harness.frame();

    assert_eq!(
        harness.pixel(gap.0, gap.1),
        before,
        "the hidden lines of the truncated block painted no highlight below it"
    );
}

#[test]
fn a_line_appended_during_an_upward_drag_keeps_the_drag_on_its_own_end() {
    TextSystem::bundled_only().install();
    let count = RwSignal::new(5u32);
    let bound = count.clone();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        let keys = bound.clone();
        let rows = bound.clone();
        selection_scope(
            move || (0..keys.get()).collect(),
            |key: &u32| text(*key),
            move || {
                let rows = rows.clone();
                smudge_core::element::for_each(
                    move || (0..rows.get()).collect::<Vec<u32>>(),
                    |key| *key,
                    |key| label(text(key)).height(px(ROW)).selectable(key),
                )
                .column()
            },
        )
    });
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();

    harness.button_down(on_row(3, 390.0));
    harness.move_to(on_row(1, 1.0));
    // A line arrives at the end of the log while the drag is held.
    count.set(6);
    harness.frame();
    harness.move_to(on_row(2, 1.0));
    harness.button_up(on_row(2, 1.0));
    harness.frame();

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}", text(2), text(3)).as_str()),
        "the drag kept the press point and moved its own end"
    );
}
