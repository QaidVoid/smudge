use proptest::prelude::*;
use smudge_core::reactive::prelude::*;

use super::{Editor, ParagraphTable, SNAPSHOTS};

/// The start and length of every paragraph in the table.
fn cuts(table: &ParagraphTable) -> Vec<(usize, usize)> {
    table
        .paragraphs
        .iter()
        .map(|paragraph| (paragraph.start, paragraph.len))
        .collect()
}

/// Returns `byte` moved down to a character boundary of `text`.
fn boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// Text drawn from letters, a two-byte letter and line breaks, so that
/// splices land on, next to and across paragraph boundaries.
fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(prop_oneof!["a", "é", "\n", "\n\n"], 0..12)
        .prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn a_splice_cuts_the_text_as_a_full_rebuild_does(
        initial in text(),
        edits in proptest::collection::vec((any::<usize>(), any::<usize>(), text()), 1..8),
    ) {
        let mut buffer = initial;
        let mut table = ParagraphTable::default();
        table.rebuild(&buffer);
        for (a, b, replacement) in edits {
            let a = boundary(&buffer, a % (buffer.len() + 1));
            let b = boundary(&buffer, b % (buffer.len() + 1));
            let range = a.min(b)..a.max(b);
            buffer.replace_range(range.clone(), &replacement);
            table.splice(range, &replacement);

            let mut rebuilt = ParagraphTable::default();
            rebuilt.rebuild(&buffer);
            prop_assert_eq!(cuts(&table), cuts(&rebuilt), "after editing {:?}", buffer);
        }
    }
}

/// Replays a fuzz input whose last word step never returned: in wrapped,
/// mixed-direction text the visual step cycled, and the word walk looped
/// until a step stopped moving. Each call now ends after a bounded walk.
#[test]
fn a_word_step_in_wrapped_mixed_direction_text_returns() {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let value = smudge_core::reactive::RwSignal::new(String::new());
        let editor = super::Editor::new(value.clone());
        editor.set_style(&crate::TextStyle::default());
        editor.delete();
        editor.move_up();
        editor.compose("", None);
        editor.move_up();
        editor.delete();
        editor.insert("ٿ� ((");
        editor.insert("\u{7ba}Ȏ\u{61c}�");
        editor.extend_up();
        editor.delete();
        editor.delete();
        editor.insert("��������");
        editor.insert("��������");
        editor.insert("�T�");
        editor.move_to_point(308.0, 392.0);
        editor.extend_right();
        editor.extend_right();
        editor.extend_right();
        editor.move_to_end();
        editor.extend_right();
        editor.extend_right();
        {
            value.set(String::from("ũ?"));
            editor.set_text("ũ?");
        };
        editor.insert("�\"���");
        editor.insert("�\u{311}\u{61c}��");
        editor.extend_to_point(300.0, 468.0);
        editor.insert("\n");
        editor.insert("��*");
        editor.extend_to_point(350.0, 188.0);
        editor.move_down();
        editor.move_lines(-8);
        editor.insert("��𪪶\u{311}");
        editor.insert("\u{61c}��:��A");
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.compose("", None);
        editor.insert("\n");
        editor.insert("��*");
        editor.extend_to_point(56.0, -26.0);
        editor.extend_up();
        editor.move_up();
        editor.move_word_right();
        editor.extend_left();
        editor.insert("����ũ?");
        editor.move_word_left();
        editor.delete_word();
        editor.move_to_end();
        editor.delete_word();
        editor.select_word_at_point(188.0, 186.0);
        editor.move_lines(-8);
        editor.insert("��𪪶\u{311}");
        editor.insert("\u{61c}:��");
        editor.insert("������l");
        editor.move_word_right();
        editor.compose("", None);
        editor.redo();
        editor.insert("ffffff�");
        editor.extend_right();
        editor.backspace();
        editor.backspace();
        editor.backspace();
        editor.backspace();
        editor.backspace();
        editor.backspace();
        editor.backspace_word();
        editor.backspace();
        editor.extend_right();
        editor.extend_right();
        editor.set_wrap_width(Some(60.0));
        editor.set_wrap_width(Some(60.0));
        editor.move_word_right();
        editor.compose("", None);
        editor.redo();
        editor.insert("ffffff�");
        editor.extend_right();
        editor.extend_right();
        editor.move_word_right();
        editor.extend_lines(0);
        let _ = done.send(());
    });
    assert!(
        finished
            .recv_timeout(std::time::Duration::from_secs(30))
            .is_ok(),
        "the edits did not finish within 30 seconds"
    );
}

#[test]
fn an_edit_that_changes_nothing_copies_no_text() {
    let editor = Editor::new(smudge_core::reactive::RwSignal::new(String::new()));
    editor.set_style(&crate::TextStyle::default());
    editor.set_text("hello");
    editor.move_to_start();
    let before = SNAPSHOTS.with(std::cell::Cell::get);

    editor.backspace();
    editor.backspace_word();
    editor.move_to_end();
    editor.delete();
    editor.delete_word();
    editor.delete_selection();
    editor.compose("\u{3042}", None);
    editor.compose("", None);
    assert_eq!(
        SNAPSHOTS.with(std::cell::Cell::get),
        before,
        "edits that change no text copy no text for undo"
    );

    editor.insert("x");
    assert_eq!(
        SNAPSHOTS.with(std::cell::Cell::get),
        before + 1,
        "an edit that opens an undo step copies the text once"
    );
}

#[test]
fn selection_rectangles_outside_the_visible_range_are_left_out() {
    let editor = Editor::new(smudge_core::reactive::RwSignal::new(String::new()));
    editor.set_style(&crate::TextStyle::default());
    editor.set_text(&"line\n".repeat(200));
    editor.select_all();
    assert!(editor.selection_geometry().len() >= 200);

    let mut rects = Vec::new();
    editor.selection_rects((40.0, 80.0), &mut rects);
    assert!(!rects.is_empty(), "the visible lines show their selection");
    assert!(
        rects.len() < 10,
        "only the lines near the range are kept: {}",
        rects.len()
    );
    assert!(
        rects.iter().all(|rect| rect.y1 >= 40.0 && rect.y0 <= 80.0),
        "every kept rectangle meets the range: {rects:?}"
    );
}
