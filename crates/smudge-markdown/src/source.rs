//! The growing markdown document a view shows, with the signals its rows
//! read.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use smudge_core::reactive::{Owner, RwSignal, prelude::*};

use crate::document::Document;
use crate::model::{Inline, Node};

/// A markdown document that grows, such as a reply streamed in tokens,
/// which [`markdown`](crate::markdown) views show.
///
/// Clones share one document. [`MarkdownSource::push`] appends text and
/// parses only what it can change: the block still open at the end, and
/// the blocks that cite a label whose definition the push changed. Every
/// other block keeps its parse, and its row in a view keeps its widgets.
///
/// ```
/// use smudge_markdown::MarkdownSource;
///
/// let reply = MarkdownSource::new();
/// reply.push("# Answer\n\nThe build ");
/// reply.push("passed.\n");
/// assert_eq!(reply.text(), "# Answer\n\nThe build passed.\n");
/// ```
#[derive(Clone)]
pub struct MarkdownSource {
    /// The document, shared by every clone.
    shared: Rc<Shared>,
}

/// The state every clone of a source shares.
struct Shared {
    /// The owner of every signal the source creates.
    owner: Owner,
    /// The owner of the current document's block signals, a child of
    /// `owner` that [`MarkdownSource::clear`] replaces, which disposes the
    /// signals of the document before.
    blocks_owner: RefCell<Owner>,
    /// How many times the source was cleared, which tells the rows and
    /// leaves of one document from those of the next.
    generation: Cell<u32>,
    /// The source and its blocks.
    document: RefCell<Document>,
    /// The nodes of each block, in the order of the document's blocks, as
    /// signals the rows read.
    nodes: RefCell<Vec<RwSignal<Rc<[Node]>>>>,
    /// The leaves of each block, in the same order.
    paths: RefCell<Vec<Rc<[Leaf]>>>,
    /// The path of each leaf of a block, by block, made when a copy first
    /// asks for one and dropped when the block changes.
    lookups: RefCell<Vec<Option<LeafPaths>>>,
    /// The key of every block, which the list keys its rows by.
    keys: RwSignal<Rc<[BlockKey]>>,
    /// Every selectable leaf of the document, in reading order.
    leaves: RwSignal<Rc<[Leaf]>>,
    /// How many blocks, from the first, are complete: every closed block,
    /// and the open one too once [`MarkdownSource::finish`] ran.
    complete: RwSignal<usize>,
}

/// The path of each leaf of one block, by leaf.
type LeafPaths = HashMap<Leaf, Rc<[u32]>>;

/// What names a block for as long as it exists: its start in the source,
/// and the generation of the document, so a block of the next document at
/// the same start is another block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BlockKey {
    /// How many times the source was cleared before the block opened.
    pub(crate) generation: u32,
    /// Where the block starts in the source.
    pub(crate) start: usize,
}

/// One run of text a selection can cover: a heading, a paragraph, a code
/// block or a table cell. It is named by where its text starts in the
/// source, which stays the same when the block that holds it divides, so a
/// selection in the open block survives the block closing around it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Leaf {
    /// How many times the source was cleared before the leaf appeared.
    pub(crate) generation: u32,
    /// Where the leaf starts in the source: the start of the table that
    /// holds a cell, and the start of its list item for the first leaf of
    /// an item.
    pub(crate) at: usize,
    /// The row and the column of a table cell, the header row first.
    pub(crate) cell: Option<(u32, u32)>,
}

impl Default for MarkdownSource {
    fn default() -> Self {
        MarkdownSource::new()
    }
}

impl MarkdownSource {
    /// Creates an empty document.
    ///
    /// Its signals live under an owner made under the current one, so the
    /// source lasts as long as the owner current here, or for as long as
    /// a clone of it is kept when no owner is current.
    pub fn new() -> MarkdownSource {
        let owner = Owner::new();
        let blocks_owner = owner.child();
        let first = blocks_owner.with(|| RwSignal::new(Rc::from([])));
        let (keys, leaves, complete) = owner.with(|| {
            (
                RwSignal::new(Rc::from([BlockKey {
                    generation: 0,
                    start: 0,
                }])),
                RwSignal::new(Rc::from([])),
                RwSignal::new(0),
            )
        });
        MarkdownSource {
            shared: Rc::new(Shared {
                owner,
                blocks_owner: RefCell::new(blocks_owner),
                generation: Cell::new(0),
                document: RefCell::new(Document::default()),
                nodes: RefCell::new(vec![first]),
                paths: RefCell::new(vec![Rc::from([])]),
                lookups: RefCell::new(vec![None]),
                keys,
                leaves,
                complete,
            }),
        }
    }

    /// The key of the block that starts at `start` in the current document.
    fn key(&self, start: usize) -> BlockKey {
        BlockKey {
            generation: self.shared.generation.get(),
            start,
        }
    }

    /// Appends `text` to the document. Every line ending in it, a carriage
    /// return or a carriage return and line feed, becomes a line feed,
    /// even when the two arrive in separate pushes.
    pub fn push(&self, text: &str) {
        let changes = self.shared.document.borrow_mut().push(text);
        if changes.changed.is_empty() {
            return;
        }
        let document = self.shared.document.borrow();
        let blocks = document.blocks();
        let mut nodes = self.shared.nodes.borrow_mut();
        let mut paths = self.shared.paths.borrow_mut();
        let mut lookups = self.shared.lookups.borrow_mut();
        let owner = self.shared.blocks_owner.borrow();
        while nodes.len() < blocks.len() {
            nodes.push(owner.with(|| RwSignal::new(Rc::from([]))));
            paths.push(Rc::from([]));
            lookups.push(None);
        }
        let mut leaves_changed = changes.opened > 0;
        for &index in &changes.changed {
            let block = &blocks[index];
            nodes[index].set(Rc::clone(&block.nodes));
            lookups[index] = None;
            let leaves = leaves_of(self.shared.generation.get(), &block.nodes);
            if *leaves != *paths[index] {
                paths[index] = leaves;
                leaves_changed = true;
            }
        }
        if changes.opened > 0 {
            let keys: Rc<[BlockKey]> = blocks.iter().map(|block| self.key(block.start)).collect();
            self.shared.keys.set(keys);
        }
        if leaves_changed {
            let all: Rc<[Leaf]> = paths
                .iter()
                .flat_map(|leaves| leaves.iter().cloned())
                .collect();
            self.shared.leaves.set(all);
        }
        if changes.opened > 0 {
            self.complete_up_to(blocks_len(self) - 1);
        }
    }

    /// Marks every block so far complete, the last one included, for a
    /// view that announces blocks as they complete: call it when a streamed
    /// reply ends, so its last block is spoken too.
    ///
    /// A later push continues the document as usual. Text it adds to the
    /// block that was last when this ran is not announced again.
    pub fn finish(&self) {
        self.complete_up_to(blocks_len(self));
    }

    /// Raises the count of complete blocks to `count`, when it is lower.
    fn complete_up_to(&self, count: usize) {
        if self.shared.complete.read(|complete| *complete < count) {
            self.shared.complete.set(count);
        }
    }

    /// Empties the document, for a new one to be pushed. Every row of a
    /// view mounts again for the next document, and a selection in the
    /// last one ends.
    pub fn clear(&self) {
        let generation = self.shared.generation.get().wrapping_add(1);
        self.shared.generation.set(generation);
        let blocks_owner = self.shared.owner.child();
        let first = blocks_owner.with(|| RwSignal::new(Rc::from([])));
        *self.shared.blocks_owner.borrow_mut() = blocks_owner;
        *self.shared.document.borrow_mut() = Document::default();
        *self.shared.nodes.borrow_mut() = vec![first];
        *self.shared.paths.borrow_mut() = vec![Rc::from([])];
        *self.shared.lookups.borrow_mut() = vec![None];
        self.shared.keys.set(Rc::from([self.key(0)]));
        self.shared.leaves.set(Rc::from([]));
        self.shared.complete.set(0);
    }

    /// The document's text so far, with every line ending a line feed.
    pub fn text(&self) -> String {
        String::from(self.shared.document.borrow().text())
    }

    /// The key of every block, which tracks the signal the list reads.
    pub(crate) fn keys(&self) -> Rc<[BlockKey]> {
        self.shared.keys.get()
    }

    /// How many blocks, from the first, are complete, which tracks the
    /// signal it reads.
    pub(crate) fn complete(&self) -> usize {
        self.shared.complete.get()
    }

    /// How many blocks are complete, without tracking.
    pub(crate) fn complete_untracked(&self) -> usize {
        self.shared.complete.read(|complete| *complete)
    }

    /// How many times the source was cleared, which tells one document
    /// from the next.
    pub(crate) fn generation(&self) -> u32 {
        self.shared.generation.get()
    }

    /// The text of the block at `index`, its leaves joined by line feeds,
    /// or an empty string when there is no such block.
    pub(crate) fn block_text(&self, index: usize) -> String {
        let leaves = self
            .shared
            .paths
            .borrow()
            .get(index)
            .cloned()
            .unwrap_or_default();
        let texts: Vec<String> = leaves
            .iter()
            .map(|leaf| self.leaf_text(leaf))
            .filter(|text| !text.is_empty())
            .collect();
        texts.join("\n")
    }

    /// Every selectable leaf, which tracks the signal the selection reads.
    pub(crate) fn leaves(&self) -> Rc<[Leaf]> {
        self.shared.leaves.get()
    }

    /// The nodes of the block `key` names, as the signal its row reads, or
    /// `None` once the block is gone.
    pub(crate) fn nodes(&self, key: BlockKey) -> Option<RwSignal<Rc<[Node]>>> {
        if key.generation != self.shared.generation.get() {
            return None;
        }
        let index = self.shared.document.borrow().index_of(key.start)?;
        self.shared.nodes.borrow().get(index).cloned()
    }

    /// The text `leaf` shows, or an empty string once it is gone.
    pub(crate) fn leaf_text(&self, leaf: &Leaf) -> String {
        if leaf.generation != self.shared.generation.get() {
            return String::new();
        }
        let Some(index) = self.shared.document.borrow().index_holding(leaf.at) else {
            return String::new();
        };
        let Some(nodes) = self.shared.nodes.borrow().get(index).cloned() else {
            return String::new();
        };
        let mut lookups = self.shared.lookups.borrow_mut();
        let Some(lookup) = lookups.get_mut(index) else {
            return String::new();
        };
        nodes.read(|nodes| {
            let lookup = lookup.get_or_insert_with(|| {
                let mut lookup = HashMap::new();
                collect_leaves(
                    leaf.generation,
                    nodes,
                    &mut Vec::new(),
                    &mut |leaf, path| {
                        lookup.entry(leaf).or_insert_with(|| Rc::from(path));
                    },
                );
                lookup
            });
            lookup
                .get(leaf)
                .and_then(|path| inlines_at(nodes, path))
                .map(|inlines| inlines.iter().map(|inline| inline.text.as_str()).collect())
                .unwrap_or_default()
        })
    }
}

/// How many blocks the document of `source` has, the open one included.
fn blocks_len(source: &MarkdownSource) -> usize {
    source.shared.document.borrow().blocks().len()
}

/// The leaf at `path` in `nodes`, of the document of `generation`, or
/// `None` when no leaf is there.
pub(crate) fn leaf_at(generation: u32, nodes: &[Node], path: &[u32]) -> Option<Leaf> {
    let leaf = |at: usize, cell: Option<(u32, u32)>| Leaf {
        generation,
        at,
        cell,
    };
    match node_at(nodes, path) {
        Some(node) => node_start(node).map(|at| leaf(at, None)),
        None => {
            let (table, cell) = path.split_at(path.len().checked_sub(2)?);
            match node_at(nodes, table)? {
                Node::Table { at, .. } => Some(leaf(*at, Some((cell[0], cell[1])))),
                _ => None,
            }
        }
    }
}

/// Where a node starts in the source, or `None` for a thematic break,
/// which holds nothing that needs a widget of its own.
pub(crate) fn node_start(node: &Node) -> Option<usize> {
    match node {
        Node::Heading { at, .. }
        | Node::Paragraph { at, .. }
        | Node::Text { at, .. }
        | Node::Code { at, .. }
        | Node::Html { at, .. }
        | Node::Table { at, .. }
        | Node::List { at, .. }
        | Node::Quote { at, .. } => Some(*at),
        Node::Rule => None,
    }
}

/// The selectable leaves of `nodes`, of the document of `generation`, in
/// reading order.
fn leaves_of(generation: u32, nodes: &[Node]) -> Rc<[Leaf]> {
    let mut out = Vec::new();
    collect_leaves(generation, nodes, &mut Vec::new(), &mut |leaf, _| {
        out.push(leaf)
    });
    Rc::from(out)
}

/// Calls `found` with each leaf of `nodes`, whose path is `path`, and the
/// leaf's own path, in reading order.
fn collect_leaves(
    generation: u32,
    nodes: &[Node],
    path: &mut Vec<u32>,
    found: &mut impl FnMut(Leaf, &[u32]),
) {
    for (index, node) in nodes.iter().enumerate() {
        path.push(index as u32);
        match node {
            Node::Heading { at, .. }
            | Node::Paragraph { at, .. }
            | Node::Text { at, .. }
            | Node::Code { at, .. }
            | Node::Html { at, .. } => found(
                Leaf {
                    generation,
                    at: *at,
                    cell: None,
                },
                path,
            ),
            Node::List { items, .. } => {
                for (item, children) in items.iter().enumerate() {
                    path.push(item as u32);
                    collect_leaves(generation, children, path, found);
                    path.pop();
                }
            }
            Node::Quote { children, .. } => collect_leaves(generation, children, path, found),
            Node::Table { at, head, rows } => {
                let all = std::iter::once(head).chain(rows.iter());
                for (row, cells) in all.enumerate() {
                    for column in 0..cells.len() {
                        let cell = (row as u32, column as u32);
                        path.extend([cell.0, cell.1]);
                        found(
                            Leaf {
                                generation,
                                at: *at,
                                cell: Some(cell),
                            },
                            path,
                        );
                        path.truncate(path.len() - 2);
                    }
                }
            }
            Node::Rule => {}
        }
        path.pop();
    }
}

/// The nodes a container at `path` holds: the top-level nodes for an
/// empty path, an item's nodes after a list and an item index, and a
/// quote's nodes.
pub(crate) fn children_at<'a>(nodes: &'a [Node], path: &[u32]) -> Option<&'a [Node]> {
    let mut list = nodes;
    let mut at = 0;
    while at < path.len() {
        match list.get(path[at] as usize)? {
            Node::List { items, .. } => {
                list = items.get(*path.get(at + 1)? as usize)?;
                at += 2;
            }
            Node::Quote { children, .. } => {
                list = children;
                at += 1;
            }
            _ => return None,
        }
    }
    Some(list)
}

/// The node at `path`, the last index naming it in its container.
pub(crate) fn node_at<'a>(nodes: &'a [Node], path: &[u32]) -> Option<&'a Node> {
    let (last, container) = path.split_last()?;
    children_at(nodes, container)?.get(*last as usize)
}

/// The inline runs of the leaf at `path`: a code or HTML block's text is
/// one run.
pub(crate) fn inlines_at(nodes: &[Node], path: &[u32]) -> Option<Vec<Inline>> {
    let plain = |text: &str, code: bool| {
        vec![Inline {
            text: String::from(text),
            code,
            ..Inline::default()
        }]
    };
    match node_at(nodes, path) {
        Some(
            Node::Heading { text, .. } | Node::Paragraph { text, .. } | Node::Text { text, .. },
        ) => Some(text.clone()),
        // A code or HTML block's text ends with the newline of its last
        // line, which would show as an empty line.
        Some(Node::Code { text, .. }) => Some(plain(text.strip_suffix('\n').unwrap_or(text), true)),
        Some(Node::Html { text, .. }) => {
            Some(plain(text.strip_suffix('\n').unwrap_or(text), false))
        }
        Some(_) => None,
        None => {
            let (table, cell) = path.split_at(path.len().checked_sub(2)?);
            let Node::Table { head, rows, .. } = node_at(nodes, table)? else {
                return None;
            };
            let row = match cell[0] {
                0 => head,
                row => rows.get(row as usize - 1)?,
            };
            row.get(cell[1] as usize).cloned()
        }
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
