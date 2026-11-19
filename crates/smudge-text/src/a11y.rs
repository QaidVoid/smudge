//! The text runs a text widget reports to the accessibility projection: its
//! laid out text, line by line, with every character's place, so a screen
//! reader can read it by character, word and line.

use parley::{Brush, Layout};
use smudge_core::geometry::{Point, Rect};
use smudge_core::semantics::{TEXT_RUN_CAP, TextOffset, TextRun, TextSelection};

/// One character of a line as a screen reader steps over it.
struct Character {
    /// Where the character starts in the text, in bytes.
    start: usize,
    /// Its length in bytes.
    len: u8,
    /// Its left edge, in the layout's coordinates.
    x: f32,
    /// Its width.
    width: f32,
    /// Whether it is white space, which a word takes after it.
    space: bool,
}

/// The stretch of a line one direction covers: a run of the layout.
struct Stretch {
    /// Where its text starts, which orders the stretches of a line.
    start: usize,
    /// Whether it reads right to left.
    right_to_left: bool,
    /// Its characters, in the order they read.
    characters: Vec<Character>,
}

/// What a truncated layout hides: the lines past `kept_lines`, and what of
/// the last kept line lies right of `cut`.
#[derive(Clone, Copy)]
pub(crate) struct Hidden {
    /// How many leading lines show.
    pub(crate) kept_lines: usize,
    /// Where the last kept line is cut, relative to the layout's left
    /// edge, or `None` when it shows whole.
    pub(crate) cut: Option<f32>,
}

/// Returns the runs of `layout`, whose text is `text`, laid out at `origin`
/// in the widget's content box.
///
/// Each line gives a run per run of the layout, a stretch of one direction
/// and one font, in the order they read, and a stretch longer than
/// [`TEXT_RUN_CAP`] characters gives several. A character is a cluster of
/// the layout, the stretch a caret steps over, and one longer than 255
/// bytes, which a run cannot count, splits at the boundaries of its code
/// points into pieces that fit, the first taking the cluster's width.
///
/// A truncated layout reports its whole text, since a screen reader reads
/// the whole value too, but what `hidden` says it hides sits at the cut of
/// the last kept line with no width, where the ellipsis stands for it.
///
/// # Panics
///
/// Panics when a cluster of `layout` covers bytes outside `text`, which
/// means the layout was built from another text.
pub(crate) fn text_runs<B: Brush>(
    layout: &Layout<B>,
    text: &str,
    origin: Point,
    hidden: Option<Hidden>,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut stretches: Vec<Stretch> = Vec::new();
    let right_to_left = layout.is_rtl();
    let mut kept_metrics = None;
    let mut after_hard_break = false;
    for (index, line) in layout.lines().enumerate() {
        let shown = hidden.is_none_or(|hidden| index < hidden.kept_lines);
        if shown || kept_metrics.is_none() {
            kept_metrics = Some(*line.metrics());
        }
        let Some(metrics) = kept_metrics else {
            continue;
        };
        let (start_edge, end_edge) = if right_to_left {
            (metrics.offset + metrics.advance, metrics.offset)
        } else {
            (metrics.offset, metrics.offset + metrics.advance)
        };
        // Hidden text sits where the reading stops: at the cut of the last
        // kept line, at its end when it shows whole, or at the start of the
        // first line when no line shows.
        let fold_at = match hidden {
            Some(hidden) if !shown => Some(if hidden.kept_lines == 0 {
                start_edge
            } else {
                hidden.cut.unwrap_or(end_edge)
            }),
            _ => None,
        };
        let cut = match hidden {
            Some(hidden) if shown && index + 1 == hidden.kept_lines => hidden.cut,
            _ => None,
        };
        let fold = |x: f32, width: f32| match (fold_at, cut) {
            (Some(at), _) => (at, 0.0),
            (None, Some(cut)) if x + width > cut + 0.01 => (cut, 0.0),
            _ => (x, width),
        };
        stretches.clear();
        let mut x = line.metrics().offset;
        for run in line.runs() {
            let mut stretch = Stretch {
                start: run.text_range().start,
                right_to_left: run.is_rtl(),
                characters: Vec::new(),
            };
            for cluster in run.visual_clusters() {
                let advance = cluster.advance();
                let (at, width) = fold(x, advance);
                push_characters(
                    &mut stretch.characters,
                    text,
                    cluster.text_range(),
                    at,
                    width,
                );
                x += advance;
            }
            stretch
                .characters
                .sort_unstable_by_key(|character| character.start);
            stretch.start = stretch
                .characters
                .first()
                .map_or(stretch.start, |character| character.start);
            stretches.push(stretch);
        }
        stretches.sort_unstable_by_key(|stretch| stretch.start);
        let top = origin.y + metrics.block_min_coord;
        let bottom = origin.y + metrics.block_max_coord;
        let mut before = None;
        // A hidden line goes on with the kept line it is folded onto, unless
        // a hard break ends the line before it, which starts a paragraph.
        let mut continues_line = !shown && index > 0 && !after_hard_break;
        for stretch in &stretches {
            for chunk in stretch.characters.chunks(TEXT_RUN_CAP) {
                runs.push(run_of(
                    chunk,
                    text,
                    origin.x,
                    (top, bottom),
                    stretch.right_to_left,
                    continues_line,
                    before,
                ));
                continues_line = true;
                before = chunk.last().map(|character| character.space);
            }
        }
        after_hard_break = line.break_reason() == parley::BreakReason::Explicit;
    }
    runs
}

/// Returns the part of a selection scope's selection that `range` of a
/// block's text, `len` bytes long, holds, from its start to its end, or
/// `None` when it holds none of the text. A range that runs on past the
/// text, as one over the whole block does, ends with it. The scope keeps
/// no direction per block, so the focus is the end, the side a selection
/// made downward moves.
pub(crate) fn scope_selection(range: &std::ops::Range<usize>, len: usize) -> Option<TextSelection> {
    let (start, end) = (range.start.min(len), range.end.min(len));
    (start < end).then_some(TextSelection {
        anchor: TextOffset {
            byte: start,
            ends_line: false,
        },
        focus: TextOffset {
            byte: end,
            ends_line: false,
        },
    })
}

/// Appends the characters of the cluster that covers `range` of `text` and
/// starts at `x` with `width`, splitting one too long for a byte to count.
fn push_characters(
    characters: &mut Vec<Character>,
    text: &str,
    range: std::ops::Range<usize>,
    x: f32,
    width: f32,
) {
    let slice = text
        .get(range.clone())
        .expect("a cluster covers a stretch of the text it was laid out from");
    let space = slice.chars().all(char::is_whitespace);
    let mut start = range.start;
    let mut width = width;
    while start < range.end {
        let mut end = (start + usize::from(u8::MAX)).min(range.end);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        characters.push(Character {
            start,
            len: u8::try_from(end - start).expect("a piece holds at most 255 bytes"),
            x,
            width,
            space,
        });
        start = end;
        width = 0.0;
    }
}

/// Builds the run of `characters`, consecutive characters of one stretch
/// of a line that spans `rows`, top to bottom.
///
/// `before` says whether the character before the first one is white
/// space, or is `None` when the first one starts the line, which always
/// starts a word.
fn run_of(
    characters: &[Character],
    text: &str,
    origin_x: f32,
    (top, bottom): (f32, f32),
    right_to_left: bool,
    continues_line: bool,
    before: Option<bool>,
) -> TextRun {
    let left = characters
        .iter()
        .map(|character| character.x)
        .fold(f32::INFINITY, f32::min);
    let right = characters
        .iter()
        .map(|character| character.x + character.width)
        .fold(f32::NEG_INFINITY, f32::max);
    let left = if left.is_finite() { left } else { 0.0 };
    let right = right.max(left);
    let mut run = TextRun {
        start: characters.first().map_or(0, |character| character.start),
        text: String::new(),
        character_lengths: Vec::with_capacity(characters.len()),
        word_starts: Vec::new(),
        character_positions: Vec::with_capacity(characters.len()),
        character_widths: Vec::with_capacity(characters.len()),
        bounds: Rect::new(origin_x + left, top, origin_x + right, bottom),
        right_to_left,
        continues_line,
    };
    let mut previous = before;
    for (index, character) in characters.iter().enumerate() {
        let end = character.start + usize::from(character.len);
        run.text.push_str(&text[character.start..end]);
        run.character_lengths.push(character.len);
        run.character_positions.push(if right_to_left {
            right - (character.x + character.width)
        } else {
            character.x - left
        });
        run.character_widths.push(character.width);
        let starts_word = match previous {
            None => true,
            Some(space) => space && !character.space,
        };
        if starts_word {
            run.word_starts
                .push(u8::try_from(index).expect("a run holds at most 255 characters"));
        }
        previous = Some(character.space);
    }
    run
}

#[cfg(test)]
#[path = "a11y_tests.rs"]
mod tests;
