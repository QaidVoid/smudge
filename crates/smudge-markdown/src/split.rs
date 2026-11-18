//! Where a growing markdown source divides into blocks that no later text
//! can change.
//!
//! The splitter reads complete lines one at a time and cuts between the
//! top-level blocks the parser finds in them. A cut is kept only where the
//! two sides, each parsed with the other's definitions, add up to the parse
//! of both together, and none is made after a line that opens a bracket
//! until a blank line arrives, since a link label closed later can turn
//! the lines between into one definition. Every block but the last is then
//! final.

use pulldown_cmark::{Event, Parser};

use crate::model::{self, options};

/// Finds the starts of the top-level blocks of a source that only grows.
#[derive(Debug, Default)]
pub(crate) struct Splitter {
    /// Where the open block starts, which is where the next scan begins.
    open: usize,
    /// How far the complete lines the splitter has read reach.
    read: usize,
}

impl Splitter {
    /// Where the open block, the one the next text can still change,
    /// starts.
    #[cfg(test)]
    pub(crate) fn open(&self) -> usize {
        self.open
    }

    /// Reads `text`, the whole source so far, which extends the text of
    /// every earlier call, and pushes onto `starts` the start of each block
    /// it opened, in order. Each new start closes the block before it.
    ///
    /// Only complete lines count, since a line whose end has not arrived
    /// can still turn into a setext underline or a list item. The splitter
    /// reads them one at a time, whatever the pieces the text arrived in,
    /// so every way of streaming a text divides it alike.
    pub(crate) fn advance(&mut self, text: &str, starts: &mut Vec<usize>) {
        let Some(complete) = complete_lines(text) else {
            return;
        };
        while self.read < complete {
            let line_end = text[self.read..complete]
                .find('\n')
                .map_or(complete, |at| self.read + at + 1);
            self.read = line_end;
            self.decide(&text[..line_end], starts);
        }
    }

    /// Finds the blocks the complete lines of `text` close, past the open
    /// block's start.
    fn decide(&mut self, text: &str, starts: &mut Vec<usize>) {
        let base = self.open;
        let region = &text[base..];
        let mut depth = 0usize;
        let mut tops = Vec::new();
        for (event, range) in Parser::new_ext(region, options()).into_offset_iter() {
            match event {
                Event::Start(_) => {
                    if depth == 0 {
                        tops.push(range);
                    }
                    depth += 1;
                }
                Event::End(_) => depth = depth.saturating_sub(1),
                _ if depth == 0 => tops.push(range),
                _ => {}
            }
        }
        // The parser's ranges are close to a block's edges but not always
        // on them: a definition before a paragraph belongs to the
        // paragraph, whose content starts after it, and the range of a
        // list that ends in an empty item can reach into the block after
        // it. So each cut is checked: the two sides, each parsed against
        // the other's definitions, must add up to the parse of both
        // together. The candidates come in a fixed order, the line after
        // the previous block, then the line the next block's content
        // starts on, then each line before that, back to the previous
        // block. When none holds, the two blocks stay one.
        //
        // A line that opens a bracket can start a link label that a later
        // line closes, which turns every line between into one definition.
        // A label holds no blank line, so past the last blank line such a
        // line keeps every cut after it open.
        let settled = last_blank_end(region);
        let mut from = 0;
        for (index, pair) in tops.windows(2).enumerate() {
            let after = line_start(region, pair[0].end);
            let own = line_begin(region, pair[1].start);
            let first = line_begin(region, pair[0].start).max(from);
            let mut candidates = vec![after, own];
            let mut line = own;
            while line > first {
                line = line_begin(region, line - 1);
                candidates.push(line);
            }
            let kept = index + 1
                - tops[..index + 1]
                    .iter()
                    .filter(|top| top.start < from)
                    .count();
            let cut = candidates
                .into_iter()
                .filter(|&line| line > from && line < region.len())
                .filter(|&line| !opens_bracket(&region[from.max(settled).min(line)..line]))
                .find(|&line| cuts_cleanly(&region[from..], line - from, kept));
            if let Some(line) = cut {
                starts.push(base + line);
                self.open = base + line;
                from = line;
            }
        }
    }
}

/// Whether cutting `text` at `at` leaves `blocks` top-level blocks before
/// the cut and parses, on both sides, as `text` does whole. Each side
/// reads the other's definitions, as the document gives them.
fn cuts_cleanly(text: &str, at: usize, blocks: usize) -> bool {
    let (head, tail) = text.split_at(at);
    let head_alone = model::parse(head, 0, &[], &[]);
    let tail = model::parse(tail, at, &head_alone.definitions, &[]);
    let head = model::parse(head, 0, &[], &tail.definitions);
    head.nodes.len() == blocks && {
        let mut both = head.nodes;
        both.extend(tail.nodes);
        both == model::document(text)
    }
}

/// Where the line after the last blank line of `text` starts, or 0 when
/// `text` holds no blank line. A blank line holds spaces and tabs alone,
/// as CommonMark counts it, and not other whitespace.
fn last_blank_end(text: &str) -> usize {
    let mut end = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        at += line.len();
        if line
            .trim_end_matches('\n')
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
        {
            end = at;
        }
    }
    end
}

/// Whether a line of `lines`, past its indent and any container markers,
/// opens a bracket, and so can start a link label.
fn opens_bracket(lines: &str) -> bool {
    lines.lines().any(|line| {
        line.trim_start_matches(|c: char| " \t>-*+.)".contains(c) || c.is_ascii_digit())
            .starts_with('[')
    })
}

/// The length of the complete lines at the start of `text`, or `None`
/// when no line has ended. The document turns every line ending into a
/// line feed before the splitter sees it.
fn complete_lines(text: &str) -> Option<usize> {
    text.rfind('\n').map(|at| at + 1)
}

/// The start of the line `at` falls in, when `at` starts one, or else of
/// the line after it, in `text`, which ends with a complete line.
fn line_start(text: &str, at: usize) -> usize {
    if at == 0 || text.as_bytes()[at - 1] == b'\n' {
        return at;
    }
    text[at..]
        .find('\n')
        .map_or(text.len(), |newline| at + newline + 1)
}

/// The start of the line `at` falls in, in `text`.
fn line_begin(text: &str, at: usize) -> usize {
    text[..at.min(text.len())]
        .rfind('\n')
        .map_or(0, |newline| newline + 1)
}

/// Divides a complete source the way a [`Splitter`] that read it in any
/// number of pieces does: the start of every block, the first at zero.
#[allow(
    dead_code,
    reason = "the tests and the fuzz target, which reads this module by path, call it"
)]
pub(crate) fn split(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    Splitter::default().advance(text, &mut starts);
    starts
}

#[cfg(test)]
#[path = "split_tests.rs"]
mod tests;
