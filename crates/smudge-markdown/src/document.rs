//! A markdown source that grows, divided into blocks that each parse again
//! only while they are open or when the definitions they read change.

use std::rc::Rc;

use crate::model::{self, Definition, Node};
use crate::split::Splitter;

/// The source and the blocks it divides into.
#[derive(Debug)]
pub(crate) struct Document {
    /// Everything pushed so far.
    text: String,
    /// The block boundaries found so far.
    splitter: Splitter,
    /// Every block, closed ones first and the open one last.
    blocks: Vec<Block>,
    /// The definitions of every closed block, in order.
    closed_definitions: Vec<Definition>,
    /// Whether the last push ended in a carriage return, whose line feed,
    /// if one comes, is part of the same line ending.
    after_return: bool,
}

/// One block of the source.
#[derive(Debug)]
pub(crate) struct Block {
    /// Where the block starts in the source, which names it for as long as
    /// the source keeps growing.
    pub(crate) start: usize,
    /// What the block renders as.
    pub(crate) nodes: Rc<[Node]>,
    /// The definitions the block holds.
    definitions: Vec<Definition>,
    /// The labels the block's last parse looked up, so only a definition
    /// of one of them can change it.
    labels: Vec<String>,
}

impl Block {
    /// A block at `start` that has not been parsed.
    fn new(start: usize) -> Block {
        Block {
            start,
            nodes: Rc::from([]),
            definitions: Vec::new(),
            labels: Vec::new(),
        }
    }
}

/// What one push changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Changes {
    /// How many blocks the push opened.
    pub(crate) opened: usize,
    /// The index of every block whose nodes changed, in order, and the
    /// last block's when the push opened any.
    pub(crate) changed: Vec<usize>,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            text: String::new(),
            splitter: Splitter::default(),
            blocks: vec![Block::new(0)],
            closed_definitions: Vec::new(),
            after_return: false,
        }
    }
}

impl Document {
    /// The source so far, with every line ending a line feed.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The blocks, closed ones first and the open one last.
    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The index of the block that starts at `start`, if one does.
    pub(crate) fn index_of(&self, start: usize) -> Option<usize> {
        self.blocks
            .binary_search_by_key(&start, |block| block.start)
            .ok()
    }

    /// The index of the block whose source holds `offset`, if one does.
    pub(crate) fn index_holding(&self, offset: usize) -> Option<usize> {
        self.blocks
            .partition_point(|block| block.start <= offset)
            .checked_sub(1)
    }

    /// The source of the block at `index`.
    fn block_text(&self, index: usize) -> &str {
        let start = self.blocks[index].start;
        let end = self
            .blocks
            .get(index + 1)
            .map_or(self.text.len(), |next| next.start);
        &self.text[start..end]
    }

    /// Appends `chunk` to the source and parses what it changed: the
    /// blocks it closed, the open block, and, when it changed a definition,
    /// every block that looked that definition's label up.
    pub(crate) fn push(&mut self, chunk: &str) -> Changes {
        if chunk.is_empty() {
            return Changes::default();
        }
        // CommonMark ends a line at a line feed, a carriage return or both,
        // and the parser reads past a lone carriage return when it decides
        // a line's meaning, so every ending becomes a line feed here. A
        // carriage return at the end of a push already ended its line, and
        // a line feed opening the next push belongs to the same ending.
        let chunk = if std::mem::take(&mut self.after_return) {
            chunk.strip_prefix('\n').unwrap_or(chunk)
        } else {
            chunk
        };
        if chunk.is_empty() {
            return Changes::default();
        }
        self.after_return = chunk.ends_with('\r');
        let normalized;
        let chunk = if chunk.contains('\r') {
            normalized = normalize_line_endings(chunk);
            normalized.as_str()
        } else {
            chunk
        };
        let was_open = self.blocks.len() - 1;
        let defined_before = self.blocks[was_open].definitions.clone();
        self.text.push_str(chunk);
        let mut starts = Vec::new();
        if chunk.contains('\n') {
            self.splitter.advance(&self.text, &mut starts);
        }
        let opened = starts.len();
        for start in starts {
            self.blocks.push(Block::new(start));
        }
        let mut changed = Vec::new();
        // The blocks that closed parse against the definitions before
        // them. The open block's definitions are not known yet, and when
        // one of them is new, the sweep below parses them again.
        let closed = std::mem::take(&mut self.closed_definitions);
        let mut closed_count = closed.len();
        let mut before = closed;
        for index in was_open..self.blocks.len() - 1 {
            if self.reparse(index, &before, &[]) {
                changed.push(index);
            }
            before.extend(self.blocks[index].definitions.iter().cloned());
        }
        let open = self.blocks.len() - 1;
        if self.reparse(open, &before, &[]) || opened > 0 {
            changed.push(open);
        }
        // A block that closed may read a definition of a block after it,
        // which can be one the push moved out of it: the definitions are
        // the same, so the sweep below would not run for it.
        for index in was_open..open {
            let own = self.blocks[index].definitions.len();
            if !self.blocks[index].labels.is_empty() {
                let after: Vec<Definition> = self.blocks[index + 1..]
                    .iter()
                    .flat_map(|block| block.definitions.iter().cloned())
                    .collect();
                if !after.is_empty() && self.reparse(index, &before[..closed_count], &after) {
                    changed.push(index);
                }
            }
            closed_count += own;
        }
        self.closed_definitions = before;
        let defined_after: Vec<Definition> = self.blocks[was_open..]
            .iter()
            .flat_map(|block| block.definitions.iter().cloned())
            .collect();
        if defined_after != defined_before {
            // Only a definition that appeared or went away can change what
            // a label resolves to, and an edited one does both.
            let mut touched: Vec<Definition> = defined_before
                .iter()
                .filter(|definition| !defined_after.contains(definition))
                .cloned()
                .collect();
            touched.extend(
                defined_after
                    .iter()
                    .filter(|definition| !defined_before.contains(definition))
                    .cloned(),
            );
            self.sweep(&touched, &mut changed);
        }
        changed.sort_unstable();
        changed.dedup();
        Changes { opened, changed }
    }

    /// Parses again every block that looked up a label one of `touched`
    /// defines, against the definitions of all the others, and adds the
    /// index of each one whose nodes changed to `changed`.
    fn sweep(&mut self, touched: &[Definition], changed: &mut Vec<usize>) {
        let readers: Vec<usize> = model::with_defined(touched, |defined| {
            self.blocks
                .iter()
                .enumerate()
                .filter(|(_, block)| block.labels.iter().any(|label| defined(label)))
                .map(|(index, _)| index)
                .collect()
        });
        for index in readers {
            let around = |blocks: &[Block]| -> Vec<Definition> {
                blocks
                    .iter()
                    .flat_map(|block| block.definitions.iter().cloned())
                    .collect()
            };
            let before = around(&self.blocks[..index]);
            let after = around(&self.blocks[index + 1..]);
            if self.reparse(index, &before, &after) {
                changed.push(index);
            }
        }
    }

    /// Parses the block at `index` against the definitions `before` and
    /// `after` it, and answers whether its nodes changed.
    fn reparse(&mut self, index: usize, before: &[Definition], after: &[Definition]) -> bool {
        let start = self.blocks[index].start;
        let parsed = model::parse(self.block_text(index), start, before, after);
        let block = &mut self.blocks[index];
        block.labels = parsed.labels;
        block.definitions = parsed.definitions;
        if *block.nodes == *parsed.nodes {
            return false;
        }
        block.nodes = Rc::from(parsed.nodes);
        true
    }
}

/// `text` with every carriage return and line feed pair, and every lone
/// carriage return, turned into a line feed.
pub(crate) fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
