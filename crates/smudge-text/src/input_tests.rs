use proptest::prelude::*;
use smudge_core::reactive::prelude::*;

use super::{Editor, MASK, ParagraphTable, SNAPSHOTS, grapheme_index};

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

/// An editor laid out in the default style holding `text`.
fn editor_with(text: &str) -> Editor {
    crate::TextSystem::bundled_only().install();
    let editor = Editor::new(smudge_core::reactive::RwSignal::new(String::new()));
    editor.set_style(&crate::TextStyle::default());
    editor.set_text(text);
    editor
}

/// The text of each run of `editor`, with where it starts.
fn runs_of(editor: &Editor) -> Vec<(usize, String)> {
    let (runs, _) = editor.accessible_text(smudge_core::geometry::Point::default());
    runs.into_iter().map(|run| (run.start, run.text)).collect()
}

#[test]
fn a_paragraph_ends_its_last_run_in_its_line_feed() {
    let editor = editor_with("ab\ncd");
    assert_eq!(
        runs_of(&editor),
        vec![(0, String::from("ab\n")), (3, String::from("cd"))]
    );
}

#[test]
fn an_empty_line_is_a_run_of_its_own() {
    let editor = editor_with("ab\n\ncd");
    let (runs, _) = editor.accessible_text(smudge_core::geometry::Point::default());
    let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["ab\n", "\n", "cd"]);
    assert!(!runs[1].continues_line, "the empty line starts a line");
    assert_eq!(runs[1].word_starts, vec![0]);
    assert!(
        runs[1].bounds.y0 >= runs[0].bounds.y1 - 0.01,
        "it sits below the first line"
    );
}

#[test]
fn an_empty_last_line_gives_the_caret_a_run() {
    let empty = editor_with("");
    assert_eq!(runs_of(&empty), vec![(0, String::new())]);
    let trailing = editor_with("ab\n");
    assert_eq!(
        runs_of(&trailing),
        vec![(0, String::from("ab\n")), (3, String::new())]
    );
}

#[test]
fn the_selection_keeps_which_end_moves() {
    let editor = editor_with("abc");
    editor.move_to_end();
    editor.extend_left();
    let (_, selection) = editor.accessible_text(smudge_core::geometry::Point::default());
    assert_eq!((selection.anchor.byte, selection.focus.byte), (3, 2));
}

#[test]
fn a_caret_moved_to_the_end_of_a_wrapped_line_says_it_ends_the_line() {
    let editor = editor_with("word word word");
    editor.set_wrap_width(Some(60.0));
    editor.move_to_start();
    editor.move_to_line_end();
    let (runs, selection) = editor.accessible_text(smudge_core::geometry::Point::default());
    assert!(runs.len() > 1, "the text wraps");
    assert_eq!(selection.focus.byte, runs[1].start);
    assert!(selection.focus.ends_line);
}

#[test]
fn a_full_run_leaves_its_line_feed_a_run_on_its_line() {
    let text = format!(
        "a\u{2028}{}\nb",
        "x".repeat(smudge_core::semantics::TEXT_RUN_CAP)
    );
    let editor = editor_with(&text);
    let (runs, _) = editor.accessible_text(smudge_core::geometry::Point::default());
    let feed = runs
        .iter()
        .position(|run| run.text == "\n")
        .expect("the line feed has a run of its own");
    assert!(
        runs[feed].continues_line,
        "it goes on with the full run's line"
    );
    assert_eq!(
        (runs[feed].bounds.y0, runs[feed].bounds.y1),
        (runs[feed - 1].bounds.y0, runs[feed - 1].bounds.y1)
    );
    assert_eq!(runs[feed].bounds.x0, runs[feed - 1].bounds.x1);
}

#[test]
fn a_caret_moved_past_a_line_separator_starts_the_next_line() {
    let editor = editor_with("ab\u{2028}cd");
    editor.move_to_start();
    editor.move_right();
    editor.move_right();
    editor.move_right();
    let (_, selection) = editor.accessible_text(smudge_core::geometry::Point::default());
    assert_eq!(selection.focus.byte, 5);
    assert!(
        !selection.focus.ends_line,
        "past a line break the caret paints at the start of the next line"
    );
}

/// One step of a masked editing session.
#[derive(Clone, Debug)]
enum MaskOp {
    Insert(String),
    Paste(String),
    Backspace,
    Delete,
    BackspaceWord,
    Left,
    Right,
    ExtendLeft,
    ExtendRight,
    LineStart,
    SelectAll,
    Undo,
    Redo,
    Toggle,
}

/// Masked editing steps, inserting the pieces `piece` draws.
fn mask_ops(piece: BoxedStrategy<String>) -> impl Strategy<Value = Vec<MaskOp>> {
    let op = prop_oneof![
        3 => piece.clone().prop_map(MaskOp::Insert),
        1 => piece.prop_map(MaskOp::Paste),
        1 => Just(MaskOp::Backspace),
        1 => Just(MaskOp::Delete),
        1 => Just(MaskOp::BackspaceWord),
        1 => Just(MaskOp::Left),
        1 => Just(MaskOp::Right),
        1 => Just(MaskOp::ExtendLeft),
        1 => Just(MaskOp::ExtendRight),
        1 => Just(MaskOp::LineStart),
        1 => Just(MaskOp::SelectAll),
        1 => Just(MaskOp::Undo),
        1 => Just(MaskOp::Redo),
        1 => Just(MaskOp::Toggle),
    ];
    proptest::collection::vec(op, 0..40)
}

/// Runs `op` on `editor`, which masks its text when `toggles` lets it.
fn apply(editor: &Editor, op: &MaskOp, toggles: bool) {
    match op {
        MaskOp::Insert(text) => editor.insert(text),
        MaskOp::Paste(text) => editor.paste(text),
        MaskOp::Backspace => editor.backspace(),
        MaskOp::Delete => editor.delete(),
        MaskOp::BackspaceWord => editor.backspace_word(),
        MaskOp::Left => editor.move_left(),
        MaskOp::Right => editor.move_right(),
        MaskOp::ExtendLeft => editor.extend_left(),
        MaskOp::ExtendRight => editor.extend_right(),
        MaskOp::LineStart => editor.move_to_line_start(),
        MaskOp::SelectAll => editor.select_all(),
        MaskOp::Undo => editor.undo(),
        MaskOp::Redo => editor.redo(),
        MaskOp::Toggle if toggles => editor.set_masked(!editor.is_masked()),
        MaskOp::Toggle => {}
    }
}

/// The selection of `editor` as grapheme indices into its text.
fn grapheme_selection(editor: &Editor) -> (usize, usize) {
    let range = editor.selection();
    if editor.is_masked() {
        let bullet = MASK.len_utf8();
        (range.start / bullet, range.end / bullet)
    } else {
        let text = editor.text();
        (
            grapheme_index(&text, range.start),
            grapheme_index(&text, range.end),
        )
    }
}

/// A one-line editor with the default style, masked when `masked` is set.
fn line_editor(masked: bool) -> (Editor, smudge_core::reactive::RwSignal<String>) {
    let value = smudge_core::reactive::RwSignal::new(String::new());
    let editor = Editor::single_line(value.clone());
    editor.set_style(&crate::TextStyle::default());
    editor.set_masked(masked);
    (editor, value)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Word steps differ on purpose and are left out, as are graphemes that
    /// a backspace takes apart in plain text and right-to-left text, whose
    /// visual moves the mask does not show.
    #[test]
    fn masked_editing_edits_the_text_as_plain_editing_does(
        ops in mask_ops(prop_oneof!["a", "b", " ", "\u{e9}", "\u{1f44d}\u{1f3fd}"].boxed()),
    ) {
        crate::TextSystem::bundled_only().install();
        let (masked, value) = line_editor(true);
        let (plain, _) = line_editor(false);
        for op in &ops {
            if matches!(op, MaskOp::BackspaceWord) {
                continue;
            }
            apply(&masked, op, true);
            apply(&plain, op, false);
            if matches!(op, MaskOp::Toggle) {
                plain.history.borrow_mut().open = None;
            }
            prop_assert_eq!(masked.text(), plain.text(), "after {:?}", op);
            prop_assert_eq!(value.get(), plain.text());
            prop_assert_eq!(grapheme_selection(&masked), grapheme_selection(&plain), "after {:?}", op);
        }
    }

    #[test]
    fn a_mask_holds_one_bullet_per_grapheme_of_the_text(
        ops in mask_ops(prop_oneof!["a", "e", "\u{301}", "\u{200d}", "\u{1f469}", "\u{1f1f3}", "\u{1f1f5}"].boxed()),
    ) {
        crate::TextSystem::bundled_only().install();
        let (editor, value) = line_editor(true);
        for op in &ops {
            apply(&editor, op, true);
            let text = editor.text();
            prop_assert_eq!(value.get(), text.clone());
            if editor.is_masked() {
                let raw = editor.raw_text();
                prop_assert!(raw.chars().all(|c| c == MASK), "{:?}", raw);
                prop_assert_eq!(raw.chars().count(), grapheme_index(&text, text.len()), "for {:?}", text);
                let range = editor.selection();
                prop_assert!(range.end <= raw.len() && range.start % MASK.len_utf8() == 0);
            } else {
                prop_assert_eq!(editor.raw_text(), text);
            }
        }
    }
}

#[test]
fn a_masked_preedit_cursor_sits_on_the_bullet_of_its_grapheme() {
    crate::TextSystem::bundled_only().install();
    let (editor, _) = line_editor(true);
    editor.insert("a");
    let bullet = MASK.len_utf8();
    for (cursor, bullets) in [((1, 1), 1), ((3, 3), 1), ((4, 4), 2), ((0, 4), 0)] {
        editor.compose("e\u{301}x", Some(cursor));
        assert_eq!(
            editor.raw_text().chars().count(),
            3,
            "one bullet per grapheme"
        );
        let start = bullet + bullets * bullet;
        let end = bullet + if cursor.0 == cursor.1 { bullets } else { 2 } * bullet;
        assert_eq!(editor.selection(), start..end, "for the cursor {cursor:?}");
    }
}
