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

/// Presses `key` with Shift, and with the word modifier of this platform
/// when `word` is set.
fn shift(harness: &mut Harness, key: Key, word: bool) {
    let mut modifiers = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    if word {
        if cfg!(target_os = "macos") {
            modifiers.alt = true;
        } else {
            modifiers.control = true;
        }
    }
    harness.press_key_with(key, modifiers);
    harness.frame();
}

#[test]
fn shift_and_the_arrows_extend_a_selection_across_a_block_edge() {
    let (mut harness, mut clipboard) = column_of(4);

    // A click at the end of the second block puts both ends there.
    harness.click(on_row(1, 390.0));
    harness.frame();
    shift(&mut harness, Key::ArrowRight, false);
    for _ in 0..4 {
        shift(&mut harness, Key::ArrowRight, false);
    }
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("\nline"),
        "the first step crossed into the next block, the rest took characters"
    );

    shift(&mut harness, Key::ArrowRight, true);
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("\nline "),
        "a word step went on to the next word's start"
    );
}

#[test]
fn shift_and_down_takes_the_rest_of_a_block_and_crosses_lines() {
    let (mut harness, mut clipboard) = column_of(4);

    harness.click(on_row(1, 1.0));
    harness.frame();
    // The first step down reaches the end of the block's last line, the
    // second crosses into the next block, and End takes its line.
    shift(&mut harness, Key::ArrowDown, false);
    shift(&mut harness, Key::ArrowDown, false);
    shift(&mut harness, Key::End, false);

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}", text(1), text(2)).as_str()),
    );
}

#[test]
fn escape_clears_and_the_arrows_without_a_selection_select_nothing() {
    let (mut harness, mut clipboard) = column_of(3);

    harness.button_down(on_row(0, 1.0));
    harness.button_up(on_row(1, 390.0));
    harness.frame();
    harness.press_key(Key::Escape);
    harness.frame();
    clipboard.set(String::from("before"));
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("before"),
        "escape cleared the selection"
    );

    shift(&mut harness, Key::ArrowDown, false);
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("before"),
        "with no selection the arrows select nothing"
    );
}

#[test]
fn select_all_copies_the_blocks_a_virtual_list_has_not_mounted() {
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

    harness.click(on_row(0, 1.0));
    harness.press_key_with(Key::Character(String::from("a")), command());
    harness.frame();

    let copied = copy(&mut harness, &mut clipboard).unwrap_or_default();
    assert_eq!(copied.lines().count(), 200, "every block, mounted or not");
    assert!(copied.ends_with(&text(199)));
}

#[test]
fn a_keyboard_extension_past_a_view_s_edge_scrolls_it() {
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
    harness.frame();
    harness.settle();

    harness.click(on_row(0, 1.0));
    harness.frame();
    // Two steps per block: to the end of its line, then into the next.
    for _ in 0..20 {
        shift(&mut harness, Key::ArrowDown, false);
    }
    harness.settle();

    // The tenth block's caret ends at its bottom edge, and the view moved
    // only as far as it takes to show that edge.
    let caret_bottom = 10.0 * ROW + 18.634;
    assert!(
        (offset.get() - (caret_bottom - 5.0 * ROW)).abs() < 6.0,
        "the view moved just far enough to show the moving end, offset {}",
        offset.get()
    );
    harness.assert_matches_full_repaint();
}

/// Builds a scope over `texts`, one label per key, each `width` pixels wide
/// so long texts wrap, with a memory clipboard installed.
fn blocks_of(texts: &'static [&'static str], width: f32) -> (Harness, MemoryClipboard) {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        selection_scope(
            move || (0..texts.len() as u32).collect(),
            move |key: &u32| String::from(texts[*key as usize]),
            move || {
                column().children(
                    (0..texts.len() as u32)
                        .map(|key| label(texts[key as usize]).width(px(width)).selectable(key)),
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
fn end_home_and_down_keep_to_the_lines_of_a_wrapped_block() {
    static TEXTS: &[&str] = &["aaaa bbbb cccc dddd eeee", "next"];
    let (mut harness, mut clipboard) = blocks_of(TEXTS, 60.0);

    harness.click(Point { x: 1.0, y: 8.0 });
    harness.frame();
    shift(&mut harness, Key::End, false);
    let first_line = copy(&mut harness, &mut clipboard).unwrap_or_default();
    assert!(
        first_line.starts_with("aaaa") && !first_line.contains("bbbb"),
        "End took the first visual line alone: {first_line:?}"
    );

    shift(&mut harness, Key::Home, false);
    clipboard.set(String::from("collapsed"));
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("collapsed"),
        "Home went back to the start of that same line"
    );

    shift(&mut harness, Key::End, false);
    shift(&mut harness, Key::ArrowDown, false);
    let two_lines = copy(&mut harness, &mut clipboard).unwrap_or_default();
    assert!(
        two_lines.contains("bbbb") && !two_lines.contains("cccc"),
        "Down from the first line's end reached the second line: {two_lines:?}"
    );
}

#[test]
fn left_through_a_right_to_left_block_crosses_into_the_block_after_it() {
    static TEXTS: &[&str] = &["abc", "\u{5d0}\u{5d1}\u{5d2}", "xyz"];
    let (mut harness, mut clipboard) = blocks_of(TEXTS, 300.0);

    harness.click(Point { x: 290.0, y: 8.0 });
    harness.frame();
    // Into the right-to-left block, through its three letters, which Left
    // walks forward, and on into the block after it.
    shift(&mut harness, Key::ArrowRight, false);
    for _ in 0..4 {
        shift(&mut harness, Key::ArrowLeft, false);
    }

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("\n\u{5d0}\u{5d1}\u{5d2}\n"),
    );
}

#[test]
fn crossing_back_to_where_the_selection_began_selects_nothing() {
    let (mut harness, mut clipboard) = column_of(3);

    harness.click(on_row(0, 390.0));
    harness.frame();
    shift(&mut harness, Key::ArrowRight, false);
    shift(&mut harness, Key::ArrowLeft, false);
    clipboard.set(String::from("kept"));

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("kept"),
        "the copy did not overwrite the clipboard with nothing"
    );
}

#[test]
fn shift_and_ctrl_with_home_or_end_reach_the_whole_scope_s_ends() {
    let (mut harness, mut clipboard) = column_of(4);

    harness.click(on_row(1, 1.0));
    harness.frame();
    let ends = Modifiers {
        shift: true,
        control: true,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::End, ends);
    harness.frame();
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n{}\n{}", text(1), text(2), text(3)).as_str()),
    );
    harness.press_key_with(Key::Home, ends);
    harness.frame();
    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some(format!("{}\n", text(0)).as_str()),
        "the moving end went to the scope's start, before the press point at the second block's start"
    );
}

#[test]
fn the_keyboard_extends_a_selection_through_rich_text() {
    let (mut harness, mut clipboard) = paragraphs(RwSignal::new(0));

    harness.click(on_row(0, 390.0));
    harness.frame();
    shift(&mut harness, Key::ArrowRight, false);
    shift(&mut harness, Key::ArrowRight, true);

    assert_eq!(
        copy(&mut harness, &mut clipboard).as_deref(),
        Some("\nline "),
        "a word step went on to the next word's start"
    );
}

#[test]
fn left_runs_through_a_wrapped_right_to_left_block_into_the_next() {
    static TEXTS: &[&str] = &[
        "abc",
        "\u{5d0}\u{5d1}\u{5d2}\u{5d3} \u{5d4}\u{5d5}\u{5d6}\u{5d7} \u{5d8}\u{5d9}\u{5da}\u{5db}",
        "xyz",
    ];
    let (mut harness, mut clipboard) = blocks_of(TEXTS, 40.0);

    // The end of the first block, then into the wrapped block.
    harness.click(Point { x: 39.0, y: 8.0 });
    harness.frame();
    shift(&mut harness, Key::ArrowRight, false);
    let whole = format!("\n{}\n", TEXTS[1]);
    let mut reached = false;
    for _ in 0..40 {
        shift(&mut harness, Key::ArrowLeft, false);
        if copy(&mut harness, &mut clipboard).as_deref() == Some(whole.as_str()) {
            reached = true;
            break;
        }
    }
    assert!(
        reached,
        "Left ran through every line of the block into the next"
    );
}

#[test]
fn a_word_step_back_from_a_block_s_end_stops_at_a_one_letter_word() {
    static TEXTS: &[&str] = &["a b", "xyz"];
    let (mut harness, mut clipboard) = blocks_of(TEXTS, 300.0);

    // The start of the second block, then back across into the first.
    harness.click(Point { x: 1.0, y: 26.0 });
    harness.frame();
    shift(&mut harness, Key::ArrowLeft, false);
    shift(&mut harness, Key::ArrowLeft, true);

    assert_eq!(copy(&mut harness, &mut clipboard).as_deref(), Some("b\n"));
}
