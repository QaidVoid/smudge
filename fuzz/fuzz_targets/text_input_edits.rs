//! Fuzzes the text editor with random edit sequences.
//!
//! Every input decodes into a sequence of edits, which the target applies to
//! one [`Editor`] bound to a signal, exactly the way the text input's and the
//! text area's handlers do. Nothing may panic, the editor's text must stay
//! equal to the signal and hold no carriage return, the selection must sit on
//! char boundaries of the text, and the paragraphs the edits left must lay
//! out exactly as a fresh editor given the same text lays them out. The
//! command set spans both widgets: line breaks, vertical moves by line and by
//! page, Home and End within a line, and a wrap width that turns one
//! paragraph into several visual lines. The first byte picks the area's
//! edits or the single-line input's. The input's editor is a single-line
//! one, which turns every line break of a paste, a commit, a typed text or
//! an outside write into a space, so the input's text must never hold one,
//! and the input sends no vertical moves, no line breaks and no wrap.

#![no_main]

use std::ops::Range;

use libfuzzer_sys::fuzz_target;
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_text::Editor;
use smudge_text::TextStyle;

// The target reads only which characters break a line; the editor applies
// the flattening itself.
#[allow(dead_code)]
#[path = "../../crates/smudge-text/src/line_breaks.rs"]
mod line_breaks;

/// One edit the target applies, decoded from the input bytes.
#[derive(Clone, Debug)]
enum Op {
    /// Inserts a short string at the caret.
    Insert(String),
    /// Inserts a line break, as Enter does in an area.
    Enter,
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
    /// Moves the caret one visual line up.
    Up,
    /// Moves the caret one visual line down.
    Down,
    /// Moves the caret several visual lines, as a page key does.
    MoveLines(isize),
    /// Jumps to the start of the visual line.
    LineStart,
    /// Jumps to the end of the visual line.
    LineEnd,
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
    /// Extends the selection one visual line up.
    ExtendUp,
    /// Extends the selection one visual line down.
    ExtendDown,
    /// Extends the selection several visual lines, as a page key does.
    ExtendLines(isize),
    /// Places the caret at a point, as a press does.
    MoveToPoint(f32, f32),
    /// Extends the selection to a point, as a drag does.
    ExtendToPoint(f32, f32),
    /// Selects the word at a point, as a double click does.
    SelectWordAt(f32, f32),
    /// Wraps the text at a fixed width, or stops wrapping it.
    Wrap(bool),
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
/// Every edit consumes one command byte and, for an insertion or a point, a
/// few more for its payload. Truncated input yields a shorter sequence.
fn decode(bytes: &[u8]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut rest = bytes;
    while let Some((&command, tail)) = rest.split_first() {
        rest = tail;
        let op = match command % 36 {
            0 => Op::Insert(take_text(&mut rest)),
            1 => Op::Enter,
            2 => Op::Backspace,
            3 => Op::BackspaceWord,
            4 => Op::Delete,
            5 => Op::DeleteWord,
            6 => Op::Left,
            7 => Op::Right,
            8 => Op::WordLeft,
            9 => Op::WordRight,
            10 => Op::Up,
            11 => Op::Down,
            12 => Op::MoveLines(take_count(&mut rest)),
            13 => Op::ToStart,
            14 => Op::ToEnd,
            15 => Op::SelectAll,
            16 => Op::ExtendLeft,
            17 => Op::ExtendRight,
            18 => Op::ExtendWordLeft,
            19 => Op::ExtendWordRight,
            20 => Op::ExtendUp,
            21 => Op::ExtendDown,
            22 => Op::ExtendLines(take_count(&mut rest)),
            23 => Op::MoveToPoint(take_x(&mut rest), take_x(&mut rest)),
            24 => Op::ExtendToPoint(take_x(&mut rest), take_x(&mut rest)),
            25 => Op::SelectWordAt(take_x(&mut rest), take_x(&mut rest)),
            26 => Op::Wrap(take_flag(&mut rest)),
            27 => Op::Write(take_multiline(&mut rest)),
            28 => Op::Undo,
            29 => Op::Redo,
            30 => Op::Compose(take_text(&mut rest), take_cursor(&mut rest)),
            31 => Op::CancelCompose,
            32 => Op::Commit(take_multiline(&mut rest)),
            33 => Op::LineStart,
            34 => Op::LineEnd,
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

/// Takes up to eight bytes as text, line breaks included, as a paste, an
/// outside write or an input method commit carries them.
fn take_multiline(rest: &mut &[u8]) -> String {
    let (chunk, tail) = rest.split_at(rest.len().min(8));
    *rest = tail;
    String::from_utf8_lossy(chunk)
        .chars()
        .filter(|character| !character.is_control() || line_breaks::is_line_break(*character))
        .collect()
}

/// Keeps the edits a single-line input sends, which leaves out Enter, the
/// vertical moves, the moves within a visual line and the wrap: the input's
/// Home and End jump to the ends of its one line. The text of every edit is
/// kept as it came, line breaks included, for the editor to flatten.
fn single_line(ops: Vec<Op>) -> Vec<Op> {
    ops.into_iter()
        .filter(|op| {
            !matches!(
                op,
                Op::Enter
                    | Op::Up
                    | Op::Down
                    | Op::MoveLines(_)
                    | Op::ExtendUp
                    | Op::ExtendDown
                    | Op::ExtendLines(_)
                    | Op::LineStart
                    | Op::LineEnd
                    | Op::Wrap(_)
            )
        })
        .collect()
}

/// Takes one byte as a coordinate that reaches before the start of the text
/// and past its end.
fn take_x(rest: &mut &[u8]) -> f32 {
    let Some((&byte, tail)) = rest.split_first() else {
        return 0.0;
    };
    *rest = tail;
    f32::from(byte) * 2.0 - 40.0
}

/// Takes one byte as a line count from eight lines up to eight lines down,
/// as a page key repeats a page of lines.
fn take_count(rest: &mut &[u8]) -> isize {
    let Some((&byte, tail)) = rest.split_first() else {
        return 0;
    };
    *rest = tail;
    isize::from(byte % 17) - 8
}

/// Takes one byte as a choice between two states.
fn take_flag(rest: &mut &[u8]) -> bool {
    let Some((&byte, tail)) = rest.split_first() else {
        return false;
    };
    *rest = tail;
    byte % 2 == 0
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

/// The width the target wraps at while wrapping is on, in logical pixels.
const WRAP: f32 = 60.0;

/// Returns the offset and size of every paragraph the editor lays out.
fn paragraphs(editor: &Editor) -> Vec<(f32, f32, f32)> {
    let mut out = Vec::new();
    editor.paragraphs(|paragraph| {
        let size = paragraph.size();
        out.push((paragraph.offset().y, size.width, size.height));
    });
    out
}

/// Applies one edit, as the editing widgets' handlers and their binding do.
fn apply(editor: &Editor, value: &RwSignal<String>, op: &Op) {
    match op {
        Op::Insert(text) => editor.insert(text),
        Op::Enter => editor.insert("\n"),
        Op::Backspace => editor.backspace(),
        Op::BackspaceWord => editor.backspace_word(),
        Op::Delete => editor.delete(),
        Op::DeleteWord => editor.delete_word(),
        Op::Left => editor.move_left(),
        Op::Right => editor.move_right(),
        Op::WordLeft => editor.move_word_left(),
        Op::WordRight => editor.move_word_right(),
        Op::Up => editor.move_up(),
        Op::Down => editor.move_down(),
        Op::MoveLines(lines) => editor.move_lines(*lines),
        Op::LineStart => editor.move_to_line_start(),
        Op::LineEnd => editor.move_to_line_end(),
        Op::ToStart => editor.move_to_start(),
        Op::ToEnd => editor.move_to_end(),
        Op::SelectAll => editor.select_all(),
        Op::ExtendLeft => editor.extend_left(),
        Op::ExtendRight => editor.extend_right(),
        Op::ExtendWordLeft => editor.extend_word_left(),
        Op::ExtendWordRight => editor.extend_word_right(),
        Op::ExtendUp => editor.extend_up(),
        Op::ExtendDown => editor.extend_down(),
        Op::ExtendLines(lines) => editor.extend_lines(*lines),
        Op::MoveToPoint(x, y) => editor.move_to_point(*x, *y),
        Op::ExtendToPoint(x, y) => editor.extend_to_point(*x, *y),
        Op::SelectWordAt(x, y) => editor.select_word_at_point(*x, *y),
        Op::Wrap(on) => {
            if *on {
                editor.set_wrap_width(Some(WRAP));
            } else {
                editor.set_wrap_width(None);
            }
        }
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
    let Some((&mode, data)) = data.split_first() else {
        return;
    };
    let is_single_line = mode % 2 == 1;
    let ops = if is_single_line {
        single_line(decode(data))
    } else {
        decode(data)
    };
    let value = RwSignal::new(String::new());
    let editor = if is_single_line {
        Editor::single_line(value.clone())
    } else {
        Editor::new(value.clone())
    };
    editor.set_style(&TextStyle::default());
    let mut wrapped = false;

    for op in &ops {
        if let Op::Wrap(on) = op {
            wrapped = *on;
        }
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

        if is_single_line {
            assert!(
                !value.get().contains(line_breaks::is_line_break),
                "the single-line text grew a line break, after {op:?}"
            );
        }

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
        assert!(
            !editor.text().contains('\r'),
            "a carriage return reached the text, after {op:?}"
        );

        // The paragraph table an edit spliced must cut the text the way a
        // fresh cut does, which the layouts of the paragraphs show.
        let fresh = Editor::new(RwSignal::new(String::new()));
        fresh.set_style(&TextStyle::default());
        fresh.set_wrap_width(wrapped.then_some(WRAP));
        fresh.set_text(&raw);
        assert_eq!(
            paragraphs(&editor),
            paragraphs(&fresh),
            "the paragraphs drifted from a fresh cut of {raw:?}, after {op:?}"
        );
    }
});
