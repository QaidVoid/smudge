//! The editing engine behind a text input.
//!
//! [`Editor`] wraps parley's `PlainEditor`, keeps the bound signal in step
//! with every edit, and turns parley's layouts into the glyph runs and
//! geometry a widget paints. All of parley stays behind this wrapper, so a
//! widget and a test drive the same edits without touching parley types for
//! anything but geometry.

use std::cell::RefCell;
use std::ops::Range;

use parley::StyleProperty;
use parley::editing::PlainEditor;
use parley::editing::PlainEditorDriver;
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

/// One single-line text buffer bound to a signal.
///
/// The editor holds the text and the selection; the signal mirrors the text.
/// Every mutating method writes the signal, so
/// [`Editor::text`] and the signal always agree. A write to the signal from
/// outside is applied with [`Editor::set_text`], which replaces the text and
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

    /// Replaces the whole text and moves the caret to the end.
    ///
    /// This is how an outside write to the bound signal reaches the editor:
    /// the user typed somewhere else, or the application replaced the value.
    /// The selection collapses at the end, because a selection into text the
    /// editor no longer has means nothing.
    pub fn set_text(&self, text: &str) {
        self.inner.borrow_mut().set_text(text);
        self.drive(|driver| driver.move_to_text_end());
    }

    /// Inserts `text` at the caret, replacing the selection.
    pub fn insert(&self, text: &str) {
        self.drive(|driver| driver.insert_or_replace_selection(text));
        self.sync_text();
    }

    /// Deletes one grapheme before the caret, or the selection.
    pub fn backspace(&self) {
        self.drive(|driver| driver.backdelete());
        self.sync_text();
    }

    /// Deletes one word before the caret, or the selection.
    pub fn backspace_word(&self) {
        self.drive(|driver| driver.backdelete_word());
        self.sync_text();
    }

    /// Deletes one grapheme after the caret, or the selection.
    pub fn delete(&self) {
        self.drive(|driver| driver.delete());
        self.sync_text();
    }

    /// Deletes one word after the caret, or the selection.
    pub fn delete_word(&self) {
        self.drive(|driver| driver.delete_word());
        self.sync_text();
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
