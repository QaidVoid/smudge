//! The editing engine behind a text input.
//!
//! [`Editor`] wraps parley's `PlainEditor`, keeps the bound signal in step
//! with every edit, and turns parley's layouts into the glyph runs and
//! geometry a widget paints. All of parley stays behind this wrapper, so a
//! widget and a test drive the same edits without touching parley types for
//! anything but geometry.

use std::cell::RefCell;
use std::ops::Range;

use parley::Cluster;
use parley::StyleProperty;
use parley::editing::PlainEditor;
use parley::editing::PlainEditorDriver;
use parley::editing::Selection;
use parley::style::LineHeight;

use parley::Generation;
use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::paint::GlyphRun;
use smudge_core::reactive::{RwSignal, prelude::*};

use crate::glyphs::glyph_runs;
use crate::style::TextStyle;
use crate::system::TextSystem;

/// The font size an editor starts with, before a style is applied.
const DEFAULT_FONT_SIZE: f32 = 14.0;

/// How many steps an editor can undo. Older steps are forgotten.
const HISTORY_LIMIT: usize = 100;

/// How far below the baseline the composing text is underlined, in logical
/// pixels.
const UNDERLINE_OFFSET: f32 = 1.0;

/// How thick the underline under the composing text is, in logical pixels.
const UNDERLINE_THICKNESS: f32 = 1.0;

/// One single-line text buffer bound to a signal.
///
/// The editor holds the text and the selection; the signal mirrors the text.
/// Every mutating method writes the signal, so [`Editor::text`] and the
/// signal always agree. A write to the signal from outside is applied with [`Editor::set_text`], which replaces the text and
/// moves the caret to the end.
///
/// The editor lays out lazily: parley marks its layout stale on every edit,
/// and the next call that needs geometry refreshes it.
///
/// # Panics
///
/// Every method that edits the text, moves the caret or reads geometry runs
/// the thread's [`TextSystem`], so it panics under the conditions of
/// [`TextSystem::with`], in particular when called from inside another
/// `with` call on the same thread. No input text or edit sequence panics.
pub struct Editor {
    /// parley's editor, behind a cell so painting, which only has `&self`,
    /// can refresh the layout before it reads geometry.
    inner: RefCell<PlainEditor<()>>,
    /// The signal the text is bound to.
    value: RwSignal<String>,
    /// The steps undo and redo walk through.
    history: RefCell<History>,
}

/// The text and selection of an editor at one moment.
struct Snapshot {
    /// The text, without any composing preedit.
    text: String,
    /// The selection, as parley keeps it.
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
    undo: Vec<Snapshot>,
    /// The states undo left, most recently undone last.
    redo: Vec<Snapshot>,
    /// The kind of the last edit and the selection it left, which the next
    /// edit of the same kind joins when it starts where that one ended.
    open: Option<(EditKind, Range<usize>)>,
}

impl Editor {
    /// Creates an editor bound to `value`, starting empty.
    ///
    /// The text the signal holds is not read: an editor starts empty and
    /// fills when its widget applies the signal's current value. Binding
    /// `value` through a widget does that on mount.
    pub fn new(value: RwSignal<String>) -> Editor {
        let mut inner = PlainEditor::new(DEFAULT_FONT_SIZE);
        // A single line never wraps: overflow is handled by scrolling, not
        // by breaking lines.
        inner.set_width(None);
        Editor {
            inner: RefCell::new(inner),
            value,
            history: RefCell::new(History::default()),
        }
    }

    /// Returns the signal the text is bound to.
    pub fn value(&self) -> RwSignal<String> {
        self.value.clone()
    }

    /// Applies `style` to the text the editor lays out.
    ///
    /// The next layout refresh picks the style up. Changing the font size
    /// changes what the editor measures, so a widget applies this before it
    /// measures.
    pub fn set_style(&self, style: &TextStyle) {
        let mut editor = self.inner.borrow_mut();
        let styles = editor.edit_styles();
        styles.insert(StyleProperty::FontFamily(style.family.clone()));
        styles.insert(StyleProperty::FontSize(style.size));
        styles.insert(StyleProperty::FontWeight(style.weight));
        styles.insert(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
            style.line_height,
        )));
    }

    /// Returns the text of the editor, without any composing preedit.
    pub fn text(&self) -> String {
        self.inner.borrow().text().to_string()
    }

    /// Returns whether the editor holds no text.
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().text().chars().next().is_none()
    }

    /// Returns whether the editor holds exactly `text`.
    pub fn has_text(&self, text: &str) -> bool {
        self.inner.borrow().text() == text
    }

    /// Returns the text of the editor, including any composing preedit.
    pub fn raw_text(&self) -> String {
        self.inner.borrow().raw_text().to_owned()
    }

    /// Returns the byte range of the selection, into [`Editor::raw_text`].
    ///
    /// The range is empty and both ends equal when the caret is collapsed.
    pub fn selection(&self) -> Range<usize> {
        self.inner.borrow().raw_selection().text_range()
    }

    /// Returns whether an input method is composing text in the editor.
    pub fn is_composing(&self) -> bool {
        self.inner.borrow().is_composing()
    }

    /// Returns the editor's layout generation, which changes whenever the
    /// editor laid its text out again.
    ///
    /// A painter keeps the generation it last built its glyph runs with and
    /// rebuilds them when this one differs.
    pub(crate) fn generation(&self) -> Generation {
        self.inner.borrow().generation()
    }

    /// Returns the area the input method should anchor to, in the editor's
    /// own coordinates.
    ///
    /// This bounds the preedit while one is being composed, and the selected
    /// part of the focused line otherwise, inflated a little so a candidate
    /// window does not jump while the user types.
    pub fn ime_area(&self) -> Rect {
        self.refresh();
        rect(self.inner.borrow().ime_cursor_area())
    }

    /// Returns one rectangle per piece of the composing preedit, in the
    /// editor's own coordinates, for the underline drawn beneath it.
    ///
    /// The rectangles sit just below the baseline of each piece, in visual
    /// order, and the list is empty while no input method is composing.
    pub fn compose_geometry(&self) -> Vec<Rect> {
        self.refresh();
        let editor = self.inner.borrow();
        let Some(range) = editor.raw_compose().clone() else {
            return Vec::new();
        };
        let Some(layout) = editor.try_layout() else {
            return Vec::new();
        };
        let mut rects = Vec::new();
        let mut index = range.start;
        while index < range.end {
            let Some(cluster) = Cluster::from_byte_index(layout, index) else {
                break;
            };
            // A cluster can hold more than the byte the loop advanced to, so
            // the next index is always at least one past the current one.
            index = cluster.text_range().end.max(index + 1);
            let Some(offset) = cluster.visual_offset() else {
                continue;
            };
            let baseline = cluster.line().metrics().baseline;
            let top = baseline + UNDERLINE_OFFSET;
            rects.push(Rect::new(
                offset,
                top,
                offset + cluster.advance(),
                top + UNDERLINE_THICKNESS,
            ));
        }
        rects
    }

    /// Replaces the whole text and moves the caret to the end.
    ///
    /// This is how an outside write to the bound signal reaches the editor:
    /// the user typed somewhere else, or the application replaced the value.
    /// The selection collapses at the end, because a selection into text the
    /// editor no longer has means nothing. The undo history is cleared, so
    /// undo never brings back text the application replaced.
    pub fn set_text(&self, text: &str) {
        self.inner.borrow_mut().set_text(text);
        self.drive(|driver| driver.move_to_text_end());
        *self.history.borrow_mut() = History::default();
    }

    /// Inserts `text` at the caret, replacing the selection.
    ///
    /// Typed characters group into one undo step until the caret moves, a
    /// different edit runs, or a whitespace character starts the next step,
    /// so undo removes text a word at a time. Inserting more than one
    /// character at once, as a paste does, is a step of its own.
    pub fn insert(&self, text: &str) {
        let mut chars = text.chars();
        let grouping = match (chars.next(), chars.next()) {
            (Some(single), None) if single.is_whitespace() => Grouping::Starts,
            (Some(_), None) => Grouping::Joins,
            _ => Grouping::Alone,
        };
        self.edit(EditKind::Insert, grouping, |driver| {
            driver.insert_or_replace_selection(text);
        });
    }

    /// Inserts `text` from a clipboard, replacing the selection, as one undo
    /// step.
    ///
    /// Line breaks go, because the editor holds one line: a paste from a
    /// multi-line source pastes its text in one line. An empty paste, or one
    /// that was nothing but line breaks, inserts nothing.
    pub fn paste(&self, text: &str) {
        let single_line: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        if single_line.is_empty() {
            return;
        }
        self.edit(EditKind::Insert, Grouping::Alone, |driver| {
            driver.insert_or_replace_selection(&single_line);
        });
    }

    /// Deletes the selected text, or nothing when the caret is collapsed.
    ///
    /// This is the deletion a cut pairs with the copy, and it is an undo
    /// step of its own.
    pub fn delete_selection(&self) {
        self.edit(EditKind::Delete, Grouping::Alone, |driver| {
            driver.delete_selection();
        });
    }

    /// Returns the selected text, or `None` when the selection is collapsed.
    pub fn selected_text(&self) -> Option<String> {
        self.refresh();
        self.inner.borrow().selected_text().map(str::to_owned)
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
    /// Cursor offsets past the end of `text`, which a broken input method
    /// could report, are clamped to it.
    pub fn compose(&self, text: &str, cursor: Option<(usize, usize)>) {
        if text.is_empty() {
            self.cancel_compose();
            return;
        }
        if !self.is_composing() {
            // The platform clears the preedit with an empty text right before
            // it commits, so the selection has to go when the composition
            // starts: a selection kept for a cancel to restore would come
            // back right before the commit and stay next to the committed
            // text.
            self.delete_selection();
        }
        let end = text.len();
        let cursor = cursor.map(|(anchor, focus)| (anchor.min(end), focus.min(end)));
        self.drive(|driver| driver.set_compose(text, cursor));
    }

    /// Removes the preedit of the running composition, if there is one.
    fn cancel_compose(&self) {
        if self.is_composing() {
            self.drive(|driver| driver.clear_compose());
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
    /// Repeated deletes group into one undo step until the caret moves or a
    /// different edit runs.
    pub fn backspace(&self) {
        self.edit(EditKind::Delete, Grouping::Joins, |driver| {
            driver.backdelete();
        });
    }

    /// Deletes one word before the caret, or the selection, as one undo step.
    pub fn backspace_word(&self) {
        self.edit(EditKind::Delete, Grouping::Alone, |driver| {
            driver.backdelete_word();
        });
    }

    /// Deletes one grapheme after the caret, or the selection.
    ///
    /// Repeated deletes group into one undo step until the caret moves or a
    /// different edit runs.
    pub fn delete(&self) {
        self.edit(EditKind::Delete, Grouping::Joins, |driver| driver.delete());
    }

    /// Deletes one word after the caret, or the selection, as one undo step.
    pub fn delete_word(&self) {
        self.edit(EditKind::Delete, Grouping::Alone, |driver| {
            driver.delete_word();
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
        let Some(previous) = self.history.borrow_mut().undo.pop() else {
            return;
        };
        let current = self.snapshot();
        let mut history = self.history.borrow_mut();
        history.redo.push(current);
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
        let Some(next) = self.history.borrow_mut().redo.pop() else {
            return;
        };
        let current = self.snapshot();
        let mut history = self.history.borrow_mut();
        history.undo.push(current);
        history.open = None;
        drop(history);
        self.restore(next);
    }

    /// Moves the caret one grapheme to the left, in visual order.
    pub fn move_left(&self) {
        self.drive(|driver| driver.move_left());
    }

    /// Moves the caret one grapheme to the right, in visual order.
    pub fn move_right(&self) {
        self.drive(|driver| driver.move_right());
    }

    /// Moves the caret one word to the left, in visual order.
    pub fn move_word_left(&self) {
        self.drive(|driver| driver.move_word_left());
    }

    /// Moves the caret one word to the right, in visual order.
    pub fn move_word_right(&self) {
        self.drive(|driver| driver.move_word_right());
    }

    /// Moves the caret to the start of the text.
    pub fn move_to_start(&self) {
        self.drive(|driver| driver.move_to_text_start());
    }

    /// Moves the caret to the end of the text.
    pub fn move_to_end(&self) {
        self.drive(|driver| driver.move_to_text_end());
    }

    /// Moves the selection focus one grapheme to the left.
    pub fn extend_left(&self) {
        self.drive(|driver| driver.select_left());
    }

    /// Moves the selection focus one grapheme to the right.
    pub fn extend_right(&self) {
        self.drive(|driver| driver.select_right());
    }

    /// Moves the selection focus one word to the left.
    pub fn extend_word_left(&self) {
        self.drive(|driver| driver.select_word_left());
    }

    /// Moves the selection focus one word to the right.
    pub fn extend_word_right(&self) {
        self.drive(|driver| driver.select_word_right());
    }

    /// Moves the selection focus to the start of the text.
    pub fn extend_to_start(&self) {
        self.drive(|driver| driver.select_to_text_start());
    }

    /// Moves the selection focus to the end of the text.
    pub fn extend_to_end(&self) {
        self.drive(|driver| driver.select_to_text_end());
    }

    /// Selects the whole text.
    pub fn select_all(&self) {
        self.drive(|driver| driver.select_all());
    }

    /// Places the caret at the point, in the editor's own coordinates.
    ///
    /// The coordinates are relative to the top left of the layout, which a
    /// widget gets by subtracting where it draws the layout from the pointer
    /// position.
    pub fn move_to_point(&self, x: f32, y: f32) {
        self.drive(|driver| driver.move_to_point(x, y));
    }

    /// Moves the selection focus to the point, in the editor's own
    /// coordinates, keeping the anchor where it was.
    pub fn extend_to_point(&self, x: f32, y: f32) {
        self.drive(|driver| driver.extend_selection_to_point(x, y));
    }

    /// Selects the word under the point, in the editor's own coordinates.
    pub fn select_word_at_point(&self, x: f32, y: f32) {
        self.drive(|driver| driver.select_word_at_point(x, y));
    }

    /// Refreshes the layout when the editor changed since it was last drawn.
    fn refresh(&self) {
        self.drive(|driver| {
            driver.refresh_layout();
        });
    }

    /// Returns the rectangle of the caret, `width` logical pixels wide, in
    /// the editor's own coordinates, or `None` when there is no caret.
    pub fn cursor_geometry(&self, width: f32) -> Option<Rect> {
        self.refresh();
        self.inner.borrow().cursor_geometry(width).map(rect)
    }

    /// Returns one rectangle per selected piece of the line, in the editor's
    /// own coordinates.
    ///
    /// A collapsed selection yields no rectangles.
    pub fn selection_geometry(&self) -> Vec<Rect> {
        self.refresh();
        self.inner
            .borrow()
            .selection_geometry()
            .into_iter()
            .map(|(area, _line)| rect(area))
            .filter(|area| area.width() > 0.0)
            .collect()
    }

    /// Returns the width and height of the laid out text.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`].
    pub fn layout_size(&self) -> Size {
        self.refresh();
        let editor = self.inner.borrow();
        let layout = editor.try_layout().expect("refresh made the layout clean");
        Size {
            width: layout.width(),
            height: layout.height(),
        }
    }

    /// Converts the laid out text into glyph runs at `origin`, in paint
    /// order, with every run in `color`.
    ///
    /// `origin` is where the top left corner of the layout box goes, in
    /// logical window coordinates.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`], in particular
    /// when called from inside another `with` call on the same thread.
    pub fn glyph_runs_at(&self, origin: Point, color: Color) -> Vec<GlyphRun> {
        self.refresh();
        let borrow = self.inner.borrow();
        match borrow.try_layout() {
            Some(layout) => glyph_runs(layout, origin, color),
            None => Vec::new(),
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
    /// An edit that changes no text records nothing. One that
    /// [`Grouping::Joins`] extends the open step when the last edit was of
    /// the same kind and left the selection where this one starts, and
    /// otherwise starts a step of its own. Any edit discards the steps redo
    /// could have taken, and one that is [`Grouping::Alone`] leaves no step
    /// open behind it.
    fn edit(
        &self,
        kind: EditKind,
        grouping: Grouping,
        f: impl FnOnce(&mut PlainEditorDriver<'_, ()>),
    ) {
        // An edit while composing ends the composition first, so the edit
        // acts on the committed text and parley never shifts a preedit range
        // across an edit, which its byte arithmetic can leave inside a
        // character.
        self.cancel_compose();
        let before = self.snapshot();
        self.drive(f);
        if self.has_text(&before.text) {
            return;
        }
        let mut history = self.history.borrow_mut();
        let starts = before.selection.text_range();
        let continues = grouping == Grouping::Joins
            && history
                .open
                .as_ref()
                .is_some_and(|(open, ended)| *open == kind && *ended == starts);
        if !continues {
            if history.undo.len() == HISTORY_LIMIT {
                history.undo.remove(0);
            }
            history.undo.push(before);
        }
        history.redo.clear();
        history.open = (grouping != Grouping::Alone).then(|| (kind, self.selection()));
        drop(history);
        self.sync_text();
    }

    /// Returns the text and the selection as they are now.
    fn snapshot(&self) -> Snapshot {
        let editor = self.inner.borrow();
        Snapshot {
            text: editor.text().to_string(),
            selection: *editor.raw_selection(),
        }
    }

    /// Puts the text and the selection of `snapshot` back, and writes the
    /// text to the bound signal.
    fn restore(&self, snapshot: Snapshot) {
        self.inner.borrow_mut().set_text(&snapshot.text);
        self.drive(|driver| driver.set_selection(snapshot.selection));
        self.sync_text();
    }

    /// Runs `f` with parley's driver, which refreshes the layout around the
    /// operation.
    fn drive<R>(&self, f: impl FnOnce(&mut PlainEditorDriver<'_, ()>) -> R) -> R {
        TextSystem::with(|system| {
            let mut editor = self.inner.borrow_mut();
            let mut driver = editor.driver(&mut system.fonts, &mut system.layouts);
            f(&mut driver)
        })
    }
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
