//! Selection across read-only text blocks.
//!
//! A [`selection_scope`] holds one selection over many labels and rich text
//! paragraphs, the blocks, in the order of a list of keys. The selection's
//! ends are a block's key and a byte offset into its text, so a selection
//! survives scrolling and virtualization: a block that mounts again finds
//! its part from the scope, and copy reads the text of blocks that are not
//! mounted through the scope's `text_of`.
//!
//! The scope takes the presses and holds the keyboard focus, which copy
//! needs. A block opts in with [`SelectableExt::selectable`] and listens with
//! [`Element::on_drag_over`], turning the point under a drag into an offset
//! with its own layout, so the hit test runs in the block's own coordinates
//! wherever a scroll view moved it. A drag held past the edge of a scroll
//! view inside the scope scrolls it, and the rows that pass the edge extend
//! the selection.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;
use std::rc::Rc;

use parley::editing::Selection as ParagraphSelection;
use parley::layout::Affinity;
use parley::{Brush, Cursor};
use smudge_core::element::{Element, IntoElement, Value, show};
use smudge_core::geometry::{Color, Corners, Point, Rect};
use smudge_core::input::{Drag, Handled, Key, KeyEvent};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{
    Owner, RenderEffect, RwSignal, prelude::*, provide_context, use_context,
};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{PaintCx, Panel, panel};
use web_time::Instant;

use crate::chrome::{DOUBLE_CLICK_DISTANCE, DOUBLE_CLICK_TIME, SELECTION_ALPHA};
use crate::label::Label;
use crate::layout::TextLayout;
use crate::rich_text::{LayoutSink, RichText};

/// One end of a selection: a block and a byte offset into its text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Endpoint<K> {
    /// The key of the block.
    pub(crate) key: K,
    /// The byte offset into the block's text.
    pub(crate) offset: usize,
}

/// What a drag found under the pointer: the block, the offset there, and the
/// word around it.
struct Hit<K> {
    /// The offset under the pointer.
    at: Endpoint<K>,
    /// The byte range of the word under the pointer.
    word: Range<usize>,
}

/// The order of a scope's blocks: their keys, and the position of each key,
/// built once each time the keys change, so a drag looks a key up instead of
/// scanning for it.
pub(crate) struct Order<K> {
    /// The keys in order.
    keys: Rc<[K]>,
    /// The position of each key in `keys`.
    positions: HashMap<K, usize>,
}

impl<K: Clone + Eq + Hash> Order<K> {
    /// Indexes `keys`. A key listed twice keeps its first position.
    pub(crate) fn new(keys: Rc<[K]>) -> Order<K> {
        let mut positions = HashMap::with_capacity(keys.len());
        for (at, key) in keys.iter().enumerate() {
            positions.entry(key.clone()).or_insert(at);
        }
        Order { keys, positions }
    }

    /// Returns the position of `key`, or `None` when it is not listed.
    fn position(&self, key: &K) -> Option<usize> {
        self.positions.get(key).copied()
    }
}

/// Returns `a` and `b` in block order, then offset order, or `None` when
/// either key is not in `order`.
pub(crate) fn ordered<K: Clone + Eq + Hash>(
    order: &Order<K>,
    a: &Endpoint<K>,
    b: &Endpoint<K>,
) -> Option<(Endpoint<K>, Endpoint<K>)> {
    let first = order.position(&a.key)?;
    let second = order.position(&b.key)?;
    if (first, a.offset) <= (second, b.offset) {
        Some((a.clone(), b.clone()))
    } else {
        Some((b.clone(), a.clone()))
    }
}

/// Returns the byte range the selection from `start` to `end`, in order,
/// covers in the block of `key`, with `usize::MAX` for a block selected to
/// its end, or `None` when it covers nothing of it.
pub(crate) fn block_range<K: Clone + Eq + Hash>(
    order: &Order<K>,
    start: &Endpoint<K>,
    end: &Endpoint<K>,
    key: &K,
) -> Option<Range<usize>> {
    let at = order.position(key)?;
    let first = order.position(&start.key)?;
    let last = order.position(&end.key)?;
    if at < first || at > last {
        return None;
    }
    let from = if at == first { start.offset } else { 0 };
    let to = if at == last { end.offset } else { usize::MAX };
    (from < to).then_some(from..to)
}

/// Joins the selected text of every block from `start` to `end`, in order,
/// with a newline between blocks, reading each block's text through
/// `text_of`.
pub(crate) fn join<K: Clone + Eq + Hash>(
    order: &Order<K>,
    start: &Endpoint<K>,
    end: &Endpoint<K>,
    text_of: impl Fn(&K) -> String,
) -> String {
    let (Some(first), Some(last)) = (order.position(&start.key), order.position(&end.key)) else {
        return String::new();
    };
    let mut joined = String::new();
    for (at, key) in order.keys.iter().enumerate().take(last + 1).skip(first) {
        let text = text_of(key);
        let from = if at == first { start.offset } else { 0 };
        let to = if at == last { end.offset } else { text.len() };
        let from = floor_boundary(&text, from);
        let to = floor_boundary(&text, to).max(from);
        if at > first {
            joined.push('\n');
        }
        joined.push_str(&text[from..to]);
    }
    joined
}

/// Moves the ends of the selection from `start` to `end`, ordered in `old`,
/// onto the blocks that `new` still lists: an end whose block is gone moves
/// to the nearest surviving block inside the selection, the start to its
/// beginning and the end to its end. Returns `None` when no block between
/// them survives.
pub(crate) fn carried<K: Clone + Eq + Hash>(
    old: &Order<K>,
    new: &Order<K>,
    start: &Endpoint<K>,
    end: &Endpoint<K>,
) -> Option<(Endpoint<K>, Endpoint<K>)> {
    let first = old.position(&start.key)?;
    let last = old.position(&end.key)?;
    let between = &old.keys[first..=last];
    let start = if new.position(&start.key).is_some() {
        start.clone()
    } else {
        let key = between.iter().find(|key| new.position(key).is_some())?;
        Endpoint {
            key: key.clone(),
            offset: 0,
        }
    };
    let end = if new.position(&end.key).is_some() {
        end.clone()
    } else {
        let key = between
            .iter()
            .rev()
            .find(|key| new.position(key).is_some())?;
        Endpoint {
            key: key.clone(),
            offset: usize::MAX,
        }
    };
    ordered(new, &start, &end)
}

/// Returns `offset` clamped into `text` and moved back to a character
/// boundary.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The signal of a mounted block's selected range.
type RangeSignal = RwSignal<Option<Range<usize>>>;

/// The signal of where the keyboard last moved the selection's moving end
/// inside a mounted block: the offset, and whether the caret sits at the end
/// of the line before it rather than the start of the line it begins. A
/// signal notifies on every write, so the block shows the same place again
/// when the end comes back to it.
type RevealSignal = RwSignal<Option<(usize, bool)>>;

/// One step the keyboard moves the selection's moving end by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// One grapheme back.
    Left,
    /// One grapheme on.
    Right,
    /// To the start of the word, or of the word before.
    WordLeft,
    /// To the end of the word, or of the word after.
    WordRight,
    /// One line up.
    Up,
    /// One line down.
    Down,
    /// To the start of the line.
    LineStart,
    /// To the end of the line.
    LineEnd,
}

impl Step {
    /// Whether the step goes toward the start of the text in a block whose
    /// text runs right to left when `rtl` is set. Left and right are visual,
    /// so they go the other way through such a block.
    fn backward(self, rtl: bool) -> bool {
        match self {
            Step::Left | Step::WordLeft => !rtl,
            Step::Right | Step::WordRight => rtl,
            Step::Up | Step::LineStart => true,
            Step::Down | Step::LineEnd => false,
        }
    }

    /// Whether the step crosses into the next block at an edge, which a
    /// move to a line's start or end never does.
    fn crosses(self) -> bool {
        !matches!(self, Step::LineStart | Step::LineEnd)
    }
}

/// What a keyboard step did inside a mounted block.
pub(crate) enum Stepped {
    /// The block has no layout yet, so its text has to answer.
    NoLayout,
    /// The step would leave the block.
    Edge,
    /// The step moved to this selection's focus.
    Moved(ParagraphSelection),
}

/// What a mounted block answers about its own text, from its layout.
pub(crate) trait Nav {
    /// Steps from `offset`, continuing `from` when it ends there, which keeps
    /// the line a caret at a soft wrap belongs to and the column a run of
    /// vertical steps holds.
    fn step(&self, from: Option<ParagraphSelection>, offset: usize, step: Step) -> Stepped;

    /// Returns the caret at `offset`, at the end of the line before it when
    /// `upstream` is set, relative to the block's rectangle.
    fn caret(&self, offset: usize, upstream: bool) -> Option<Rect>;

    /// Returns whether the block's text runs right to left, or `None` before
    /// it has a layout.
    fn rtl(&self) -> Option<bool>;

    /// Returns the length of the block's text, or `None` before it has a
    /// layout.
    fn len(&self) -> Option<usize>;
}

/// Steps from `offset` in `layout`, as [`Nav::step`] describes.
///
/// Left and right, by a character or a word, move through the text in its
/// logical order, toward its start or its end as the arrow points in the
/// paragraph's direction. Moving visually loses its way in a wrapped
/// paragraph whose runs go against its direction, while a selection that
/// grows in text order cannot. Lines, and their starts and ends, move
/// visually.
fn step_in<B: Brush>(
    layout: &parley::Layout<B>,
    carried: Option<ParagraphSelection>,
    offset: usize,
    step: Step,
) -> Stepped {
    let from = carried
        .filter(|selection| selection.focus().index() == offset)
        .unwrap_or_else(|| {
            ParagraphSelection::from_byte_index(layout, offset, Affinity::Downstream)
        });
    let cursor = from.focus();
    let backward = step.backward(layout.is_rtl());
    let logical = |at: Option<usize>| match at.filter(|&at| at != cursor.index()) {
        Some(at) => Stepped::Moved(ParagraphSelection::new(
            from.anchor(),
            Cursor::from_byte_index(layout, at, Affinity::Downstream),
        )),
        None => Stepped::Edge,
    };
    let moved = match step {
        Step::Left | Step::Right if backward => {
            return logical(
                cursor
                    .upstream_cluster(layout)
                    .map(|cluster| cluster.text_range().start),
            );
        }
        Step::Left | Step::Right => {
            return logical(
                cursor
                    .downstream_cluster(layout)
                    .map(|cluster| cluster.text_range().end),
            );
        }
        Step::WordLeft | Step::WordRight if backward => {
            // At the text's end, a word one character long that ends it is
            // a stop of its own, which the logical word move steps past.
            let last_word = cursor
                .downstream_cluster(layout)
                .is_none()
                .then(|| cursor.upstream_cluster(layout))
                .flatten()
                .filter(|cluster| cluster.is_word_boundary() && !cluster.is_space_or_nbsp())
                .map(|cluster| cluster.text_range().start);
            return logical(
                last_word.or_else(|| Some(cursor.previous_logical_word(layout).index())),
            );
        }
        Step::WordLeft | Step::WordRight => {
            return logical(Some(cursor.next_logical_word(layout).index()));
        }
        Step::Up => from.previous_line(layout, true),
        Step::Down => from.next_line(layout, true),
        Step::LineStart => from.line_start(layout, true),
        Step::LineEnd => from.line_end(layout, true),
    };
    if moved.focus() == from.focus() {
        Stepped::Edge
    } else {
        Stepped::Moved(moved)
    }
}

/// Returns the length of the text of `layout`.
fn len_in<B: Brush>(layout: &parley::Layout<B>) -> usize {
    Cursor::from_byte_index(layout, usize::MAX, Affinity::Downstream).index()
}

/// Returns the caret at `offset` in `layout`, at the end of the line before
/// it when `upstream` is set.
fn caret_in<B: Brush>(layout: &parley::Layout<B>, offset: usize, upstream: bool) -> Rect {
    let affinity = if upstream {
        Affinity::Upstream
    } else {
        Affinity::Downstream
    };
    let area = Cursor::from_byte_index(layout, offset, affinity).geometry(layout, 1.0);
    Rect::new(
        area.x0 as f32,
        area.y0 as f32,
        area.x1 as f32,
        area.y1 as f32,
    )
}

/// Returns the offset `step` reaches from `offset` in `text`, a block with
/// no layout because it is not mounted, by characters, by runs of
/// whitespace and the rest, and by hard lines, or `None` when the step would
/// leave the text.
pub(crate) fn step_text(text: &str, offset: usize, step: Step) -> Option<usize> {
    let offset = floor_boundary(text, offset);
    let before = &text[..offset];
    let after = &text[offset..];
    let moved = match step {
        Step::Left => before.char_indices().next_back().map(|(at, _)| at),
        Step::Right => after.chars().next().map(|c| offset + c.len_utf8()),
        Step::WordLeft => {
            let trimmed = before.trim_end();
            (!before.is_empty()).then(|| {
                trimmed.rfind(char::is_whitespace).map_or(0, |at| {
                    at + trimmed[at..].chars().next().map_or(1, char::len_utf8)
                })
            })
        }
        Step::WordRight => {
            let skipped = after.len() - after.trim_start().len();
            (!after.is_empty()).then(|| {
                let rest = &after[skipped..];
                offset + skipped + rest.find(char::is_whitespace).unwrap_or(rest.len())
            })
        }
        Step::Up => before.rfind('\n'),
        Step::Down => after.find('\n').map(|at| offset + at + 1),
        Step::LineStart => Some(before.rfind('\n').map_or(0, |at| at + 1)),
        Step::LineEnd => Some(after.find('\n').map_or(text.len(), |at| offset + at)),
    };
    moved.filter(|&end| end != offset)
}

/// One mounted block of a scope.
struct Block {
    /// The id the block registered with.
    id: u64,
    /// The block's selected range.
    range: RangeSignal,
    /// Where the keyboard moved the selection's end inside the block.
    reveal: RevealSignal,
    /// The block's own answers about its text.
    nav: Rc<dyn Nav>,
}

/// The selection a scope holds, shared with its blocks through reactive
/// context.
pub(crate) struct Scope<K> {
    /// The keys of every block in order, mounted or not.
    keys: Box<dyn Fn() -> Rc<[K]>>,
    /// The order the keys gave when they last changed.
    order: RefCell<Rc<Order<K>>>,
    /// The text of any block, for copy.
    text_of: Box<dyn Fn(&K) -> String>,
    /// The end the selection started from.
    anchor: RefCell<Option<Endpoint<K>>>,
    /// The end the pointer moves.
    focus: RefCell<Option<Endpoint<K>>>,
    /// What the drag found under the pointer last, cleared when the drag
    /// ends, whichever node took its press.
    over: RefCell<Option<Hit<K>>>,
    /// The mounted blocks, by key, then by the id each registered with, since
    /// one key can be mounted more than once.
    blocks: RefCell<HashMap<K, Vec<Block>>>,
    /// The last selection the keyboard stepped to in a mounted block, which
    /// the next step continues while the moving end is still there.
    stepped: RefCell<Option<(K, ParagraphSelection)>>,
    /// The id the next registration takes.
    next_block: Cell<u64>,
    /// Whether a press the scope took is held.
    pressing: Cell<bool>,
    /// When and where the last press began, for a double click.
    last_press: Cell<Option<(Instant, Point)>>,
}

impl<K: Clone + Eq + Hash + 'static> Scope<K> {
    /// Returns the selection's ends in order in `order`, or `None` when it is
    /// empty.
    fn ends_in(&self, order: &Order<K>) -> Option<(Endpoint<K>, Endpoint<K>)> {
        let anchor = self.anchor.borrow();
        let focus = self.focus.borrow();
        let (Some(anchor), Some(focus)) = (anchor.as_ref(), focus.as_ref()) else {
            return None;
        };
        if anchor == focus {
            return None;
        }
        ordered(order, anchor, focus)
    }

    /// Reads the keys again when they changed, carries the selection's ends
    /// onto the blocks that remain, and updates the mounted blocks.
    fn refresh(&self) {
        let keys = (self.keys)();
        let old = Rc::clone(&self.order.borrow());
        if !Rc::ptr_eq(&old.keys, &keys) {
            let new = Rc::new(Order::new(keys));
            // The ends are carried in order and handed back to the anchor and
            // the focus they came from, so a drag keeps moving its own end.
            let anchor_first = self
                .ends_in(&old)
                .is_none_or(|(start, _)| self.anchor.borrow().as_ref() == Some(&start));
            let ends = self
                .ends_in(&old)
                .and_then(|(start, end)| carried(&old, &new, &start, &end));
            let (anchor, focus) = match ends {
                Some((start, end)) if anchor_first => (Some(start), Some(end)),
                Some((start, end)) => (Some(end), Some(start)),
                None if self.pressing.get() => {
                    (self.anchor.borrow().clone(), self.focus.borrow().clone())
                }
                None => (None, None),
            };
            *self.anchor.borrow_mut() = anchor;
            *self.focus.borrow_mut() = focus;
            *self.order.borrow_mut() = new;
        }
        self.update();
    }

    /// Sets the range of every mounted block from the selection, waking only
    /// the blocks whose range changed.
    fn update(&self) {
        let order = Rc::clone(&self.order.borrow());
        let ends = self.ends_in(&order);
        let mut changes: Vec<(RangeSignal, Option<Range<usize>>)> = Vec::new();
        for (key, blocks) in self.blocks.borrow().iter() {
            let range = ends
                .as_ref()
                .and_then(|(start, end)| block_range(&order, start, end, key));
            for block in blocks {
                if block.range.read(|current| *current != range) {
                    changes.push((block.range.clone(), range.clone()));
                }
            }
        }
        for (signal, range) in changes {
            signal.set(range);
        }
    }

    /// Registers a mounted block of `key`, and returns its id and the signal
    /// of its selected range, starting from the selection as it is.
    fn register(&self, key: &K, nav: Rc<dyn Nav>) -> (u64, RangeSignal, RevealSignal) {
        let order = Rc::clone(&self.order.borrow());
        let range = self
            .ends_in(&order)
            .and_then(|(start, end)| block_range(&order, &start, &end, key));
        let range = RwSignal::new(range);
        let reveal = RwSignal::new(None);
        let id = self.next_block.get();
        self.next_block.set(id + 1);
        self.blocks
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .push(Block {
                id,
                range: range.clone(),
                reveal: reveal.clone(),
                nav,
            });
        (id, range, reveal)
    }

    /// Forgets the block of `key` that registered as `id`, leaving any other
    /// block of the same key in place.
    fn unregister(&self, key: &K, id: u64) {
        let mut blocks = self.blocks.borrow_mut();
        if let Some(list) = blocks.get_mut(key) {
            list.retain(|block| block.id != id);
            if list.is_empty() {
                blocks.remove(key);
            }
        }
    }

    /// Starts a selection where the press found a block, selecting the word
    /// there on a double click, or clears it when the press found none.
    fn press(&self, at: Point) {
        self.pressing.set(true);
        let now = smudge_core::timer::now();
        let double = self.last_press.get().is_some_and(|(then, there)| {
            now.duration_since(then) <= DOUBLE_CLICK_TIME
                && (at.x - there.x).powi(2) + (at.y - there.y).powi(2)
                    <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
        });
        self.last_press
            .set(if double { None } else { Some((now, at)) });
        let (anchor, focus) = match self.over.borrow().as_ref() {
            Some(hit) if double => (
                Some(Endpoint {
                    key: hit.at.key.clone(),
                    offset: hit.word.start,
                }),
                Some(Endpoint {
                    key: hit.at.key.clone(),
                    offset: hit.word.end,
                }),
            ),
            Some(hit) => (Some(hit.at.clone()), Some(hit.at.clone())),
            None => (None, None),
        };
        *self.anchor.borrow_mut() = anchor;
        *self.focus.borrow_mut() = focus;
        *self.stepped.borrow_mut() = None;
        self.update();
    }

    /// Records what a drag found under the pointer, and moves the
    /// selection's moving end there while the scope's press is held. A block
    /// calls it each time a drag reaches it, by a pointer move or by content
    /// that scrolled under the pointer.
    fn drag_over(&self, hit: Hit<K>) {
        let at = hit.at.clone();
        *self.over.borrow_mut() = Some(hit);
        if !self.pressing.get() || self.anchor.borrow().is_none() {
            return;
        }
        if self.focus.borrow().as_ref() != Some(&at) {
            *self.focus.borrow_mut() = Some(at);
            *self.stepped.borrow_mut() = None;
            self.update();
        }
    }

    /// Handles a key event the scope's focus received: copy, select all,
    /// clear, and the moves of the selection's moving end. Answers whether
    /// it took the event.
    fn key(&self, event: &KeyEvent) -> Handled {
        if !event.pressed {
            return Handled::No;
        }
        let modifiers = event.modifiers;
        if modifiers.command() && is_letter(&event.key, "c") {
            self.copy();
            return Handled::Yes;
        }
        if modifiers.command() && is_letter(&event.key, "a") {
            self.select_all();
            return Handled::Yes;
        }
        if event.key == Key::Escape {
            let order = Rc::clone(&self.order.borrow());
            let selected = self.ends_in(&order).is_some();
            *self.anchor.borrow_mut() = None;
            *self.focus.borrow_mut() = None;
            *self.stepped.borrow_mut() = None;
            self.update();
            return if selected { Handled::Yes } else { Handled::No };
        }
        if !modifiers.shift || self.anchor.borrow().is_none() {
            return Handled::No;
        }
        // The modifiers mean what they mean in the text inputs: Ctrl or Alt
        // moves by word, and Ctrl or Cmd with Home or End goes to the start
        // or the end of the whole text, here the whole scope.
        let word = modifiers.control || modifiers.alt;
        let whole = modifiers.control || modifiers.meta;
        let step = match &event.key {
            Key::ArrowLeft if word => Step::WordLeft,
            Key::ArrowRight if word => Step::WordRight,
            Key::ArrowLeft => Step::Left,
            Key::ArrowRight => Step::Right,
            Key::ArrowUp => Step::Up,
            Key::ArrowDown => Step::Down,
            Key::Home if whole => return self.move_focus_to_edge(false),
            Key::End if whole => return self.move_focus_to_edge(true),
            Key::Home => Step::LineStart,
            Key::End => Step::LineEnd,
            _ => return Handled::No,
        };
        self.move_focus(step);
        Handled::Yes
    }

    /// Selects every block of the scope.
    fn select_all(&self) {
        let order = Rc::clone(&self.order.borrow());
        let (Some(first), Some(last)) = (order.keys.first(), order.keys.last()) else {
            return;
        };
        *self.anchor.borrow_mut() = Some(Endpoint {
            key: first.clone(),
            offset: 0,
        });
        *self.focus.borrow_mut() = Some(Endpoint {
            key: last.clone(),
            offset: self.text_len(last),
        });
        *self.stepped.borrow_mut() = None;
        self.update();
    }

    /// Returns the length of the text of the block of `key`, from its layout
    /// when it is mounted and from `text_of` when it is not.
    fn text_len(&self, key: &K) -> usize {
        self.nav_of(key)
            .and_then(|nav| nav.len())
            .unwrap_or_else(|| (self.text_of)(key).len())
    }

    /// Returns the navigator of a mounted block of `key`.
    fn nav_of(&self, key: &K) -> Option<Rc<dyn Nav>> {
        self.blocks
            .borrow()
            .get(key)
            .and_then(|list| list.first().map(|block| Rc::clone(&block.nav)))
    }

    /// Moves the selection's moving end to the start or the end of the whole
    /// scope.
    fn move_focus_to_edge(&self, end: bool) -> Handled {
        let order = Rc::clone(&self.order.borrow());
        let key = if end {
            order.keys.last()
        } else {
            order.keys.first()
        };
        let Some(key) = key else {
            return Handled::No;
        };
        let at = Endpoint {
            key: key.clone(),
            offset: if end { self.text_len(key) } else { 0 },
        };
        *self.stepped.borrow_mut() = None;
        self.set_focus(at, false);
        Handled::Yes
    }

    /// Moves the selection's moving end by `step`, inside its block through
    /// the block's own layout when it is mounted and through its text when it
    /// is not, and into the next block in that direction at an edge.
    fn move_focus(&self, step: Step) {
        let Some(focus) = self.focus.borrow().clone() else {
            return;
        };
        let order = Rc::clone(&self.order.borrow());
        let nav = self.nav_of(&focus.key);
        let carried = self
            .stepped
            .borrow()
            .as_ref()
            .filter(|(key, _)| *key == focus.key)
            .map(|(_, selection)| *selection);
        let text_step = || step_text(&(self.text_of)(&focus.key), focus.offset, step);
        // The move inside the block, as the layout's selection when the block
        // has one and as an offset into its text when it has none.
        let inside = match nav
            .as_ref()
            .map(|nav| nav.step(carried, focus.offset, step))
        {
            Some(Stepped::Moved(selection)) => Some(Ok(selection)),
            Some(Stepped::Edge) => None,
            Some(Stepped::NoLayout) | None => text_step().map(Err),
        };
        let (at, upstream) = match inside {
            Some(Ok(selection)) => {
                let cursor = selection.focus();
                *self.stepped.borrow_mut() = Some((focus.key.clone(), selection));
                (
                    Endpoint {
                        key: focus.key.clone(),
                        offset: cursor.index(),
                    },
                    cursor.affinity() == Affinity::Upstream,
                )
            }
            Some(Err(offset)) => {
                *self.stepped.borrow_mut() = None;
                (
                    Endpoint {
                        key: focus.key.clone(),
                        offset,
                    },
                    false,
                )
            }
            None if step.crosses() => {
                let rtl = nav.as_ref().and_then(|nav| nav.rtl()).unwrap_or(false);
                let backward = step.backward(rtl);
                let Some(position) = order.position(&focus.key) else {
                    return;
                };
                let neighbour = if backward {
                    position.checked_sub(1).and_then(|at| order.keys.get(at))
                } else {
                    order.keys.get(position + 1)
                };
                let Some(neighbour) = neighbour else {
                    return;
                };
                *self.stepped.borrow_mut() = None;
                let offset = if backward {
                    self.text_len(neighbour)
                } else {
                    0
                };
                (
                    Endpoint {
                        key: neighbour.clone(),
                        offset,
                    },
                    false,
                )
            }
            None => return,
        };
        self.set_focus(at, upstream);
    }

    /// Puts the selection's moving end at `at`, and has the block that holds
    /// it show it, at the end of the line before it when `upstream` is set.
    fn set_focus(&self, at: Endpoint<K>, upstream: bool) {
        if let Some(list) = self.blocks.borrow().get(&at.key) {
            for block in list {
                block.reveal.set(Some((at.offset, upstream)));
            }
        }
        *self.focus.borrow_mut() = Some(at);
        self.update();
    }

    /// Ends the drag, whichever node took its press.
    fn release(&self) {
        self.pressing.set(false);
        *self.over.borrow_mut() = None;
    }

    /// Returns the selected text of every block in order, with a newline
    /// between blocks, or `None` when nothing is selected.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let order = Rc::clone(&self.order.borrow());
        let (start, end) = self.ends_in(&order)?;
        Some(join(&order, &start, &end, &*self.text_of))
    }

    /// Writes the selected text to the clipboard.
    fn copy(&self) {
        if let Some(text) = self.selected_text() {
            smudge_core::clipboard::with(|clipboard| clipboard.set(text));
        }
    }
}

/// Paints the rectangles of `range` in `layout`, placed at the node's
/// origin, in the selection color the text inputs use.
pub(crate) fn paint_selected<B: Brush>(
    layout: &parley::Layout<B>,
    range: Range<usize>,
    cx: &PaintCx<'_>,
    out: &mut Vec<PaintOp>,
) {
    let color = Color {
        a: SELECTION_ALPHA,
        ..cx.theme.color(ColorToken::ACCENT)
    };
    let origin = cx.rect.origin();
    let selection = ParagraphSelection::new(
        Cursor::from_byte_index(layout, range.start, Affinity::Downstream),
        Cursor::from_byte_index(layout, range.end, Affinity::Downstream),
    );
    selection.geometry_with(layout, |area, _| {
        let rect = Rect::new(
            origin.x + area.x0 as f32,
            origin.y + area.y0 as f32,
            origin.x + area.x1 as f32,
            origin.y + area.y1 as f32,
        );
        // A truncated text keeps its hidden lines in the layout, so the
        // selection is cut to the node, where the text shows.
        let rect = rect.intersect(cx.rect);
        if rect.width() > 0.0 && rect.height() > 0.0 {
            out.push(PaintOp::FillRect {
                rect,
                radius: Corners::ZERO,
                color,
            });
        }
    });
}

/// Returns what lies under `at` in `layout`, for the block of `key`.
fn hit<K, B: Brush>(key: K, layout: &parley::Layout<B>, at: Point) -> Hit<K> {
    let offset = Cursor::from_point(layout, at.x, at.y).index();
    let word = ParagraphSelection::word_from_point(layout, at.x, at.y).text_range();
    Hit {
        at: Endpoint { key, offset },
        word,
    }
}

/// Creates a scope whose labels and rich text paragraphs share one
/// selection across blocks.
///
/// `keys` returns the key of every block in order, including blocks a
/// virtual list has not mounted, and `text_of` returns any block's plain
/// text, which copy reads. A block takes part with
/// [`SelectableExt::selectable`] and the key it has in `keys`. `content`
/// builds what the scope shows, inside the scope, so its blocks find it.
///
/// A press on a block starts a selection there, and a drag extends it,
/// across blocks and through a scroll view that scrolls while the drag is
/// held past its edge. A double click selects a word. The scope takes the
/// keyboard focus, and Ctrl+C, or Cmd+C on macOS, copies the selected text
/// of every block in order, with a newline between blocks. A press that
/// finds no block clears the selection.
///
/// With the focus, Shift and the arrow keys extend the selection by a
/// character or a line, and with Ctrl or Alt by a word, crossing into the
/// next block at a block's edge. Shift and Home or End extend it to the start
/// or the end of the line, and with Ctrl or Cmd to the start or the end of
/// the whole scope. The scroll views around the block that holds the moving
/// end follow it. Ctrl+A, or Cmd+A, selects every block, and Escape clears
/// the selection. With nothing selected, the arrow keys pass to the nodes
/// around the scope. When `keys` changes, an end whose
/// block left moves to the nearest block of the selection that remains, so
/// trimming the oldest lines of a log keeps the rest of the selection, and a
/// selection none of whose blocks remains is cleared. A block that mounts
/// again shows its part of the selection.
///
/// `keys` runs inside an effect, and the scope reads the order again only
/// when it returns another `Rc`, so it must read the signals its list
/// changes with and hand back the same `Rc` until the list changes.
///
/// `text_of` must return the text the block shows, because the selection's
/// offsets are byte offsets into it: a label's text, or the text of a rich
/// text paragraph's spans joined with nothing between them.
///
/// The selection starts only from a press the scope takes. A row inside the
/// scope that takes presses itself, with a press or hover handler or a hover
/// style, keeps them, and a press on a link takes the keyboard focus from
/// the scope, so copy needs a press on the scope's text again.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::widget::panel;
/// use smudge_text::{SelectableExt, label, selection_scope};
///
/// let lines: Rc<[String]> = Rc::from([String::from("first"), String::from("second")]);
/// let texts = Rc::clone(&lines);
/// let scope = selection_scope(
///     || Rc::from([0usize, 1]),
///     move |at: &usize| texts[*at].clone(),
///     move || {
///         panel()
///             .column()
///             .child(label(lines[0].clone()).selectable(0usize))
///             .child(label(lines[1].clone()).selectable(1usize))
///     },
/// );
/// ```
pub fn selection_scope<K: Clone + Eq + Hash + 'static, E: IntoElement>(
    keys: impl Fn() -> Rc<[K]> + 'static,
    text_of: impl Fn(&K) -> String + 'static,
    content: impl Fn() -> E + 'static,
) -> Element<Panel> {
    let keys = smudge_core::hot::wrap(keys);
    let first = Rc::new(Order::new(keys()));
    let scope = Rc::new(Scope {
        keys: Box::new(keys),
        order: RefCell::new(first),
        text_of: Box::new(smudge_core::hot::wrap1_ref(text_of)),
        anchor: RefCell::new(None),
        focus: RefCell::new(None),
        over: RefCell::new(None),
        blocks: RefCell::new(HashMap::new()),
        next_block: Cell::new(0),
        stepped: RefCell::new(None),
        pressing: Cell::new(false),
        last_press: Cell::new(None),
    });
    {
        let scope = Rc::clone(&scope);
        RenderEffect::new(move || scope.refresh());
    }
    let content = smudge_core::hot::wrap(content);
    let body = Rc::clone(&scope);
    let down = Rc::clone(&scope);
    let up = Rc::clone(&scope);
    let ended = Rc::clone(&scope);
    let keys = Rc::clone(&scope);
    panel()
        .column()
        .on_pointer_down(move |at| down.press(at))
        .on_pointer_up(move |_| up.release())
        // The scope's rectangle holds every press made inside it, so it hears
        // the end of a drag whose press a link or a row inside it took.
        .on_drag(move |step| {
            if matches!(step, Drag::Ended) {
                ended.release();
            }
            Handled::No
        })
        .on_key(move |event: &KeyEvent| keys.key(event))
        // The content mounts under an owner of its own, which carries the
        // scope, so blocks built inside it later, such as a virtual list's
        // rows, find the scope and siblings outside it do not.
        .child(
            show(
                || true,
                move || {
                    provide_context(Rc::clone(&body));
                    content()
                },
            )
            // The slot lays the content out as the scope's own column would,
            // stretched across and growing into the scope's height.
            .column()
            .grow(1.0),
        )
}

/// Returns whether `key` is the letter `letter`, in either case.
fn is_letter(key: &Key, letter: &str) -> bool {
    matches!(key, Key::Character(text) if text.eq_ignore_ascii_case(letter))
}

/// Makes a label or a rich text paragraph a block of the selection scope
/// around it.
///
/// Import the trait to use it.
pub trait SelectableExt: Sized {
    /// Makes the element the block of `key` in the [`selection_scope`]
    /// around it, so a drag over it selects its text and it paints its part
    /// of the selection.
    ///
    /// `key` must be the key the scope's `keys` lists for this block, of the
    /// same type. Outside a scope with keys of that type it does nothing,
    /// and the element behaves and costs as it would without it.
    fn selectable<K: Clone + Eq + Hash + 'static>(self, key: K) -> Self;
}

/// The key of a selectable block's layout sharing binding.
struct ShareKey;

/// The key of a selectable block's selected range binding.
struct SelectedKey;

/// Registers the block of `key` with `scope` for the current owner's
/// lifetime, and returns the signal of its selected range.
fn join_scope<K: Clone + Eq + Hash + 'static>(
    scope: &Rc<Scope<K>>,
    key: &K,
    nav: Rc<dyn Nav>,
) -> (RangeSignal, RevealSignal) {
    let (id, range, reveal) = scope.register(key, nav);
    let scope = Rc::clone(scope);
    let key = key.clone();
    Owner::on_cleanup(move || scope.unregister(&key, id));
    (range, reveal)
}

/// The keyboard's view of a selectable label, through the layout it shares.
struct LabelNav(Rc<RefCell<Option<TextLayout>>>);

impl Nav for LabelNav {
    fn step(&self, from: Option<ParagraphSelection>, offset: usize, step: Step) -> Stepped {
        match self.0.borrow().as_ref() {
            Some(layout) => step_in(&layout.layout, from, offset, step),
            None => Stepped::NoLayout,
        }
    }

    fn caret(&self, offset: usize, upstream: bool) -> Option<Rect> {
        Some(caret_in(
            &self.0.borrow().as_ref()?.layout,
            offset,
            upstream,
        ))
    }

    fn rtl(&self) -> Option<bool> {
        Some(self.0.borrow().as_ref()?.layout.is_rtl())
    }

    fn len(&self) -> Option<usize> {
        Some(len_in(&self.0.borrow().as_ref()?.layout))
    }
}

/// The keyboard's view of a selectable paragraph, through the layout it
/// hands over.
struct RichNav(LayoutSink);

impl RichNav {
    /// Runs `f` with the paragraph's layout, when it has one.
    fn with<R>(&self, f: impl FnOnce(&parley::Layout<u32>) -> R) -> Option<R> {
        let shared = self.0.borrow();
        let layout = shared.as_ref()?.borrow();
        Some(f(&layout.as_ref()?.layout))
    }
}

impl Nav for RichNav {
    fn step(&self, from: Option<ParagraphSelection>, offset: usize, step: Step) -> Stepped {
        self.with(|layout| step_in(layout, from, offset, step))
            .unwrap_or(Stepped::NoLayout)
    }

    fn caret(&self, offset: usize, upstream: bool) -> Option<Rect> {
        self.with(|layout| caret_in(layout, offset, upstream))
    }

    fn rtl(&self) -> Option<bool> {
        self.with(|layout| layout.is_rtl())
    }

    fn len(&self) -> Option<usize> {
        self.with(len_in)
    }
}

impl SelectableExt for Element<Label> {
    fn selectable<K: Clone + Eq + Hash + 'static>(self, key: K) -> Self {
        let Some(scope) = use_context::<Rc<Scope<K>>>() else {
            return self;
        };
        let shared: Rc<RefCell<Option<TextLayout>>> = Rc::new(RefCell::new(None));
        let nav: Rc<dyn Nav> = Rc::new(LabelNav(Rc::clone(&shared)));
        let (range, reveal) = join_scope(&scope, &key, Rc::clone(&nav));
        let layout = Rc::clone(&shared);
        self.bind(ShareKey, Value::Static(shared), crate::label::share_layout)
            .bind(SelectedKey, move || range.get(), crate::label::set_selected)
            .reveal(move || {
                reveal
                    .get()
                    .and_then(|(offset, upstream)| nav.caret(offset, upstream))
            })
            .on_drag_over(move |at| {
                if let Some(layout) = layout.borrow().as_ref() {
                    scope.drag_over(hit(key.clone(), &layout.layout, at));
                }
            })
    }
}

impl SelectableExt for Element<RichText> {
    fn selectable<K: Clone + Eq + Hash + 'static>(self, key: K) -> Self {
        let Some(scope) = use_context::<Rc<Scope<K>>>() else {
            return self;
        };
        let sink: LayoutSink = Rc::new(RefCell::new(None));
        let nav: Rc<dyn Nav> = Rc::new(RichNav(Rc::clone(&sink)));
        let (range, reveal) = join_scope(&scope, &key, Rc::clone(&nav));
        let layout = Rc::clone(&sink);
        self.bind(
            ShareKey,
            Value::Static(sink),
            crate::rich_text::share_layout,
        )
        .bind(
            SelectedKey,
            move || range.get(),
            crate::rich_text::set_selected,
        )
        .reveal(move || {
            reveal
                .get()
                .and_then(|(offset, upstream)| nav.caret(offset, upstream))
        })
        .on_drag_over(move |at| {
            let shared = layout.borrow();
            let Some(shared) = shared.as_ref() else {
                return;
            };
            if let Some(layout) = shared.borrow().as_ref() {
                scope.drag_over(hit(key.clone(), &layout.layout, at));
            }
        })
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
