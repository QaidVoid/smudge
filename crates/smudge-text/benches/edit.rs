//! Measures the per-keystroke cost of editing a large buffer.
//!
//! Typing at the end of a full document is the load a streaming editor
//! faces, and the number that decides whether the editor's one `String`
//! needs a rope behind it. Each measured keystroke inserts one character at
//! the caret, which sits at the end of the text, through the public
//! [`Editor`], and then asks for everything a text area's frame asks for:
//! the caret and the text size for the scroll, twice as the edit and the
//! paint each place the caret, the selection and preedit geometry, the
//! input method area, the comparison the area's binding makes against the
//! bound value, and the glyph runs of every paragraph whose layout changed.
//! That covers the buffer write, the undo history, the signal write and the
//! relayout of the paragraph the keystroke touched.
//!
//! The document stays the same size: after every measured keystroke, or
//! every typed word, a backspace that is not measured takes the typed
//! characters off again, and its frame lays the paragraph out as it was.
//!
//! The undo case types a word unmeasured and measures the undo that takes
//! it off again, with the frame after it, which is the cost of one undo
//! step in a document of that size.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use smudge_core::geometry::{Color, Point};
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_text::{Editor, TextSystem};

/// The width the editor wraps its lines to, in logical pixels.
const WRAP_WIDTH: f32 = 600.0;

/// How many characters one typed word holds.
const WORD: u64 = 8;

/// The words the generated prose draws from.
const WORDS: [&str; 8] = [
    "stream", "row", "log", "line", "buffer", "pane", "edge", "glyph",
];

/// Builds about `target` bytes of prose, broken into lines of twelve words,
/// with no line break after the last line, so typing at the end lays out a
/// full line again.
fn prose(target: usize) -> String {
    let mut text = String::with_capacity(target + 16);
    let mut seed = 0x5EED_u64;
    while text.len() < target {
        for word in 0..12 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            text.push_str(WORDS[(seed >> 33) as usize % WORDS.len()]);
            if word == 11 {
                text.push('\n');
            } else {
                text.push(' ');
            }
        }
    }
    text.pop();
    text
}

/// An editor over a fixed document, with what a frame keeps between frames.
struct Document {
    /// The editor, with the caret at the end of the text.
    editor: Editor,
    /// The signal the editor writes, which the area's binding reads.
    value: RwSignal<String>,
    /// The layout stamp of every paragraph as the last frame saw it, which
    /// decides the paragraphs whose glyph runs a frame builds again.
    stamps: Vec<u64>,
}

impl Document {
    /// Creates an editor holding `bytes` of prose, wrapped, with the caret
    /// at the end and every paragraph laid out.
    fn new(bytes: usize) -> Document {
        TextSystem::bundled_only().install();
        let value = RwSignal::new(String::new());
        let editor = Editor::new(value.clone());
        editor.set_wrap_width(Some(WRAP_WIDTH));
        editor.set_text(&prose(bytes));
        let mut document = Document {
            editor,
            value,
            stamps: Vec::new(),
        };
        document.frame();
        document
    }

    /// Asks for what a text area's frame asks for after an edit.
    fn frame(&mut self) {
        let editor = &self.editor;
        for _ in 0..2 {
            black_box(editor.cursor_geometry(1.0));
            black_box(editor.layout_size());
        }
        black_box(self.value.read(|text| editor.has_text(text)));
        black_box(editor.selection_geometry());
        black_box(editor.compose_geometry());
        black_box(editor.is_empty());
        black_box(editor.ime_area());
        let stamps = &mut self.stamps;
        let mut index = 0;
        editor.paragraphs(|paragraph| {
            if stamps.get(index) != Some(&paragraph.stamp()) {
                black_box(paragraph.glyph_runs_at(Point::default(), Color::BLACK));
                if index < stamps.len() {
                    stamps[index] = paragraph.stamp();
                } else {
                    stamps.push(paragraph.stamp());
                }
            }
            index += 1;
        });
        stamps.truncate(index);
    }

    /// Types a word unmeasured, then returns how long undoing it and the
    /// frame after the undo took.
    fn undo_word(&mut self) -> Duration {
        for _ in 0..WORD {
            self.editor.insert("x");
        }
        self.frame();
        let start = Instant::now();
        self.editor.undo();
        self.frame();
        start.elapsed()
    }

    /// Types `count` characters and returns how long the keystrokes and
    /// their frames took, then takes the characters off again unmeasured.
    fn type_and_reset(&mut self, count: u64) -> Duration {
        let mut spent = Duration::ZERO;
        for _ in 0..count {
            let start = Instant::now();
            self.editor.insert("x");
            self.frame();
            spent += start.elapsed();
        }
        for _ in 0..count {
            self.editor.backspace();
        }
        self.frame();
        spent
    }
}

fn editing(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit");
    for bytes in [1024, 1024 * 1024] {
        let mut document = Document::new(bytes);
        group.bench_function(
            format!("keystroke in a word at end of {bytes} bytes"),
            |b| {
                b.iter_custom(|iters| {
                    let mut spent = Duration::ZERO;
                    let mut left = iters;
                    while left > 0 {
                        let count = left.min(WORD);
                        spent += document.type_and_reset(count);
                        left -= count;
                    }
                    spent
                })
            },
        );
        group.bench_function(
            format!("undo of a typed word at end of {bytes} bytes"),
            |b| b.iter_custom(|iters| (0..iters).map(|_| document.undo_word()).sum::<Duration>()),
        );
        group.bench_function(
            format!("keystroke opening an undo step at end of {bytes} bytes"),
            |b| {
                b.iter_custom(|iters| {
                    (0..iters)
                        .map(|_| document.type_and_reset(1))
                        .sum::<Duration>()
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, editing);
criterion_main!(benches);
