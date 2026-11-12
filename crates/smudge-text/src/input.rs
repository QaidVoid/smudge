//! The editing engine behind a text input and a text area.
//!
//! [`Editor`] keeps the bound signal in step with every edit and turns its
//! layouts into the glyph runs and geometry a widget paints. The text is
//! one string, cut into paragraphs at its line breaks, and each paragraph
//! is laid out on its own: an edit re-lays out the paragraphs it touched
//! and leaves the rest of a large document shaped as it was, so typing
//! stays cheap in a document far larger than the screen. All of parley
//! stays behind this wrapper, so a widget and a test drive the same edits
//! without touching parley types for anything but geometry.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Range;

use parley::Cluster;
use parley::editing::Cursor;
use parley::editing::Selection as ParagraphSelection;
use parley::layout::Affinity;

use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::paint::GlyphRun;
use smudge_core::reactive::{RwSignal, prelude::*};

use crate::glyphs::glyph_runs;
use crate::layout::TextLayout;
use crate::style::TextStyle;
use crate::system::TextSystem;

/// How many bytes the undo and redo steps of one editor may hold together.
/// Older undo steps are forgotten past this, but the newest one is always
/// kept, so a text larger than the bound can still undo once.
///
/// Every step keeps the whole text it restores, so the bound is in bytes: a
/// short text keeps thousands of steps and a text of several megabytes a
/// few.
const HISTORY_BYTES: usize = 8 << 20;

/// How wide the highlight of a selected line break is, in font sizes.
const NEWLINE_WIDTH: f32 = 0.3;

/// How far below the baseline the composing text is underlined, in logical
/// pixels.
const UNDERLINE_OFFSET: f32 = 1.0;

/// How thick the underline under the composing text is, in logical pixels.
const UNDERLINE_THICKNESS: f32 = 1.0;

/// How far the input method area inflates around the text it bounds, in
/// font sizes, so a candidate window does not jump while the user types.
/// This assumes an average glyph advance of a bit over half a font size
/// and pads three advances.
const IME_INFLATE: f32 = 1.8;

/// One text buffer bound to a signal, over any number of paragraphs.
///
/// The editor holds the text and the selection; the signal mirrors the text
/// without any composing preedit. Every mutating method writes the signal,
/// so [`Editor::text`] and the signal always agree. A write to the signal
/// from outside is applied with [`Editor::set_text`], which replaces the
/// text and moves the caret to the end.
///
/// The editor lays out lazily and per paragraph: an edit drops the layouts
/// of the paragraphs it touched, and the next call that needs geometry
/// rebuilds them, alone. A document far larger than the screen costs an
/// edit only the work its own lines take.
///
/// # Panics
///
/// Every method that edits the text, moves the caret or reads geometry can
/// build layouts, so it runs the thread's [`TextSystem`] and panics under
/// the conditions of [`TextSystem::with`], in particular when called from
/// inside another `with` call on the same thread. No input text or edit
/// sequence panics.
pub struct Editor {
    /// The signal the text is bound to.
    value: RwSignal<String>,
    /// The text of the editor, including any composing preedit.
    buffer: RefCell<String>,
    /// The paragraphs the buffer is cut into, with their layout caches.
    paragraphs: RefCell<ParagraphTable>,
    /// Where the selection sits.
    selection: RefCell<Selection>,
    /// The preedit an input method is composing, if one is.
    compose: RefCell<Option<Compose>>,
    /// The steps undo and redo walk through.
    history: RefCell<History>,
    /// The font style the paragraphs lay out with.
    style: RefCell<TextStyle>,
    /// The width paragraphs wrap at, in logical pixels, or `None` to lay
    /// each out as one unbroken line.
    width: Cell<Option<f32>>,
    /// How many times the editor has built or wrapped a paragraph layout,
    /// which paragraph stamps are counted against.
    builds: Cell<u64>,
}

/// The text and selection of an editor at one moment.
struct Snapshot {
    /// The text, without any composing preedit.
    text: String,
    /// The selection, as the engine keeps it.
    selection: Selection,
}

/// What kind of change an edit made, which decides the edits it groups with.
#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    /// Text was typed or inserted.
    Insert,
    /// Text was deleted.
    Delete,
}

/// How one edit groups with the edits around it in the undo history.
#[derive(Clone, Copy, PartialEq)]
enum Grouping {
    /// Extends the open step of the same kind, and stays open.
    Joins,
    /// Starts a step that the next edit of the same kind can join, as a
    /// typed space starts the step of the word after it.
    Starts,
    /// Is a step of its own, which nothing joins.
    Alone,
}

/// The undo and redo steps of one editor.
#[derive(Default)]
struct History {
    /// The states before each step, oldest first.
    undo: VecDeque<Snapshot>,
    /// The states undo left, most recently undone last.
    redo: Vec<Snapshot>,
    /// The bytes the steps of both lists hold, which [`HISTORY_BYTES`]
    /// bounds.
    bytes: usize,
    /// The kind of the last edit and the selection it left, which the next
    /// edit of the same kind joins when it starts where that one ended.
    /// A caret move closes it.
    open: Option<(EditKind, Range<usize>)>,
}

impl History {
    /// The bytes one step holds.
    fn cost(snapshot: &Snapshot) -> usize {
        snapshot.text.len() + size_of::<Snapshot>()
    }

    /// Adds a step to undo, forgetting the oldest steps past the byte bound.
    fn push_undo(&mut self, snapshot: Snapshot) {
        self.bytes += Self::cost(&snapshot);
        self.undo.push_back(snapshot);
        while self.bytes > HISTORY_BYTES && self.undo.len() > 1 {
            if let Some(oldest) = self.undo.pop_front() {
                self.bytes -= Self::cost(&oldest);
            }
        }
    }

    /// Takes the newest undo step.
    fn pop_undo(&mut self) -> Option<Snapshot> {
        let snapshot = self.undo.pop_back()?;
        self.bytes -= Self::cost(&snapshot);
        Some(snapshot)
    }

    /// Adds a step to redo.
    fn push_redo(&mut self, snapshot: Snapshot) {
        self.bytes += Self::cost(&snapshot);
        self.redo.push(snapshot);
    }

    /// Takes the newest redo step.
    fn pop_redo(&mut self) -> Option<Snapshot> {
        let snapshot = self.redo.pop()?;
        self.bytes -= Self::cost(&snapshot);
        Some(snapshot)
    }

    /// Forgets every redo step.
    fn clear_redo(&mut self) {
        for snapshot in self.redo.drain(..) {
            self.bytes -= Self::cost(&snapshot);
        }
    }
}

/// One end of a selection: a paragraph and a byte offset into its text.
///
/// The affinity says which side of the cluster at the offset the caret
/// sits on. Two anchors at the same visual boundary can carry different
/// offsets when a line break lies between them: the end of one paragraph
/// and the start of the next name the break's two sides.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    /// The paragraph the anchor sits in.
    paragraph: usize,
    /// The byte offset into the paragraph's text.
    index: usize,
    /// Which side of the cluster the caret sits on.
    affinity: Affinity,
}

/// The granularity a shift click extends a selection with.
///
/// A plain click, a caret move or an edit resets this to clusters. A double
/// click sets it to words, so the shift clicks that follow extend a word at
/// a time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Base {
    /// Extends by cluster.
    #[default]
    Cluster,
    /// Extends to word boundaries, with the two ends the double click made.
    Word([Anchor; 2]),
}

/// Where a selection sits, in paragraph-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Selection {
    /// The end the selection extends from.
    anchor: Anchor,
    /// The end the selection extends to.
    focus: Anchor,
    /// The granularity a shift click extends with.
    base: Base,
    /// The horizontal position the focus keeps across vertical moves, in
    /// logical pixels.
    h_pos: Option<f32>,
}

/// The preedit an input method is composing into the buffer.
struct Compose {
    /// The byte range of the buffer the preedit occupies.
    range: Range<usize>,
    /// The byte range of the input method's cursor inside the preedit,
    /// relative to its start, or `None` when the input method hides it.
    cursor: Option<(usize, usize)>,
}

/// One paragraph of an editor's buffer, with its layout cache.
struct Paragraph {
    /// Byte offset of the paragraph's first byte in the buffer.
    start: usize,
    /// Byte length of the paragraph, without the line break that ends it.
    len: usize,
    /// The laid out paragraph, or `None` when it needs building.
    layout: Option<TextLayout>,
    /// The editor's build count when the layout was built or wrapped
    /// again. A paragraph whose layout is missing was not built yet.
    stamp: u64,
}

/// The paragraphs an editor's buffer is cut into, in text order.
#[derive(Default)]
struct ParagraphTable {
    /// One entry per line break plus one for the text after the last one,
    /// so the table is never empty and covers the buffer exactly. Each
    /// paragraph but the last is followed by its line break in the buffer.
    paragraphs: Vec<Paragraph>,
}

impl ParagraphTable {
    /// The global byte offset of a paragraph-local anchor.
    fn global(&self, anchor: Anchor) -> usize {
        self.paragraphs[anchor.paragraph].start + anchor.index
    }

    /// The paragraph and local index of a global byte offset.
    ///
    /// The line break that ends a paragraph belongs to that paragraph's
    /// span, so an offset on the break itself names the paragraph before
    /// it, at the end of its text.
    fn locate(&self, byte: usize) -> (usize, usize) {
        let paragraph = self.index_at(byte);
        (paragraph, byte - self.paragraphs[paragraph].start)
    }

    /// The index of the paragraph whose span holds `byte`.
    fn index_at(&self, byte: usize) -> usize {
        self.paragraphs
            .partition_point(|paragraph| paragraph.start <= byte)
            - 1
    }

    /// Re-cuts the whole buffer into paragraphs, keeping no layouts.
    fn rebuild(&mut self, buffer: &str) {
        let mut paragraphs = Vec::new();
        let mut start = 0;
        for (offset, byte) in buffer.bytes().enumerate() {
            if byte == b'\n' {
                paragraphs.push(Paragraph {
                    start,
                    len: offset - start,
                    layout: None,
                    stamp: 0,
                });
                start = offset + 1;
            }
        }
        paragraphs.push(Paragraph {
            start,
            len: buffer.len() - start,
            layout: None,
            stamp: 0,
        });
        self.paragraphs = paragraphs;
    }

    /// Re-cuts the buffer into paragraphs after an edit replaced `range`
    /// with `replacement`, keeping every layout whose paragraph text the
    /// edit did not touch.
    ///
    /// The buffer already holds the edited text: the table follows it. The
    /// paragraphs before the edit keep their layouts, the paragraphs after
    /// it keep theirs with shifted offsets, and the paragraphs the
    /// replacement straddled are laid out again on the next need. A range
    /// that reaches the next paragraph's start consumed the line break that
    /// separated the two, so that paragraph's text belongs to the
    /// replacement's last piece and cuts together with it.
    fn splice(&mut self, range: Range<usize>, replacement: &str) {
        let first = self.index_at(range.start);
        let mut last = if range.is_empty() {
            first
        } else {
            self.index_at(range.end - 1)
        };
        if !range.is_empty()
            && last + 1 < self.paragraphs.len()
            && range.end == self.paragraphs[last + 1].start
        {
            last += 1;
        }
        let first_start = self.paragraphs[first].start;
        let last_end = self.paragraphs[last].start + self.paragraphs[last].len;
        let head = range.start - first_start;
        let tail = last_end.saturating_sub(range.end);
        self.paragraphs.drain(first..=last);

        let delta = replacement.len() as isize - (range.end - range.start) as isize;
        for paragraph in self.paragraphs[first..].iter_mut() {
            paragraph.start = (paragraph.start as isize + delta) as usize;
        }

        let pieces: Vec<&str> = replacement.split('\n').collect();
        let last_piece = pieces.len() - 1;
        let mut entries = Vec::with_capacity(pieces.len());
        let mut start = first_start;
        for (index, piece) in pieces.iter().enumerate() {
            let len = piece.len()
                + if index == 0 { head } else { 0 }
                + if index == last_piece { tail } else { 0 };
            entries.push(Paragraph {
                start,
                len,
                layout: None,
                stamp: 0,
            });
            start += len + 1;
        }
        self.paragraphs.splice(first..first, entries);
    }
}

impl Editor {
    /// Creates an editor bound to `value`, starting empty.
    ///
    /// The text the signal holds is not read: an editor starts empty and
    /// fills when its widget applies the signal's current value. Binding
    /// `value` through a widget does that on mount.
    pub fn new(value: RwSignal<String>) -> Editor {
        let mut paragraphs = ParagraphTable::default();
        paragraphs.rebuild("");
        Editor {
            value,
            buffer: RefCell::new(String::new()),
            paragraphs: RefCell::new(paragraphs),
            selection: RefCell::new(Selection {
                anchor: START,
                focus: START,
                base: Base::Cluster,
                h_pos: None,
            }),
            compose: RefCell::new(None),
            history: RefCell::new(History::default()),
            style: RefCell::new(TextStyle::default()),
            width: Cell::new(None),
            builds: Cell::new(0),
        }
    }

    /// Returns the signal the text is bound to.
    pub fn value(&self) -> RwSignal<String> {
        self.value.clone()
    }

    /// Applies `style` to the text the editor lays out.
    ///
    /// The next layout rebuild picks the style up. Changing the font size
    /// changes what the editor measures, so a widget applies this before it
    /// measures.
    pub fn set_style(&self, style: &TextStyle) {
        *self.style.borrow_mut() = style.clone();
        self.invalidate_layouts();
    }

    /// Sets the width the editor wraps its lines to, in logical pixels, or
    /// `None` to lay each paragraph out as one unbroken line.
    ///
    /// An editor starts unwrapped. The paragraphs already laid out break
    /// their lines again at the new width without shaping their text again,
    /// and the caret and selection geometry follow. The same width again
    /// does nothing.
    pub fn set_wrap_width(&self, width: Option<f32>) {
        if self.width.get() == width {
            return;
        }
        self.width.set(width);
        // The width only decides where lines break, so the paragraphs keep
        // their shaping and wrap again.
        for paragraph in self.paragraphs.borrow_mut().paragraphs.iter_mut() {
            if let Some(layout) = paragraph.layout.as_mut() {
                layout.rebreak(width);
                self.builds.set(self.builds.get() + 1);
                paragraph.stamp = self.builds.get();
            }
        }
    }

    /// Returns the text of the editor, without any composing preedit.
    pub fn text(&self) -> String {
        let buffer = self.buffer.borrow();
        let compose = self.compose.borrow();
        match compose.as_ref() {
            Some(compose) => {
                let mut text = String::with_capacity(buffer.len() - compose.range.len());
                text.push_str(&buffer[..compose.range.start]);
                text.push_str(&buffer[compose.range.end..]);
                text
            }
            None => buffer.clone(),
        }
    }

    /// Returns whether the editor holds no text.
    pub fn is_empty(&self) -> bool {
        let preedit = self
            .compose
            .borrow()
            .as_ref()
            .map_or(0, |compose| compose.range.len());
        self.buffer.borrow().len() == preedit
    }

    /// Returns whether the editor holds exactly `text`.
    pub fn has_text(&self, text: &str) -> bool {
        let buffer = self.buffer.borrow();
        match self.compose.borrow().as_ref() {
            Some(compose) => {
                let (head, tail) = (&buffer[..compose.range.start], &buffer[compose.range.end..]);
                text.len() == head.len() + tail.len()
                    && text.starts_with(head)
                    && text.ends_with(tail)
            }
            None => *buffer == text,
        }
    }

    /// Returns the text of the editor, including any composing preedit.
    pub fn raw_text(&self) -> String {
        self.buffer.borrow().clone()
    }

    /// Returns the byte range of the selection, into [`Editor::raw_text`].
    ///
    /// The range is empty and both ends equal when the caret is collapsed.
    pub fn selection(&self) -> Range<usize> {
        let paragraphs = self.paragraphs.borrow();
        let selection = self.selection.borrow();
        let anchor = paragraphs.global(selection.anchor);
        let focus = paragraphs.global(selection.focus);
        anchor.min(focus)..anchor.max(focus)
    }

    /// Returns whether an input method is composing text in the editor.
    pub fn is_composing(&self) -> bool {
        self.compose.borrow().is_some()
    }

    /// Returns the area the input method should anchor to, in the editor's
    /// own coordinates.
    ///
    /// This bounds the preedit while one is being composed, and the selected
    /// part of the focused line otherwise, inflated a little so a candidate
    /// window does not jump while the user types.
    pub fn ime_area(&self) -> Rect {
        let preedit = self
            .compose
            .borrow()
            .as_ref()
            .map(|compose| compose.range.clone());
        self.session(|session| {
            session.refresh_all();
            let mut area: Option<Rect> = None;
            let mut add = |rect: Rect| area = Some(area.map_or(rect, |area| area.union(rect)));
            match preedit {
                Some(range) => {
                    for (rect, _) in session.range_rects(range) {
                        add(rect);
                    }
                    if area.is_none() {
                        // A preedit of only line breaks holds no glyphs, so
                        // the caret anchors the area instead.
                        let focus = session.selection.focus;
                        let (caret, y) = session.caret_box(focus, 0.0);
                        area = Some(Rect::new(caret.x0, caret.y0 + y, caret.x1, caret.y1 + y));
                    }
                }
                None => {
                    // Bound the selected parts of the line the focus sits
                    // on only, starting from its caret so a collapsed
                    // selection still gives the area some width.
                    let focus = session.selection.focus;
                    let (caret, y) = session.caret_box(focus, 0.0);
                    let caret = Rect::new(caret.x0, caret.y0 + y, caret.x1, caret.y1 + y);
                    for (rect, _) in session.range_rects(session.selection_range()) {
                        if rect.y0 == caret.y0 {
                            add(rect);
                        }
                    }
                    add(caret);
                }
            }
            let area = area.expect("the caret always gives the area a rectangle");
            // The caret and selection boxes span the font's content box,
            // which a small line height can push past the text's own top
            // and bottom, and an anchor outside the text means nothing to
            // an input method.
            let height: f32 = session
                .table
                .paragraphs
                .iter()
                .map(|paragraph| {
                    paragraph
                        .layout
                        .as_ref()
                        .expect("refresh built every layout")
                        .size()
                        .height
                })
                .sum();
            let style = self.style.borrow();
            let inflate = IME_INFLATE * style.size;
            let width = session.width.unwrap_or(f32::INFINITY);
            Rect::new(
                (area.x0 - inflate).max(0.0),
                area.y0.max(0.0),
                (area.x1 + inflate).min(width),
                area.y1.min(height),
            )
        })
    }

    /// Returns one rectangle per piece of the composing preedit, in the
    /// editor's own coordinates, for the underline drawn beneath it.
    ///
    /// The rectangles sit just below the baseline of each piece, in visual
    /// order, and the list is empty while no input method is composing.
    pub fn compose_geometry(&self) -> Vec<Rect> {
        let Some(range) = self
            .compose
            .borrow()
            .as_ref()
            .map(|compose| compose.range.clone())
        else {
            return Vec::new();
        };
        self.session(|session| {
            session.refresh_all();
            let mut rects = Vec::new();
            let mut y = 0.0;
            for index in 0..session.table.paragraphs.len() {
                let paragraph = &session.table.paragraphs[index];
                let start = paragraph.start;
                let len = paragraph.len;
                let layout = paragraph
                    .layout
                    .as_ref()
                    .expect("refresh built every layout");
                if range.start < start + len && range.end > start {
                    let mut index = range.start.max(start) - start;
                    let end = range.end.min(start + len) - start;
                    while index < end {
                        let Some(cluster) = Cluster::from_byte_index(&layout.layout, index) else {
                            break;
                        };
                        // A cluster can hold more than the byte the loop
                        // advanced to, so the next index is always at least
                        // one past the current one.
                        index = cluster.text_range().end.max(index + 1);
                        let Some(offset) = cluster.visual_offset() else {
                            continue;
                        };
                        let baseline = cluster.line().metrics().baseline;
                        let top = baseline + UNDERLINE_OFFSET;
                        rects.push(Rect::new(
                            offset,
                            top + y,
                            offset + cluster.advance(),
                            top + UNDERLINE_THICKNESS + y,
                        ));
                    }
                }
                y += layout.size().height;
            }
            rects
        })
    }

    /// Replaces the whole text and moves the caret to the end.
    ///
    /// This is how an outside write to the bound signal reaches the editor:
    /// the user typed somewhere else, or the application replaced the value.
    /// The selection collapses at the end, because a selection into text the
    /// editor no longer has means nothing. The undo history is cleared, so
    /// undo never brings back text the application replaced, and any
    /// composition stops, because the text it was composing into is gone.
    ///
    /// Every carriage return and line feed pair, and every lone carriage
    /// return, becomes one line feed, and the bound signal is written with
    /// the text the editor kept.
    pub fn set_text(&self, text: &str) {
        let text = normalize_breaks(text);
        let text = text.as_ref();
        self.session(|session| {
            session.buffer.clear();
            session.buffer.push_str(text);
            session.table.rebuild(session.buffer);
            let end = session.document_end();
            session.selection = Selection {
                anchor: end,
                focus: end,
                base: Base::Cluster,
                h_pos: None,
            };
        });
        *self.history.borrow_mut() = History::default();
        *self.compose.borrow_mut() = None;
        self.sync_text();
    }

    /// Inserts `text` at the caret, replacing the selection.
    ///
    /// Typed characters group into one undo step until the caret moves, a
    /// different edit runs, or a whitespace character starts the next step,
    /// so undo removes text a word at a time. Inserting more than one
    /// character at once, as a paste does, is a step of its own. The text
    /// may hold line breaks, which cut the document into more paragraphs. A
    /// carriage return and line feed pair, or a lone carriage return, is
    /// inserted as one line feed.
    pub fn insert(&self, text: &str) {
        let text = normalize_breaks(text);
        let text = text.as_ref();
        let mut chars = text.chars();
        let grouping = match (chars.next(), chars.next()) {
            (Some(single), None) if single.is_whitespace() => Grouping::Starts,
            (Some(_), None) => Grouping::Joins,
            _ => Grouping::Alone,
        };
        self.edit(EditKind::Insert, grouping, |session| {
            let range = session.selection_range();
            let (byte, upstream) = Session::caret_after_insert(range.start, text);
            session.replace(range, text, byte, upstream)
        });
    }

    /// Inserts `text` from a clipboard, replacing the selection, as one undo
    /// step.
    ///
    /// Line breaks in the text cut the document into more paragraphs, with
    /// a carriage return and line feed pair, or a lone carriage return,
    /// taken as one line feed. An empty paste inserts nothing.
    pub fn paste(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = normalize_breaks(text);
        let text = text.as_ref();
        self.edit(EditKind::Insert, Grouping::Alone, |session| {
            let range = session.selection_range();
            let (byte, upstream) = Session::caret_after_insert(range.start, text);
            session.replace(range, text, byte, upstream)
        });
    }

    /// Deletes the selected text, or nothing when the caret is collapsed.
    ///
    /// This is the deletion a cut pairs with the copy, and it is an undo
    /// step of its own.
    pub fn delete_selection(&self) {
        self.edit(EditKind::Delete, Grouping::Alone, |session| {
            let range = session.selection_range();
            if range.is_empty() {
                return false;
            }
            let byte = range.start;
            session.replace(range, "", byte, true)
        });
    }

    /// Returns the selected text, or `None` when the selection is collapsed.
    pub fn selected_text(&self) -> Option<String> {
        let range = self.selection();
        if range.is_empty() {
            return None;
        }
        Some(self.buffer.borrow()[range].to_owned())
    }

    /// Shows, updates or clears the text an input method is composing.
    ///
    /// Non-empty `text` becomes the preedit, which replaces the previous
    /// preedit, and `cursor`, the byte range of the input method's cursor
    /// inside `text`, moves the caret into the preedit, or hides it when
    /// `None`. A composition that starts over a selection deletes the
    /// selection first, the way typing over it would, and that deletion is an
    /// undo step of its own. Empty `text` removes the preedit and leaves the
    /// caret where the preedit started.
    ///
    /// The preedit is not the editor's text: [`Editor::text`] leaves it out
    /// and the bound signal is not written, so a canceled or lost
    /// composition leaves the value as it was. Only [`Editor::commit`] makes
    /// composing text stick.
    ///
    /// Cursor offsets past the end of `text`, or inside one of its
    /// characters, which a broken input method could report, are clamped or
    /// snapped to the text.
    pub fn compose(&self, text: &str, cursor: Option<(usize, usize)>) {
        if text.is_empty() {
            self.cancel_compose();
            return;
        }
        if !self.is_composing() {
            // The platform clears the preedit with an empty text right
            // before it commits, so the selection has to go when the
            // composition starts: a selection kept for a cancel to restore
            // would come back right before the commit and stay next to the
            // committed text.
            self.delete_selection();
        }
        let cursor =
            cursor.map(|(anchor, focus)| (char_boundary(text, anchor), char_boundary(text, focus)));
        self.session(|session| {
            let (replaced, start) = if let Some(previous) = self.compose.borrow_mut().as_ref() {
                let range = previous.range.clone();
                session.buffer.replace_range(range.clone(), text);
                let start = range.start;
                (range, start)
            } else {
                let start = session.selection_range().start;
                session.buffer.insert_str(start, text);
                (start..start, start)
            };
            let range = start..start + text.len();
            *self.compose.borrow_mut() = Some(Compose {
                range: range.clone(),
                cursor,
            });
            session.table.splice(replaced, text);
            let (anchor, focus) = match cursor {
                Some((anchor, focus)) => (start + anchor, start + focus),
                None => (start, start),
            };
            let anchor = session.cursor_anchor(anchor);
            let focus = session.cursor_anchor(focus);
            session.selection = Selection {
                anchor,
                focus,
                base: Base::Cluster,
                h_pos: None,
            };
        });
    }

    /// Removes the preedit of the running composition, if there is one.
    fn cancel_compose(&self) {
        if self.compose.borrow().is_some() {
            self.session(|session| {
                let Some(compose) = self.compose.borrow_mut().take() else {
                    return;
                };
                let range = compose.range;
                session.buffer.replace_range(range.clone(), "");
                session.table.splice(range.clone(), "");
                let caret = session.cursor_anchor(range.start);
                session.selection = Selection {
                    anchor: caret,
                    focus: caret,
                    base: Base::Cluster,
                    h_pos: None,
                };
            });
        }
    }

    /// Commits `text` from an input method, replacing any preedit with it.
    ///
    /// A commit can arrive while the preedit is still showing, in which case
    /// the committed text takes its place, or after the platform already
    /// cleared it, in which case it is inserted at the caret. The committed
    /// text groups for undo like typed text: one character joins the word
    /// around it, and longer text is a step of its own. An empty commit only
    /// removes the preedit.
    pub fn commit(&self, text: &str) {
        if text.is_empty() {
            self.cancel_compose();
            return;
        }
        self.insert(text);
    }

    /// Deletes one grapheme before the caret, or the selection.
    ///
    /// A backspace before a combining mark takes the mark off first, and one
    /// after a line break joins the two lines. Repeated deletes group into
    /// one undo step until the caret moves or a different edit runs.
    pub fn backspace(&self) {
        self.edit(EditKind::Delete, Grouping::Joins, |session| {
            if session.selection.anchor != session.selection.focus {
                let range = session.selection_range();
                let byte = range.start;
                return session.replace(range, "", byte, true);
            }
            let focus = session.selection.focus;
            let Some(range) = session.backspace_range(focus) else {
                return false;
            };
            let byte = range.start;
            session.replace(range, "", byte, false)
        });
    }

    /// Deletes one word before the caret, or the selection, as one undo step.
    pub fn backspace_word(&self) {
        self.edit(EditKind::Delete, Grouping::Alone, |session| {
            if session.selection.anchor != session.selection.focus {
                let range = session.selection_range();
                let byte = range.start;
                return session.replace(range, "", byte, true);
            }
            let focus = session.selection.focus;
            let Some(start) = session.word_before(focus) else {
                return false;
            };
            let end = session.table.global(focus);
            if start >= end {
                return false;
            }
            let byte = start;
            session.replace(byte..end, "", byte, false)
        });
    }

    /// Deletes one grapheme after the caret, or the selection.
    ///
    /// A delete before a line break joins the two lines. Repeated deletes
    /// group into one undo step until the caret moves or a different edit
    /// runs.
    pub fn delete(&self) {
        self.edit(EditKind::Delete, Grouping::Joins, |session| {
            if session.selection.anchor != session.selection.focus {
                let range = session.selection_range();
                let byte = range.start;
                return session.replace(range, "", byte, true);
            }
            let focus = session.selection.focus;
            let Some(range) = session.forward_delete_range(focus) else {
                return false;
            };
            let byte = range.start;
            session.replace(range, "", byte, false)
        });
    }

    /// Deletes one word after the caret, or the selection, as one undo step.
    pub fn delete_word(&self) {
        self.edit(EditKind::Delete, Grouping::Alone, |session| {
            if session.selection.anchor != session.selection.focus {
                let range = session.selection_range();
                let byte = range.start;
                return session.replace(range, "", byte, true);
            }
            let focus = session.selection.focus;
            let start = session.table.global(focus);
            let Some(end) = session.word_after(focus) else {
                return false;
            };
            if start >= end {
                return false;
            }
            let byte = start;
            session.replace(byte..end, "", byte, false)
        });
    }

    /// Undoes the last undo step, restoring the text and the selection from
    /// before it, or does nothing when there is none.
    ///
    /// Nothing is undone while an input method is composing, because the
    /// composition owns the text around the caret until it commits.
    pub fn undo(&self) {
        if self.is_composing() {
            return;
        }
        let Some(previous) = self.history.borrow_mut().pop_undo() else {
            return;
        };
        let current = self.snapshot();
        let mut history = self.history.borrow_mut();
        history.push_redo(current);
        history.open = None;
        drop(history);
        self.restore(previous);
    }

    /// Redoes the last undone step, or does nothing when there is none.
    ///
    /// Any edit after an undo discards the steps it could have redone.
    pub fn redo(&self) {
        if self.is_composing() {
            return;
        }
        let Some(next) = self.history.borrow_mut().pop_redo() else {
            return;
        };
        let current = self.snapshot();
        let mut history = self.history.borrow_mut();
        history.push_undo(current);
        history.open = None;
        drop(history);
        self.restore(next);
    }

    /// Moves the caret one grapheme to the left, in visual order.
    pub fn move_left(&self) {
        self.caret_move(|session| session.move_horizontally(false, false));
    }

    /// Moves the caret one grapheme to the right, in visual order.
    pub fn move_right(&self) {
        self.caret_move(|session| session.move_horizontally(true, false));
    }

    /// Moves the caret one word to the left, in visual order.
    pub fn move_word_left(&self) {
        self.caret_move(|session| session.move_word(false, false));
    }

    /// Moves the caret one word to the right, in visual order.
    pub fn move_word_right(&self) {
        self.caret_move(|session| session.move_word(true, false));
    }

    /// Moves the caret one visual line up, keeping its horizontal position.
    pub fn move_up(&self) {
        self.caret_move(|session| session.move_lines(-1, false));
    }

    /// Moves the caret one visual line down, keeping its horizontal position.
    pub fn move_down(&self) {
        self.caret_move(|session| session.move_lines(1, false));
    }

    /// Moves the caret `lines` visual lines down, or up when negative,
    /// keeping its horizontal position where the move can.
    pub fn move_lines(&self, lines: isize) {
        self.caret_move(|session| session.move_lines(lines, false));
    }

    /// Moves the caret to the start of the text.
    pub fn move_to_start(&self) {
        self.caret_move(|session| session.move_to_edge(false, false));
    }

    /// Moves the caret to the end of the text.
    pub fn move_to_end(&self) {
        self.caret_move(|session| session.move_to_edge(true, false));
    }

    /// Moves the caret to the start of the visual line it sits on.
    ///
    /// A paragraph wrapped over several lines moves within the wrapped line,
    /// not to the start of the paragraph.
    pub fn move_to_line_start(&self) {
        self.caret_move(|session| session.move_to_line_edge(false, false));
    }

    /// Moves the caret to the end of the visual line it sits on.
    ///
    /// The end of a wrapped line sits after the space the line broke at, on
    /// the same line.
    pub fn move_to_line_end(&self) {
        self.caret_move(|session| session.move_to_line_edge(true, false));
    }

    /// Moves the selection focus one grapheme to the left.
    pub fn extend_left(&self) {
        self.caret_move(|session| session.move_horizontally(false, true));
    }

    /// Moves the selection focus one grapheme to the right.
    pub fn extend_right(&self) {
        self.caret_move(|session| session.move_horizontally(true, true));
    }

    /// Moves the selection focus one word to the left.
    pub fn extend_word_left(&self) {
        self.caret_move(|session| session.move_word(false, true));
    }

    /// Moves the selection focus one word to the right.
    pub fn extend_word_right(&self) {
        self.caret_move(|session| session.move_word(true, true));
    }

    /// Moves the selection focus one visual line up.
    pub fn extend_up(&self) {
        self.caret_move(|session| session.move_lines(-1, true));
    }

    /// Moves the selection focus one visual line down.
    pub fn extend_down(&self) {
        self.caret_move(|session| session.move_lines(1, true));
    }

    /// Moves the selection focus `lines` visual lines down, or up when
    /// negative.
    pub fn extend_lines(&self, lines: isize) {
        self.caret_move(|session| session.move_lines(lines, true));
    }

    /// Moves the selection focus to the start of the text.
    pub fn extend_to_start(&self) {
        self.caret_move(|session| session.move_to_edge(false, true));
    }

    /// Moves the selection focus to the end of the text.
    pub fn extend_to_end(&self) {
        self.caret_move(|session| session.move_to_edge(true, true));
    }

    /// Moves the selection focus to the start of the visual line it sits
    /// on.
    pub fn extend_to_line_start(&self) {
        self.caret_move(|session| session.move_to_line_edge(false, true));
    }

    /// Moves the selection focus to the end of the visual line it sits on.
    pub fn extend_to_line_end(&self) {
        self.caret_move(|session| session.move_to_line_edge(true, true));
    }

    /// Selects the whole text.
    pub fn select_all(&self) {
        self.caret_move(|session| {
            let end = session.document_end();
            session.selection = Selection {
                anchor: START,
                focus: end,
                base: Base::Cluster,
                h_pos: None,
            };
        });
    }

    /// Places the caret at the point, in the editor's own coordinates.
    ///
    /// The coordinates are relative to the top left of the text, which a
    /// widget gets by subtracting where it draws the text from the pointer
    /// position. A point below the last line places the caret at the end of
    /// its paragraph, and one past a line's end places it at that line's
    /// end.
    pub fn move_to_point(&self, x: f32, y: f32) {
        self.caret_move(|session| {
            let target = session.point_anchor(x, y);
            session.set_focus(target, false);
        });
    }

    /// Moves the selection focus to the point, in the editor's own
    /// coordinates, keeping the anchor where it was.
    ///
    /// The extension follows the granularity a double or triple click left
    /// the selection at: by cluster after a plain drag, and by word after a
    /// double click.
    pub fn extend_to_point(&self, x: f32, y: f32) {
        self.caret_move(|session| session.extend_to_point(x, y));
    }

    /// Selects the word under the point, in the editor's own coordinates.
    pub fn select_word_at_point(&self, x: f32, y: f32) {
        self.caret_move(|session| session.select_word_at_point(x, y));
    }

    /// Returns the rectangle of the caret, `width` logical pixels wide, in
    /// the editor's own coordinates, or `None` when there is no caret.
    ///
    /// A caret an input method hid, by composing without a cursor, is none.
    pub fn cursor_geometry(&self, width: f32) -> Option<Rect> {
        if self
            .compose
            .borrow()
            .as_ref()
            .is_some_and(|compose| compose.cursor.is_none())
        {
            return None;
        }
        self.session(|session| {
            let focus = session.selection.focus;
            let (caret, y) = session.caret_box(focus, width);
            Some(Rect::new(caret.x0, caret.y0 + y, caret.x1, caret.y1 + y))
        })
    }

    /// Returns one rectangle per selected piece of a line, in the editor's
    /// own coordinates.
    ///
    /// A collapsed selection yields no rectangles.
    pub fn selection_geometry(&self) -> Vec<Rect> {
        self.session(|session| {
            let range = session.selection_range();
            if range.is_empty() {
                return Vec::new();
            }
            session
                .range_rects(range)
                .into_iter()
                .map(|(area, _)| area)
                .collect()
        })
    }

    /// Returns the width and height of the laid out text.
    ///
    /// The width is the longest paragraph, and the height is the sum of
    /// every paragraph's lines.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`].
    pub fn layout_size(&self) -> Size {
        self.session(|session| {
            session.refresh_all();
            let mut size = Size {
                width: 0.0,
                height: 0.0,
            };
            for paragraph in &session.table.paragraphs {
                let paragraph_size = paragraph
                    .layout
                    .as_ref()
                    .expect("refresh built every layout")
                    .size();
                size.width = size.width.max(paragraph_size.width);
                size.height += paragraph_size.height;
            }
            size
        })
    }

    /// Converts the laid out text into glyph runs at `origin`, in paint
    /// order, with every run in `color`.
    ///
    /// `origin` is where the top left corner of the text box goes, in the
    /// caller's paint coordinates. A widget that paints a large document
    /// walks [`Editor::paragraphs`] instead and builds the runs of the
    /// paragraphs it shows, so an edit elsewhere does not rebuild them.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`], in particular
    /// when called from inside another `with` call on the same thread.
    pub fn glyph_runs_at(&self, origin: Point, color: Color) -> Vec<GlyphRun> {
        self.session(|session| {
            session.refresh_all();
            let mut runs = Vec::new();
            let mut y = 0.0;
            for paragraph in &session.table.paragraphs {
                let layout = paragraph
                    .layout
                    .as_ref()
                    .expect("refresh built every layout");
                runs.extend(glyph_runs(
                    &layout.layout,
                    Point {
                        x: origin.x,
                        y: origin.y + y,
                    },
                    color,
                ));
                y += layout.size().height;
            }
            runs
        })
    }

    /// Walks the editor's laid out paragraphs, top to bottom, handing each
    /// to `f`.
    ///
    /// A painter walks these to build its glyph runs one paragraph at a
    /// time: each paragraph carries a stamp that changes when its layout
    /// does, so a cache keyed on the stamp rebuilds only the paragraphs an
    /// edit touched, and a painter that shows part of the text builds the
    /// paragraphs it shows alone.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`].
    pub fn paragraphs(&self, mut f: impl FnMut(EditorParagraph<'_>)) {
        self.session(|session| session.refresh_all());
        let table = self.paragraphs.borrow();
        let mut y = 0.0;
        for paragraph in &table.paragraphs {
            let layout = paragraph
                .layout
                .as_ref()
                .expect("paragraphs built every layout");
            f(EditorParagraph {
                layout,
                stamp: paragraph.stamp,
                y,
            });
            y += layout.size().height;
        }
    }

    /// Writes the editor's text into the bound signal, when it changed.
    ///
    /// An equal write would wake every binding that reads the signal for
    /// nothing, so the comparison is what keeps an idle editor from asking
    /// for frames.
    fn sync_text(&self) {
        if !self.value.read(|current| self.has_text(current)) {
            self.value.set(self.text());
        }
    }

    /// Runs one edit of `kind` and records it in the undo history.
    ///
    /// `f` returns whether it changed the text, and an edit that changes no
    /// text records nothing. One that [`Grouping::Joins`] extends the open
    /// step when the last edit was of the same kind and left the selection
    /// where this one starts, and otherwise starts a step of its own, which
    /// is the only time the text is copied for undo. Any edit discards the
    /// steps redo could have taken, and one that is [`Grouping::Alone`]
    /// leaves no step open behind it.
    fn edit(&self, kind: EditKind, grouping: Grouping, f: impl FnOnce(&mut Session<'_>) -> bool) {
        // An edit while composing ends the composition first, so the edit
        // acts on the committed text and the preedit never shifts across an
        // edit.
        self.cancel_compose();
        let starts = self.selection();
        let continues = grouping == Grouping::Joins
            && self
                .history
                .borrow()
                .open
                .as_ref()
                .is_some_and(|(open, ended)| *open == kind && *ended == starts);
        let before = (!continues).then(|| self.snapshot());
        if !self.session(f) {
            return;
        }
        let mut history = self.history.borrow_mut();
        if let Some(before) = before {
            history.push_undo(before);
        }
        history.clear_redo();
        history.open = (grouping != Grouping::Alone).then(|| (kind, self.selection()));
        drop(history);
        self.sync_text();
    }

    /// Returns the text and the selection as they are now.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            // A snapshot only happens outside compositions, where the
            // buffer is the committed text.
            text: self.buffer.borrow().clone(),
            selection: *self.selection.borrow(),
        }
    }

    /// Puts the text and the selection of `snapshot` back, and writes the
    /// text to the bound signal.
    fn restore(&self, snapshot: Snapshot) {
        self.session(|session| {
            session.buffer.clear();
            session.buffer.push_str(&snapshot.text);
            session.table.rebuild(session.buffer);
            session.selection = snapshot.selection;
        });
        self.sync_text();
    }

    /// Lets `f` act on the editor's text, paragraph table and selection.
    ///
    /// The session borrows the parts an operation touches and works on a
    /// copy of the selection, which the editor stores back afterwards.
    fn session<R>(&self, f: impl FnOnce(&mut Session<'_>) -> R) -> R {
        let mut buffer = self.buffer.borrow_mut();
        let mut table = self.paragraphs.borrow_mut();
        let style = self.style.borrow();
        let mut session = Session {
            buffer: &mut buffer,
            table: &mut table,
            style: &style,
            width: self.width.get(),
            builds: &self.builds,
            selection: *self.selection.borrow(),
        };
        let result = f(&mut session);
        *self.selection.borrow_mut() = session.selection;
        result
    }

    /// Runs a caret move, which ends the open undo step: text typed after
    /// the caret moved is a step of its own, even where the caret came back
    /// to.
    fn caret_move(&self, f: impl FnOnce(&mut Session<'_>)) {
        self.session(f);
        self.history.borrow_mut().open = None;
    }

    /// Drops every paragraph layout so the next need rebuilds them.
    fn invalidate_layouts(&self) {
        for paragraph in self.paragraphs.borrow_mut().paragraphs.iter_mut() {
            paragraph.layout = None;
        }
    }
}

/// The anchor the caret starts at in an empty document.
const START: Anchor = Anchor {
    paragraph: 0,
    index: 0,
    affinity: Affinity::Downstream,
};

/// One laid out paragraph of an editor.
pub struct EditorParagraph<'a> {
    /// The paragraph's layout.
    layout: &'a TextLayout,
    /// The editor's build count when the layout was built, which changes
    /// when the paragraph is laid out again.
    stamp: u64,
    /// The paragraph's offset from the top of the text, in logical pixels.
    y: f32,
}

impl EditorParagraph<'_> {
    /// Returns the stamp the paragraph's layout was built under, which
    /// changes when the paragraph is laid out again.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Returns where the paragraph's top left corner sits, relative to the
    /// top left of the text, in logical pixels.
    pub fn offset(&self) -> Point {
        Point { x: 0.0, y: self.y }
    }

    /// Returns the width and height of the laid out paragraph in logical
    /// pixels.
    pub fn size(&self) -> Size {
        self.layout.size()
    }

    /// Converts the paragraph into glyph runs at `origin`, in paint order,
    /// with every run in `color`.
    ///
    /// `origin` is where the top left corner of the paragraph goes, in the
    /// caller's paint coordinates.
    pub fn glyph_runs_at(&self, origin: Point, color: Color) -> Vec<GlyphRun> {
        self.layout.glyph_runs_at(origin, color)
    }
}

/// The state one operation on an editor acts on.
///
/// The operation borrows the editor's text and paragraph table through the
/// editor, works on a copy of the selection, and lays out the paragraphs it
/// needs. The editor stores the selection back and counts the operation as
/// a change when the operation says it edited the text or the selection's
/// ends moved.
struct Session<'a> {
    /// The editor's text, including any composing preedit.
    buffer: &'a mut String,
    /// The paragraphs the buffer is cut into, with their layout caches.
    table: &'a mut ParagraphTable,
    /// The font style the paragraphs lay out with.
    style: &'a TextStyle,
    /// The width paragraphs wrap at, in logical pixels.
    width: Option<f32>,
    /// The editor's build count, which paragraph stamps are counted against.
    builds: &'a Cell<u64>,
    /// A copy of the selection the operation works on.
    selection: Selection,
}

impl Session<'_> {
    /// The byte range the selection covers, in text order.
    fn selection_range(&self) -> Range<usize> {
        let anchor = self.table.global(self.selection.anchor);
        let focus = self.table.global(self.selection.focus);
        anchor.min(focus)..anchor.max(focus)
    }

    /// The anchor a cursor at `byte` takes.
    ///
    /// A cursor the input method places takes the downstream side of its
    /// cluster, or the upstream side of the text's end, which no cluster
    /// follows. `upstream` flips that for the ends a deletion leaves, which
    /// parley builds from the upstream side.
    fn cursor_anchor(&self, byte: usize) -> Anchor {
        self.anchor_at(byte, false)
    }

    fn anchor_at(&self, byte: usize, upstream: bool) -> Anchor {
        let (paragraph, index) = self.table.locate(byte);
        let affinity = if byte == 0 {
            // No cluster ends at the start of the text.
            Affinity::Downstream
        } else if byte >= self.buffer.len() || upstream {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        Anchor {
            paragraph,
            index,
            affinity,
        }
    }

    /// The caret byte and side an insert of `text` at `start` leaves, which
    /// sits after the text, on the side the text's ending names.
    fn caret_after_insert(start: usize, text: &str) -> (usize, bool) {
        (
            start + text.len(),
            !text.ends_with(['\n', '\r', '\u{2028}', '\u{2029}']),
        )
    }

    /// The anchor of the end of the text.
    fn document_end(&self) -> Anchor {
        let last = self.table.paragraphs.len() - 1;
        Anchor {
            paragraph: last,
            index: self.table.paragraphs[last].len,
            affinity: Affinity::Upstream,
        }
    }

    /// Builds the paragraph's layout when it is missing.
    fn ensure(&mut self, index: usize) {
        if self.table.paragraphs[index].layout.is_some() {
            return;
        }
        let paragraph = &self.table.paragraphs[index];
        let text = &self.buffer[paragraph.start..paragraph.start + paragraph.len];
        let layout = TextSystem::with(|system| {
            TextLayout::build(
                text,
                self.style,
                self.width,
                &mut system.fonts,
                &mut system.layouts,
            )
        });
        self.builds.set(self.builds.get() + 1);
        let paragraph = &mut self.table.paragraphs[index];
        paragraph.layout = Some(layout);
        paragraph.stamp = self.builds.get();
    }

    /// Builds every paragraph layout that is missing.
    fn refresh_all(&mut self) {
        if !self
            .table
            .paragraphs
            .iter()
            .any(|paragraph| paragraph.layout.is_none())
        {
            return;
        }
        for index in 0..self.table.paragraphs.len() {
            self.ensure(index);
        }
    }

    /// Builds the paragraph's layout when it is missing and runs `f` with it.
    fn with_layout<R>(&mut self, index: usize, f: impl FnOnce(&TextLayout) -> R) -> R {
        self.ensure(index);
        let layout = self.table.paragraphs[index]
            .layout
            .as_ref()
            .expect("ensure built the layout");
        f(layout)
    }

    /// The paragraph the point's `y` falls in, with the paragraph's offset
    /// from the top of the text.
    ///
    /// A point below the last paragraph names the last one.
    fn paragraph_at_y(&mut self, y: f32) -> (usize, f32) {
        let mut offset = 0.0;
        let mut index = 0;
        loop {
            self.ensure(index);
            let height = self.table.paragraphs[index]
                .layout
                .as_ref()
                .expect("ensure built the layout")
                .size()
                .height;
            if y < offset + height || index + 1 >= self.table.paragraphs.len() {
                return (index, offset);
            }
            offset += height;
            index += 1;
        }
    }

    /// The offset the paragraph sits at from the top of the text, building
    /// the layouts the sum runs over.
    fn offset_of(&mut self, index: usize) -> f32 {
        let mut offset = 0.0;
        for i in 0..index {
            self.ensure(i);
            offset += self.table.paragraphs[i]
                .layout
                .as_ref()
                .expect("ensure built the layout")
                .size()
                .height;
        }
        offset
    }

    /// The rectangle of a paragraph-local caret in its paragraph, with the
    /// paragraph's offset from the top of the text.
    fn caret_box(&mut self, anchor: Anchor, width: f32) -> (Rect, f32) {
        let y = self.offset_of(anchor.paragraph);
        self.with_layout(anchor.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, anchor.index, anchor.affinity);
            let area = cursor.geometry(&layout.layout, width);
            (
                Rect::new(
                    area.x0 as f32,
                    area.y0 as f32,
                    area.x1 as f32,
                    area.y1 as f32,
                ),
                y,
            )
        })
    }

    /// One rectangle per line piece the byte range covers, in the editor's
    /// coordinates, with the paragraph's offset from the top of the text.
    ///
    /// A line break inside the range shows as a narrow rectangle after the
    /// last line of its paragraph, so a selected empty line still shows.
    fn range_rects(&mut self, range: Range<usize>) -> Vec<(Rect, f32)> {
        self.refresh_all();
        let newline_width = NEWLINE_WIDTH * self.style.size;
        let count = self.table.paragraphs.len();
        let mut rects = Vec::new();
        let mut y = 0.0;
        for index in 0..count {
            let paragraph = &self.table.paragraphs[index];
            let start = paragraph.start;
            let end = start + paragraph.len;
            let layout = paragraph
                .layout
                .as_ref()
                .expect("refresh built every layout");
            if range.start < end && range.end > start {
                let local_start = range.start.max(start) - start;
                let local_end = range.end.min(end) - start;
                if local_start < local_end {
                    let selection = ParagraphSelection::new(
                        Cursor::from_byte_index(&layout.layout, local_start, Affinity::Downstream),
                        Cursor::from_byte_index(&layout.layout, local_end, Affinity::Downstream),
                    );
                    for (area, _) in selection.geometry(&layout.layout) {
                        let area = rect(area);
                        if area.width() > 0.0 {
                            rects.push((Rect::new(area.x0, area.y0 + y, area.x1, area.y1 + y), y));
                        }
                    }
                }
            }
            if index + 1 < count
                && range.start <= end
                && range.end > end
                && let Some(line) = layout.layout.get(layout.layout.len().saturating_sub(1))
            {
                let metrics = line.metrics();
                let top = metrics.block_min_coord.min(metrics.content_block_min_coord);
                let bottom = metrics.block_max_coord.max(metrics.content_block_max_coord);
                let caret =
                    Cursor::from_byte_index(&layout.layout, end - start, Affinity::Upstream)
                        .geometry(&layout.layout, 0.0);
                let x = caret.x0 as f32;
                let (x0, x1) = if layout.layout.is_rtl() {
                    (x - newline_width, x)
                } else {
                    (x, x + newline_width)
                };
                rects.push((Rect::new(x0, top + y, x1, bottom + y), y));
            }
            y += layout.size().height;
        }
        rects
    }

    /// Collapses the selection at `focus`, or extends to it, and resets the
    /// granularity a shift click extends with.
    fn set_focus(&mut self, focus: Anchor, extend: bool) {
        if extend {
            self.selection.focus = focus;
        } else {
            self.selection.anchor = focus;
            self.selection.focus = focus;
        }
        self.selection.base = Base::Cluster;
        self.selection.h_pos = None;
    }

    /// Replaces `range` with `text` and leaves the selection collapsed at
    /// `caret_byte`, on the side `upstream` names.
    ///
    /// The caret anchor is read after the table followed the edit, so an
    /// insert of text that holds line breaks names the paragraph the caret
    /// actually landed in. Returns whether the text changed: a replacement
    /// with the same text only moves the caret, and keeps the layouts.
    fn replace(
        &mut self,
        range: Range<usize>,
        text: &str,
        caret_byte: usize,
        upstream: bool,
    ) -> bool {
        let changed = self.buffer[range.clone()] != *text;
        if changed {
            if range.is_empty() {
                self.buffer.insert_str(range.start, text);
            } else {
                self.buffer.replace_range(range.clone(), text);
            }
            self.table.splice(range, text);
        }
        let caret = self.anchor_at(caret_byte, upstream);
        self.selection = Selection {
            anchor: caret,
            focus: caret,
            base: Base::Cluster,
            h_pos: None,
        };
        changed
    }

    /// Moves the focus one cluster left or right, in visual order.
    fn move_horizontally(&mut self, forward: bool, extend: bool) {
        if !extend && self.selection.anchor != self.selection.focus {
            // A plain move over a selection collapses to the end the
            // movement points at. Which end that is reads from the
            // geometry, because in a bidirectional paragraph the text order
            // does not decide what comes first.
            let (first, last) = self.visual_ends();
            self.set_focus(if forward { last } else { first }, false);
            return;
        }
        let focus = self.selection.focus;
        if !forward && self.at_visual_left_edge(focus) {
            // The previous cluster is the line break that ends the previous
            // paragraph, which no layout holds.
            if focus.paragraph > 0 {
                let previous = focus.paragraph - 1;
                let target = Anchor {
                    paragraph: previous,
                    index: self.visual_right_index(previous),
                    affinity: Affinity::Downstream,
                };
                self.set_focus(target, extend);
            }
            return;
        }
        if forward && self.at_visual_right_edge(focus) {
            if focus.paragraph + 1 < self.table.paragraphs.len() {
                let next = focus.paragraph + 1;
                let target = Anchor {
                    paragraph: next,
                    index: self.visual_left_index(next),
                    affinity: Affinity::Downstream,
                };
                self.set_focus(target, extend);
            }
            return;
        }
        let moved = self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            let moved = if forward {
                cursor.next_visual(&layout.layout)
            } else {
                cursor.previous_visual(&layout.layout)
            };
            Anchor {
                paragraph: focus.paragraph,
                index: moved.index(),
                affinity: moved.affinity(),
            }
        });
        self.set_focus(moved, extend);
    }

    /// Whether the paragraph's dominant direction is right-to-left.
    fn is_rtl(&mut self, paragraph: usize) -> bool {
        self.with_layout(paragraph, |layout| layout.layout.is_rtl())
    }

    /// Whether a caret at `anchor` sits at the paragraph's visual left
    /// edge, with no cluster of the paragraph further left.
    ///
    /// Which logical index a visual edge holds hangs on the direction: the
    /// visual left edge is the logical start of a left-to-right paragraph
    /// and the logical end of a right-to-left one. An empty paragraph is
    /// both edges at once.
    fn at_visual_left_edge(&mut self, anchor: Anchor) -> bool {
        let len = self.table.paragraphs[anchor.paragraph].len;
        if len == 0 {
            return true;
        }
        if anchor.index >= len {
            return self.is_rtl(anchor.paragraph);
        }
        anchor.index == 0 && !self.is_rtl(anchor.paragraph)
    }

    /// Whether a caret at `anchor` sits at the paragraph's visual right
    /// edge, with no cluster of the paragraph further right.
    fn at_visual_right_edge(&mut self, anchor: Anchor) -> bool {
        let len = self.table.paragraphs[anchor.paragraph].len;
        if len == 0 {
            return true;
        }
        if anchor.index >= len {
            return !self.is_rtl(anchor.paragraph);
        }
        anchor.index == 0 && self.is_rtl(anchor.paragraph)
    }

    /// The logical index the caret holds at the visual right edge of a
    /// paragraph, which its direction decides.
    fn visual_right_index(&mut self, paragraph: usize) -> usize {
        if self.is_rtl(paragraph) {
            0
        } else {
            self.table.paragraphs[paragraph].len
        }
    }

    /// The logical index the caret holds at the visual left edge of a
    /// paragraph, which its direction decides.
    fn visual_left_index(&mut self, paragraph: usize) -> usize {
        if self.is_rtl(paragraph) {
            self.table.paragraphs[paragraph].len
        } else {
            0
        }
    }

    /// The selection's ends, in visual order: the first that comes first on
    /// the screen, the second that comes last.
    fn visual_ends(&mut self) -> (Anchor, Anchor) {
        let anchor = self.selection.anchor;
        let focus = self.selection.focus;
        if anchor.paragraph == focus.paragraph {
            let (first, last) = self.with_layout(anchor.paragraph, |layout| {
                let anchor_box =
                    Cursor::from_byte_index(&layout.layout, anchor.index, anchor.affinity)
                        .geometry(&layout.layout, 0.0);
                let focus_box =
                    Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity)
                        .geometry(&layout.layout, 0.0);
                if (anchor_box.y0, anchor_box.x0) > (focus_box.y0, focus_box.x0) {
                    (focus, anchor)
                } else {
                    (anchor, focus)
                }
            });
            (first, last)
        } else if anchor.paragraph < focus.paragraph {
            (anchor, focus)
        } else {
            (focus, anchor)
        }
    }

    /// Moves the focus one word left or right, in visual order.
    fn move_word(&mut self, forward: bool, extend: bool) {
        let focus = self.selection.focus;
        // The word step crosses the line break that ends or begins the
        // focus's paragraph, which no layout holds.
        let focus = if forward && self.at_visual_right_edge(focus) {
            if focus.paragraph + 1 >= self.table.paragraphs.len() {
                return;
            }
            let next = focus.paragraph + 1;
            Anchor {
                paragraph: next,
                index: self.visual_left_index(next),
                affinity: Affinity::Downstream,
            }
        } else if !forward && self.at_visual_left_edge(focus) {
            if focus.paragraph == 0 {
                return;
            }
            let previous = focus.paragraph - 1;
            Anchor {
                paragraph: previous,
                index: self.visual_right_index(previous),
                affinity: Affinity::Downstream,
            }
        } else {
            focus
        };
        let steps = self.table.paragraphs[focus.paragraph].len + 1;
        let moved = self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            let moved = visual_word(cursor, &layout.layout, forward, steps);
            Anchor {
                paragraph: focus.paragraph,
                index: moved.index(),
                affinity: moved.affinity(),
            }
        });
        self.set_focus(moved, extend);
    }

    /// The byte offset a backspace word deletes down to before the focus,
    /// or `None` when the focus opens the text.
    ///
    /// The offset is global, so the caller can delete it against the buffer
    /// directly.
    fn word_before(&mut self, focus: Anchor) -> Option<usize> {
        let start = self.table.paragraphs[focus.paragraph].start;
        if focus.index == 0 {
            if focus.paragraph == 0 {
                return None;
            }
            let previous = focus.paragraph - 1;
            let len = self.table.paragraphs[previous].len;
            let previous_start = self.table.paragraphs[previous].start;
            return self.with_layout(previous, |layout| {
                let cursor = Cursor::from_byte_index(&layout.layout, len, Affinity::Downstream);
                Some(previous_start + cursor.previous_logical_word(&layout.layout).index())
            });
        }
        self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            Some(start + cursor.previous_logical_word(&layout.layout).index())
        })
    }

    /// The byte offset a delete word deletes up to after the focus, or
    /// `None` when the focus closes the text.
    ///
    /// The offset is global, so the caller can delete it against the buffer
    /// directly.
    fn word_after(&mut self, focus: Anchor) -> Option<usize> {
        let start = self.table.paragraphs[focus.paragraph].start;
        if focus.index >= self.table.paragraphs[focus.paragraph].len {
            if focus.paragraph + 1 >= self.table.paragraphs.len() {
                return None;
            }
            let next = focus.paragraph + 1;
            let next_start = self.table.paragraphs[next].start;
            return self.with_layout(next, |layout| {
                let cursor = Cursor::from_byte_index(&layout.layout, 0, Affinity::Downstream);
                Some(next_start + cursor.next_logical_word(&layout.layout).index())
            });
        }
        self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            Some(start + cursor.next_logical_word(&layout.layout).index())
        })
    }

    /// Moves the focus `delta` visual lines down, or up when negative,
    /// walking across paragraphs and keeping the horizontal position.
    fn move_lines(&mut self, delta: isize, extend: bool) {
        if delta == 0 {
            return;
        }
        let focus = self.selection.focus;
        let fallback_x = self.with_layout(focus.paragraph, |layout| {
            Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity)
                .geometry(&layout.layout, 0.0)
                .x0 as f32
        });
        let h_pos = self.selection.h_pos.unwrap_or(fallback_x);
        let mut paragraph = focus.paragraph;
        let mut line = self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            let area = cursor.geometry(&layout.layout, 0.0);
            let middle = ((area.y0 + area.y1) / 2.0) as f32;
            line_at_y(&layout.layout, middle)
        });
        let mut remaining = delta;
        let mut clamped = false;
        while remaining != 0 {
            if remaining < 0 {
                if line > 0 {
                    line -= 1;
                    remaining += 1;
                } else if paragraph > 0 {
                    paragraph -= 1;
                    line =
                        self.with_layout(paragraph, |layout| layout.layout.len().saturating_sub(1));
                    remaining += 1;
                } else {
                    clamped = true;
                    break;
                }
            } else {
                let count = self.with_layout(paragraph, |layout| layout.layout.len());
                if line + 1 < count {
                    line += 1;
                    remaining -= 1;
                } else if paragraph + 1 < self.table.paragraphs.len() {
                    paragraph += 1;
                    line = 0;
                    remaining -= 1;
                } else {
                    clamped = true;
                    break;
                }
            }
        }
        if clamped {
            // A move past the ends of the text lands at the start or end of
            // the line it stopped on, and the sticky horizontal position
            // goes with it.
            let target = if remaining < 0 {
                Anchor {
                    paragraph: 0,
                    index: 0,
                    affinity: Affinity::Downstream,
                }
            } else {
                self.document_end()
            };
            self.set_focus(target, extend);
            return;
        }
        let moved = self.with_layout(paragraph, |layout| {
            let line = layout
                .layout
                .get(line)
                .expect("the walk stopped within the paragraph");
            let metrics = line.metrics();
            let middle = (metrics.block_min_coord + metrics.block_max_coord) / 2.0;
            let moved = Cursor::from_point(&layout.layout, h_pos, middle);
            Anchor {
                paragraph,
                index: moved.index(),
                affinity: moved.affinity(),
            }
        });
        if extend {
            self.selection.focus = moved;
            self.selection.base = Base::Cluster;
        } else {
            self.selection.anchor = moved;
            self.selection.focus = moved;
            self.selection.base = Base::Cluster;
        }
        self.selection.h_pos = Some(h_pos);
    }

    /// Moves the focus to the start or end of the text.
    fn move_to_edge(&mut self, end: bool, extend: bool) {
        let target = if end { self.document_end() } else { START };
        self.set_focus(target, extend);
    }

    /// Moves the focus to the start or end of the visual line it sits on.
    fn move_to_line_edge(&mut self, end: bool, extend: bool) {
        let focus = self.selection.focus;
        let moved = self.with_layout(focus.paragraph, |layout| {
            let layout = &layout.layout;
            let here = ParagraphSelection::from(Cursor::from_byte_index(
                layout,
                focus.index,
                focus.affinity,
            ));
            let moved = if end {
                here.line_end(layout, false)
            } else {
                here.line_start(layout, false)
            }
            .focus();
            Anchor {
                paragraph: focus.paragraph,
                index: moved.index(),
                affinity: moved.affinity(),
            }
        });
        self.set_focus(moved, extend);
    }

    /// The anchor the point names.
    fn point_anchor(&mut self, x: f32, y: f32) -> Anchor {
        let (paragraph, offset) = self.paragraph_at_y(y);
        self.with_layout(paragraph, |layout| {
            let moved = Cursor::from_point(&layout.layout, x, y - offset);
            Anchor {
                paragraph,
                index: moved.index(),
                affinity: moved.affinity(),
            }
        })
    }

    /// Moves the selection focus to the point, keeping the anchor, and
    /// extends at the granularity the selection was made at.
    fn extend_to_point(&mut self, x: f32, y: f32) {
        match self.selection.base {
            Base::Cluster => {
                let target = self.point_anchor(x, y);
                self.selection.focus = target;
                self.selection.h_pos = None;
            }
            Base::Word(base) => {
                // Extend over the word the click landed in, taking in every
                // byte between the word's far end and the double click's
                // anchor, whichever side the click came from.
                let (target_anchor, target_focus) = self.word_at_point(x, y);
                let mut ends = [target_anchor, target_focus, base[0], base[1]];
                ends.sort_by_key(|anchor| self.table.global(*anchor));
                self.selection.anchor = ends[0];
                self.selection.focus = ends[3];
                self.selection.h_pos = None;
            }
        }
    }

    /// Selects the word under the point.
    fn select_word_at_point(&mut self, x: f32, y: f32) {
        let (anchor, focus) = self.word_at_point(x, y);
        self.selection = Selection {
            anchor,
            focus,
            base: Base::Word([anchor, focus]),
            h_pos: None,
        };
    }

    /// The two ends of the word under the point, collapsed to the point's
    /// paragraph end when the paragraph holds no words.
    fn word_at_point(&mut self, x: f32, y: f32) -> (Anchor, Anchor) {
        let (paragraph, offset) = self.paragraph_at_y(y);
        let word = self.with_layout(paragraph, |layout| {
            let layout = &layout.layout;
            let (mut cluster, _) = Cluster::from_point(layout, x, y - offset)?;
            if !cluster.is_word_boundary()
                && let Some(previous) = cluster.previous_logical_word()
            {
                cluster = previous;
            }
            let anchor = Cursor::from_byte_index(
                layout,
                cluster.text_range().start,
                affinity_for_dir(cluster.is_rtl(), true),
            );
            let focus = match cluster.next_logical_word() {
                Some(end) => Cursor::from_byte_index(
                    layout,
                    end.text_range().start,
                    affinity_for_dir(end.is_rtl(), true),
                ),
                None => Cursor::from_byte_index(layout, usize::MAX, Affinity::Downstream),
            };
            Some((
                Anchor {
                    paragraph,
                    index: anchor.index(),
                    affinity: anchor.affinity(),
                },
                Anchor {
                    paragraph,
                    index: focus.index(),
                    affinity: focus.affinity(),
                },
            ))
        });
        word.unwrap_or_else(|| {
            let end = Anchor {
                paragraph,
                index: self.table.paragraphs[paragraph].len,
                affinity: Affinity::Upstream,
            };
            (end, end)
        })
    }

    /// The byte range one backspace deletes before a collapsed focus, or
    /// `None` when there is nothing before it.
    fn backspace_range(&mut self, focus: Anchor) -> Option<Range<usize>> {
        if focus.paragraph == 0 && focus.index == 0 {
            return None;
        }
        let start = self.table.paragraphs[focus.paragraph].start;
        if focus.index == 0 {
            // The cluster before the caret is the line break that ends the
            // previous paragraph, which no layout holds.
            return Some(start - 1..start);
        }
        let paragraph_start = start;
        let mut range = None;
        let mut whole = false;
        self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            if let Some(cluster) = cursor.upstream_cluster(&layout.layout) {
                range = Some(cluster.text_range());
                whole = cluster.is_hard_line_break() || cluster.is_emoji();
            }
        });
        let range = range?;
        let end = paragraph_start + range.end;
        let start = if whole {
            paragraph_start + range.start
        } else {
            // A combining mark comes off before the whole cluster does.
            let text = &self.buffer[paragraph_start..end];
            let (start, _) = text.char_indices().next_back()?;
            paragraph_start + start
        };
        Some(start..end)
    }

    /// The byte range one delete removes after a collapsed focus, or `None`
    /// when there is nothing after it.
    fn forward_delete_range(&mut self, focus: Anchor) -> Option<Range<usize>> {
        let paragraph = &self.table.paragraphs[focus.paragraph];
        let start = paragraph.start;
        let len = paragraph.len;
        if focus.index >= len {
            if focus.paragraph + 1 >= self.table.paragraphs.len() {
                return None;
            }
            // The next cluster is the line break that ends the paragraph,
            // which no layout holds.
            return Some(start + len..start + len + 1);
        }
        let mut range = None;
        self.with_layout(focus.paragraph, |layout| {
            let cursor = Cursor::from_byte_index(&layout.layout, focus.index, focus.affinity);
            if let Some(cluster) = cursor.downstream_cluster(&layout.layout) {
                range = Some(cluster.text_range());
            }
        });
        range.map(|local| start + local.start..start + local.end)
    }
}

/// Returns `text` with every carriage return and line feed pair, and every
/// lone carriage return, turned into one line feed.
///
/// The buffer is cut into paragraphs at line feeds alone, and a carriage
/// return left at a paragraph's end would lay out as a second, empty line.
/// Text without a carriage return is borrowed as it is.
fn normalize_breaks(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Returns `byte` clamped to `text` and snapped down to a character
/// boundary, which an offset from a broken input method may need.
fn char_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// Converts one of parley's boxes into a rectangle in logical pixels.
fn rect(area: parley::BoundingBox) -> Rect {
    Rect::new(
        area.x0 as f32,
        area.y0 as f32,
        area.x1 as f32,
        area.y1 as f32,
    )
}

/// The affinity a cursor takes when it arrives at a cluster from the given
/// side, which bidirectional text flips.
fn affinity_for_dir(is_rtl: bool, moving_right: bool) -> Affinity {
    match (is_rtl, moving_right) {
        (true, true) | (false, false) => Affinity::Downstream,
        _ => Affinity::Upstream,
    }
}

/// The index of the layout line whose vertical range holds `y`, in the
/// layout's own coordinates, or the last line when the point falls past
/// every line.
fn line_at_y(layout: &parley::Layout<()>, y: f32) -> usize {
    for (index, line) in layout.lines().enumerate() {
        if y < line.metrics().block_max_coord {
            return index;
        }
    }
    layout.len().saturating_sub(1)
}

/// Returns the cursor one word away from `cursor` in visual order, the
/// way parley's `next_visual_word` and `previous_visual_word` find it, but
/// after at most `steps` cluster steps.
///
/// Parley's visual step can cycle between two positions in mixed-direction
/// text that wraps, and its word walk loops until a step stops moving, so
/// it never returns there. A paragraph of `len` bytes holds at most `len`
/// clusters, so `len + 1` steps reach any boundary a walk without a cycle
/// reaches.
fn visual_word<B: parley::Brush>(
    cursor: Cursor,
    layout: &parley::Layout<B>,
    forward: bool,
    steps: usize,
) -> Cursor {
    let mut current = cursor;
    for _ in 0..steps {
        let next = if forward {
            current.next_visual(layout)
        } else {
            current.previous_visual(layout)
        };
        if next == current {
            break;
        }
        current = next;
        let [Some(left), Some(right)] = current.visual_clusters(layout) else {
            break;
        };
        let boundary = match (forward, left.is_rtl()) {
            (true, true) => left.is_word_boundary() && !left.is_space_or_nbsp(),
            (true, false) => right.is_word_boundary() && !left.is_space_or_nbsp(),
            (false, true) => {
                left.is_word_boundary()
                    && (left.is_space_or_nbsp()
                        || (right.is_word_boundary() && !right.is_space_or_nbsp()))
            }
            (false, false) => right.is_word_boundary() && !right.is_space_or_nbsp(),
        };
        if boundary {
            break;
        }
    }
    current
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
