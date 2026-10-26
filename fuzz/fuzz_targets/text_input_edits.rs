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
        let op = match command % 20 {
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
            _ => Op::Write(take_text(&mut rest)),
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
    }
}

fuzz_target!(|data: &[u8]| {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value.clone());
    editor.set_style(&TextStyle::default());

    for op in &decode(data) {
        apply(&editor, &value, op);

        let text = editor.text();
        assert_eq!(text, value.get(), "the editor and the signal drifted apart");

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
