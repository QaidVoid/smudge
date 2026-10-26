//! Fuzzes the text input's editor with random edit sequences.
//!
//! Every input decodes into a sequence of edits, which the target applies to
//! one [`Editor`] bound to a signal, exactly the way the text input's
//! handlers do. Nothing may panic, the editor's text must stay equal to the
//! signal, and the selection must sit on char boundaries of the text.

#![no_main]

use std::ops::Range;

use libfuzzer_sys::fuzz_target;
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_text::Editor;
use smudge_text::TextStyle;

/// One edit the target applies, decoded from the input bytes.
#[derive(Clone, Debug)]
enum Op {
    /// Inserts a short string at the caret.
    Insert(String),
    /// Deletes backwards, by grapheme.
    Backspace,
    /// Deletes backwards, by word.
    BackspaceWord,
    /// Deletes forwards, by grapheme.
    Delete,
    /// Deletes forwards, by word.
    DeleteWord,
    /// Moves the caret one grapheme left.
    Left,
    /// Moves the caret one grapheme right.
    Right,
    /// Moves the caret one word left.
    WordLeft,
    /// Moves the caret one word right.
    WordRight,
    /// Jumps to the start.
    ToStart,
    /// Jumps to the end.
    ToEnd,
    /// Selects everything.
    SelectAll,
    /// Extends the selection one grapheme left.
    ExtendLeft,
    /// Extends the selection one grapheme right.
    ExtendRight,
    /// Extends the selection one word left.
    ExtendWordLeft,
    /// Extends the selection one word right.
    ExtendWordRight,
    /// Places the caret at an x coordinate, as a press does.
    MoveToPoint(f32),
    /// Extends the selection to an x coordinate, as a drag does.
    ExtendToPoint(f32),
    /// Selects the word at an x coordinate, as a double click does.
    SelectWordAt(f32),
    /// Replaces the text from outside, as a write to the bound signal does.
    Write(String),
    /// Shows or updates the text an input method composes, with its cursor.
    Compose(String, Option<(usize, usize)>),
    /// Cancels the composing text, as an empty preedit does.
    CancelCompose,
    /// Commits text from an input method.
    Commit(String),
    /// Pastes text from a clipboard, line breaks included.
    Paste(String),
    /// Undoes the last step.
    Undo,
    /// Redoes the last undone step.
    Redo,
}

/// Decodes the input bytes into a sequence of edits.
///
/// Every edit consumes one command byte and, for an insertion, a few more
/// for the text. Truncated input yields a shorter sequence.
fn decode(bytes: &[u8]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut rest = bytes;
    while let Some((&command, tail)) = rest.split_first() {
        rest = tail;
        let op = match command % 26 {
            0 => Op::Insert(take_text(&mut rest)),
            1 => Op::Backspace,
            2 => Op::BackspaceWord,
            3 => Op::Delete,
            4 => Op::DeleteWord,
            5 => Op::Left,
            6 => Op::Right,
            7 => Op::WordLeft,
            8 => Op::WordRight,
            9 => Op::ToStart,
            10 => Op::ToEnd,
            11 => Op::SelectAll,
            12 => Op::ExtendLeft,
            13 => Op::ExtendRight,
            14 => Op::ExtendWordLeft,
            15 => Op::ExtendWordRight,
            16 => Op::MoveToPoint(take_x(&mut rest)),
            17 => Op::ExtendToPoint(take_x(&mut rest)),
            18 => Op::SelectWordAt(take_x(&mut rest)),
            19 => Op::Write(take_text(&mut rest)),
            20 => Op::Undo,
            21 => Op::Redo,
            22 => Op::Compose(take_text(&mut rest), take_cursor(&mut rest)),
            23 => Op::CancelCompose,
            24 => Op::Commit(take_text(&mut rest)),
            _ => Op::Paste(take_multiline(&mut rest)),
        };
        ops.push(op);
    }
    ops
}

/// Takes up to eight bytes as text, without control characters.
fn take_text(rest: &mut &[u8]) -> String {
    let (chunk, tail) = rest.split_at(rest.len().min(8));
    *rest = tail;
    String::from_utf8_lossy(chunk)
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

/// Takes one byte as an x coordinate that reaches before the start of the
/// text and past its end.
fn take_x(rest: &mut &[u8]) -> f32 {
    let Some((&byte, tail)) = rest.split_first() else {
        return 0.0;
    };
    *rest = tail;
    f32::from(byte) * 2.0 - 40.0
}

/// Takes two bytes as the offsets of an input method cursor, which may sit
/// outside the preedit, or none, as a platform reports no cursor.
fn take_cursor(rest: &mut &[u8]) -> Option<(usize, usize)> {
    let (&anchor, tail) = rest.split_first()?;
    let Some((&focus, tail)) = tail.split_first() else {
        *rest = tail;
        return None;
    };
    *rest = tail;
    (anchor != 0 || focus != 0).then_some((usize::from(anchor), usize::from(focus)))
}

/// Takes up to eight bytes as text, line breaks included, as a clipboard
/// paste carries them.
fn take_multiline(rest: &mut &[u8]) -> String {
    let (chunk, tail) = rest.split_at(rest.len().min(8));
    *rest = tail;
    String::from_utf8_lossy(chunk)
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\r')
        .collect()
}

/// Applies one edit, as the text input's handlers and its binding would.
fn apply(editor: &Editor, value: &RwSignal<String>, op: &Op) {
    match op {
        Op::Insert(text) => editor.insert(text),
        Op::Backspace => editor.backspace(),
        Op::BackspaceWord => editor.backspace_word(),
        Op::Delete => editor.delete(),
        Op::DeleteWord => editor.delete_word(),
        Op::Left => editor.move_left(),
        Op::Right => editor.move_right(),
        Op::WordLeft => editor.move_word_left(),
        Op::WordRight => editor.move_word_right(),
        Op::ToStart => editor.move_to_start(),
        Op::ToEnd => editor.move_to_end(),
        Op::SelectAll => editor.select_all(),
        Op::ExtendLeft => editor.extend_left(),
        Op::ExtendRight => editor.extend_right(),
        Op::ExtendWordLeft => editor.extend_word_left(),
        Op::ExtendWordRight => editor.extend_word_right(),
        Op::MoveToPoint(x) => editor.move_to_point(*x, 8.0),
        Op::ExtendToPoint(x) => editor.extend_to_point(*x, 8.0),
        Op::SelectWordAt(x) => editor.select_word_at_point(*x, 8.0),
        Op::Write(text) => {
            value.set(text.clone());
            editor.set_text(text);
        }
        Op::Compose(text, cursor) => editor.compose(text, *cursor),
        Op::CancelCompose => editor.compose("", None),
        Op::Commit(text) => editor.commit(text),
        Op::Paste(text) => editor.paste(text),
        Op::Undo => editor.undo(),
        Op::Redo => editor.redo(),
    }
}

fuzz_target!(|data: &[u8]| {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value.clone());
    editor.set_style(&TextStyle::default());

    for op in &decode(data) {
        let value_before = value.get();
        let was_composing = editor.is_composing();
        apply(&editor, &value, op);

        // The preedit is not committed text, so only the selection a new
        // composition deletes can change the value while composing.
        let keeps_value = match op {
            Op::Compose(..) => was_composing,
            Op::CancelCompose => true,
            _ => false,
        };
        if keeps_value {
            assert_eq!(
                value.get(),
                value_before,
                "composing changed the signal, after {op:?}"
            );
        }

        assert_eq!(
            editor.text(),
            value.get(),
            "the editor and the signal drifted apart, after {op:?}"
        );

        let raw = editor.raw_text();
        let Range { start, end } = editor.selection();
        assert!(
            raw.is_char_boundary(start),
            "the selection starts inside a character at {start} of {raw:?}"
        );
        assert!(
            raw.is_char_boundary(end),
            "the selection ends inside a character at {end} of {raw:?}"
        );
        assert!(start <= raw.len() && end <= raw.len());
    }
});
