//! The text area: multi-line editable text bound to a signal.
//!
//! A [`TextArea`] drives an [`Editor`](crate::Editor) the way a [`TextInput`](crate::TextInput)
//! does, over text that spans paragraphs: Enter inserts a line break and is
//! handled, so it does not bubble; a paste keeps its line breaks; Up, Down,
//! Page Up and Page Down move by visual line with the horizontal position
//! kept where the move can; Home and End move within the visual line; and
//! the text wraps at the width the area is given, scrolling up and down
//! inside itself so the caret stays visible, and under the wheel.
//!
//! Create one with [`text_area`] and shape it with the [`TextAreaExt`]
//! builder methods. The area is one line tall, or
//! [`TextAreaExt::min_lines`] lines, by its content and fills the width it
//! is offered, so give it a height through the style builders when the
//! layout needs one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smudge_core::clipboard;
use smudge_core::element::{Element, Value};
use smudge_core::geometry::{Color, Corners, Point, Rect, Size};
use smudge_core::input::{Handled, Key, KeyEvent};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, Semantics};
use smudge_core::style::Length;
use smudge_core::theme::ColorToken;
use smudge_core::tree::Dirty;
use smudge_core::widget::{AvailableSpace, MeasureInput, PaintCx, Widget};
use smudge_core::window::Cursor;

use crate::chrome::{
    Direction, InputChrome, ParagraphCache, Placeholder, SELECTION_ALPHA, apply_edit_count,
    apply_move, apply_outside_write, caret_node, copy_selection, cut_selection, handle_commit,
    handle_focus, handle_move, handle_preedit, handle_press, handle_release, handle_wheel,
    is_shortcut, outside_write, paint_paragraphs, types_text,
};
use crate::style::{TextStyle, current_text_style};

/// A block of editable text.
///
/// Build one with [`text_area`]. The area keeps the editor and the glyph
/// runs of the paragraphs it shows, and rebuilds the runs only when the
/// editor laid a paragraph out again, the area moved or scrolled, or the
/// resolved text color changed.
pub struct TextArea {
    /// The state shared with the handlers and the caret node.
    chrome: Rc<InputChrome>,
    /// The text shown while the editor is empty.
    placeholder: RefCell<Placeholder>,
    /// The text style the editor lays out and the area paints with,
    /// resolved from the ambient style at mount.
    style: TextStyle,
    /// The height floor, in lines, the area measures.
    min_lines: u32,
    /// The width the editor was last told to wrap at, so a paint whose
    /// content box did not change asks the editor for nothing.
    wrap_width: Cell<Option<f32>>,
    /// The glyph runs of the editor's text, with everything they were built
    /// from.
    runs: RefCell<Option<ParagraphCache>>,
}

impl TextArea {
    /// Creates an area with an empty placeholder, a one-line floor and the
    /// default style until the mount reads the ambient one.
    fn new(chrome: Rc<InputChrome>) -> TextArea {
        TextArea {
            chrome,
            placeholder: RefCell::new(Placeholder::new()),
            style: TextStyle::default(),
            min_lines: 1,
            wrap_width: Cell::new(None),
            runs: RefCell::new(None),
        }
    }

    /// Returns the color the text and the caret draw with: the node's text
    /// color when it sets one, otherwise the area's style color.
    fn text_color(&self, cx: &PaintCx<'_>) -> Color {
        match &cx.style.paint.text_color {
            Some(color) => color.resolve(cx.theme),
            None => self.style.color.resolve(cx.theme),
        }
    }
}

impl Widget for TextArea {
    /// Reports the area's placeholder as the accessible name and its text
    /// as the accessible value.
    ///
    /// An area with no placeholder reports no name, which leaves the
    /// announcement to the area's surroundings.
    fn semantics(&self) -> Option<Semantics> {
        let label = (!self.placeholder.borrow().text().is_empty())
            .then(|| self.placeholder.borrow().text().into());
        Some(Semantics {
            role: Role::MultilineTextInput,
            label,
            value: Some(self.chrome.editor.text()),
        })
    }

    /// Returns the size of the area's line floor.
    ///
    /// Offered space is filled on the x axis, which is what an editing
    /// field conventionally does, and the intrinsic width is zero, so an
    /// area that is not stretched by its parent needs a width or a grow
    /// factor. The height is the [`TextAreaExt::min_lines`] floor, and does
    /// not depend on the text, so an edit repaints the area without laying
    /// it out again and the text scrolls inside whatever height the area
    /// ended up with.
    ///
    /// The floor also rides on the min-height style that
    /// [`TextAreaExt::min_lines`] sets, because the layout engine measures
    /// childless nodes only and the area always carries its caret child.
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        let width = match input.available_width {
            AvailableSpace::Definite(width) => width.max(0.0),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => 0.0,
        };
        Some(Size {
            width,
            height: self.min_lines as f32 * self.style.size * self.style.line_height,
        })
    }

    /// Appends the selection, the underline under a composing preedit, the
    /// text or the placeholder, in that order, for the paragraphs the
    /// content box shows.
    ///
    /// Painting also wraps the editor at the content width, records the
    /// content box, which the pointer handlers and the next edit read, and
    /// places the caret against it. An edit has placed the caret already,
    /// so this only moves it when the box itself changed, which lands one
    /// frame later like every size-driven change.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`](crate::TextSystem::with),
    /// or when the glyph run cache is already borrowed on this thread,
    /// which the paint pass cannot do.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let color = self.text_color(cx);
        let inset = Point {
            x: cx.content.x0 - cx.rect.x0,
            y: cx.content.y0 - cx.rect.y0,
        };
        self.chrome.painted.set(Some((cx.content, color, inset)));
        let wrap_width = Some(cx.content.width().max(0.0));
        if self.wrap_width.get() != wrap_width {
            self.chrome.editor.set_wrap_width(wrap_width);
            self.wrap_width.set(wrap_width);
        }
        let (scroll_x, scroll_y) = self.chrome.place(cx.content, color);
        self.chrome
            .line_height
            .set(self.style.size * self.style.line_height);
        let text_origin = Point {
            x: cx.content.x0 - scroll_x,
            y: cx.content.y0 - scroll_y,
        };

        let selection_color = Color {
            a: SELECTION_ALPHA,
            ..cx.theme.color(ColorToken::ACCENT)
        };
        for area in self.chrome.editor.selection_geometry() {
            out.push(PaintOp::FillRect {
                rect: Rect::new(
                    text_origin.x + area.x0,
                    text_origin.y + area.y0,
                    text_origin.x + area.x1,
                    text_origin.y + area.y1,
                ),
                radius: Corners::ZERO,
                color: selection_color,
            });
        }

        // The composing text is underlined so the user can tell it apart
        // from text that is in the area for good.
        for area in self.chrome.editor.compose_geometry() {
            out.push(PaintOp::FillRect {
                rect: Rect::new(
                    text_origin.x + area.x0,
                    text_origin.y + area.y0,
                    text_origin.x + area.x1,
                    text_origin.y + area.y1,
                ),
                radius: Corners::ZERO,
                color,
            });
        }

        if self.chrome.editor.is_empty()
            && !self.chrome.editor.is_composing()
            && !self.placeholder.borrow().text().is_empty()
        {
            let muted = cx.theme.color(ColorToken::TEXT_MUTED);
            self.placeholder
                .borrow_mut()
                .paint(&self.style, text_origin, muted, out);
        } else {
            let top = scroll_y;
            let bottom = scroll_y + cx.content.height();
            paint_paragraphs(
                &mut self.runs.borrow_mut(),
                &self.chrome.editor,
                text_origin,
                color,
                (top, bottom),
                out,
            );
        }
    }

    /// Reports the area the input method anchors to while the area holds
    /// the focus.
    ///
    /// The area is the text the user is editing, which is the preedit while
    /// one is being composed and the selection otherwise, in paint
    /// coordinates.
    fn ime_area(&self) -> Option<Rect> {
        let (content, _, _) = self.chrome.painted.get()?;
        let area = self.chrome.editor.ime_area();
        let scroll_x = self.chrome.scroll_x.get();
        let scroll_y = self.chrome.scroll_y.get();
        Some(Rect::new(
            content.x0 + area.x0 - scroll_x,
            content.y0 + area.y0 - scroll_y,
            content.x0 + area.x1 - scroll_x,
            content.y0 + area.y1 - scroll_y,
        ))
    }
}

/// Creates an area that edits `value`.
///
/// The area writes every edit to `value`, and a write to `value` from
/// outside replaces the text and moves the caret to the end, so the two
/// always agree:
///
/// ```
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_text::{TextAreaExt, text_area};
///
/// let draft = RwSignal::new(String::from("Dear reader,\n"));
/// let field = text_area(draft.clone()).min_lines(4);
///
/// // An outside write replaces the text.
/// draft.set(String::from("Dear editor,\n"));
/// ```
///
/// Enter inserts a line break and is handled, so it does not reach a
/// parent's key handler. With Control, Alt or the Meta key held, Enter
/// bubbles instead and inserts nothing, so a parent can take it as a
/// shortcut, such as one that submits the text.
///
/// The wheel scrolls the text while the pointer is over the area, and the
/// area takes the whole wheel event, the way a scroll view does, so a scroll
/// view around the area does not scroll with it.
pub fn text_area(value: RwSignal<String>) -> Element<TextArea> {
    let chrome = Rc::new(InputChrome::new(value.clone()));

    let key_chrome = Rc::clone(&chrome);
    let press_chrome = Rc::clone(&chrome);
    let move_chrome = Rc::clone(&chrome);
    let up_chrome = Rc::clone(&chrome);
    let wheel_chrome = Rc::clone(&chrome);
    let focus_chrome = Rc::clone(&chrome);
    let commit_chrome = Rc::clone(&chrome);
    let preedit_chrome = Rc::clone(&chrome);
    let edits_chrome = Rc::clone(&chrome);
    let caret_chrome = Rc::clone(&chrome);
    let value_chrome = Rc::clone(&chrome);

    Element::new(TextArea::new(Rc::clone(&chrome)))
        .clip(true)
        .cursor(Cursor::Text)
        .min_height(|| line_floor(Value::Static(1)))
        .bind(AmbientStyleKey, current_text_style, apply_ambient_style)
        .bind(
            ValueKey,
            move || outside_write(&value_chrome, &value),
            apply_value,
        )
        .bind(EditsKey, move || edits_chrome.edits.get(), apply_edits)
        .on_key(move |event| handle_key(&key_chrome, event))
        .on_pointer_down(move |position| handle_press(&press_chrome, position))
        .on_pointer_move(move |position| handle_move(&move_chrome, position))
        .on_pointer_up(move |position| handle_release(&up_chrome, position))
        .on_wheel(move |delta| handle_wheel(&wheel_chrome, delta))
        .on_focus(move |focused| handle_focus(&focus_chrome, focused))
        .on_ime_commit(move |text| handle_commit(&commit_chrome, text))
        .on_ime_preedit(move |text, cursor| handle_preedit(&preedit_chrome, text, cursor))
        .child(caret_node(&caret_chrome))
}

/// Builder methods on a [`text_area`] element.
///
/// Import the trait to use them:
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_text::TextAreaExt;
/// use smudge_text::text_area;
///
/// let field = text_area(RwSignal::new(String::new()))
///     .placeholder("Notes")
///     .min_lines(3);
/// ```
pub trait TextAreaExt: Sized {
    /// Sets the text shown while the area is empty.
    ///
    /// The placeholder paints in the theme's muted text color, so a theme
    /// switch repaints it.
    fn placeholder(self, text: impl Into<Value<String>>) -> Self;

    /// Sets the height floor of the area, in lines, which is one line until
    /// this sets it.
    ///
    /// The area measures at least `lines` lines of its style tall, and
    /// taller when a style builder gives it an explicit height. The text
    /// scrolls inside whatever height the area ends up with.
    fn min_lines(self, lines: impl Into<Value<u32>>) -> Self;
}

/// The key of the area's ambient text style binding.
struct AmbientStyleKey;

/// The key of the area's value binding.
struct ValueKey;

/// The key of the area's edit counter binding.
struct EditsKey;

/// The key of the area's placeholder binding.
struct PlaceholderKey;

/// The key of the area's line floor binding.
struct MinLinesKey;

impl TextAreaExt for Element<TextArea> {
    fn placeholder(self, text: impl Into<Value<String>>) -> Self {
        self.bind(PlaceholderKey, text, set_placeholder)
    }

    fn min_lines(self, lines: impl Into<Value<u32>>) -> Self {
        let lines = lines.into();
        self.bind(MinLinesKey, lines.clone(), set_min_lines)
            .min_height(move || line_floor(lines.clone()))
    }
}

/// Returns the height floor `lines` measures, from the ambient text style.
///
/// The floor is read when the node mounts, the same moment the area reads
/// the ambient style for its own text.
fn line_floor(lines: Value<u32>) -> Length {
    let lines = match lines {
        Value::Static(lines) => lines,
        Value::Dynamic(get) => get(),
    };
    let style = current_text_style();
    Length::px(lines as f32 * style.size * style.line_height)
}

/// Applies the ambient text style to the editor and the placeholder.
fn apply_ambient_style(area: &mut TextArea, ambient: TextStyle) -> Dirty {
    if area.style == ambient {
        return Dirty::Clean;
    }
    area.style = ambient;
    area.chrome.editor.set_style(&area.style);
    area.placeholder.borrow_mut().invalidate_style();
    *area.runs.borrow_mut() = None;
    Dirty::Layout
}

/// Applies an outside write to the bound signal, which arrives as `None`
/// when the write was the editor's own.
fn apply_value(area: &mut TextArea, value: Option<String>) -> Dirty {
    match value {
        Some(value) => apply_outside_write(&area.chrome, &value),
        None => Dirty::Clean,
    }
}

/// Applies an editor state change the handlers made.
fn apply_edits(area: &mut TextArea, _edits: u64) -> Dirty {
    apply_edit_count(&area.chrome)
}

/// Writes the placeholder into the area.
fn set_placeholder(area: &mut TextArea, text: String) -> Dirty {
    let mut placeholder = area.placeholder.borrow_mut();
    if placeholder.text() == text {
        return Dirty::Clean;
    }
    placeholder.set_text(text);
    Dirty::Paint
}

/// Writes the height floor into the area.
fn set_min_lines(area: &mut TextArea, lines: u32) -> Dirty {
    if area.min_lines == lines {
        return Dirty::Clean;
    }
    area.min_lines = lines;
    Dirty::Layout
}

/// The number of lines one page of the area holds, from the content box and
/// the line height the last paint measured.
///
/// A page is at least one line, so a page key moves even inside a squashed
/// area or before the first paint.
fn visible_lines(chrome: &InputChrome) -> isize {
    let content = chrome
        .painted
        .get()
        .map(|(content, _, _)| content.height())
        .unwrap_or(0.0);
    let line = chrome.line_height.get().max(1.0);
    ((content / line) as isize).max(1)
}

/// Edits the keyboard makes while the area holds the focus.
///
/// Enter inserts a line break and is handled, so it does not bubble, and
/// with Control, Alt or Meta held it bubbles without editing. A paste keeps
/// the line breaks of its text, because the area holds several lines. Up
/// and Down move by visual line with the horizontal position kept where the
/// move can, Page Up and Page Down move a page of lines, Home and End move
/// to the ends of the visual line, and with Control or Meta to the ends of
/// the text, and the rest works as the single-line input's keys do.
fn handle_key(chrome: &InputChrome, event: &KeyEvent) -> Handled {
    if !event.pressed {
        return Handled::No;
    }
    let modifiers = event.modifiers;
    let by_word = modifiers.control || modifiers.alt;
    match &event.key {
        Key::Character(text) if is_shortcut(modifiers) && text.eq_ignore_ascii_case("a") => {
            chrome.editor.select_all();
            chrome.touched();
            Handled::Yes
        }
        Key::Character(text) if is_shortcut(modifiers) && text.eq_ignore_ascii_case("c") => {
            copy_selection(chrome);
            Handled::Yes
        }
        Key::Character(text) if is_shortcut(modifiers) && text.eq_ignore_ascii_case("x") => {
            if cut_selection(chrome) {
                chrome.touched();
            }
            Handled::Yes
        }
        Key::Character(text) if is_shortcut(modifiers) && text.eq_ignore_ascii_case("v") => {
            paste_from_clipboard(chrome);
            Handled::Yes
        }
        Key::Character(text) if is_shortcut(modifiers) && text.eq_ignore_ascii_case("z") => {
            if modifiers.shift {
                chrome.editor.redo();
            } else {
                chrome.editor.undo();
            }
            chrome.touched();
            Handled::Yes
        }
        Key::Character(text)
            if cfg!(not(target_os = "macos"))
                && is_shortcut(modifiers)
                && text.eq_ignore_ascii_case("y") =>
        {
            chrome.editor.redo();
            chrome.touched();
            Handled::Yes
        }
        Key::Enter if modifiers.control || modifiers.alt || modifiers.meta => Handled::No,
        Key::Enter => {
            chrome.editor.insert("\n");
            chrome.touched();
            Handled::Yes
        }
        Key::Backspace => {
            if by_word {
                chrome.editor.backspace_word();
            } else {
                chrome.editor.backspace();
            }
            chrome.touched();
            Handled::Yes
        }
        Key::Delete => {
            if by_word {
                chrome.editor.delete_word();
            } else {
                chrome.editor.delete();
            }
            chrome.touched();
            Handled::Yes
        }
        Key::ArrowLeft => {
            apply_move(chrome, modifiers.shift, by_word, Direction::Left);
            Handled::Yes
        }
        Key::ArrowRight => {
            apply_move(chrome, modifiers.shift, by_word, Direction::Right);
            Handled::Yes
        }
        Key::ArrowUp => {
            if modifiers.shift {
                chrome.editor.extend_up();
            } else {
                chrome.editor.move_up();
            }
            chrome.touched();
            Handled::Yes
        }
        Key::ArrowDown => {
            if modifiers.shift {
                chrome.editor.extend_down();
            } else {
                chrome.editor.move_down();
            }
            chrome.touched();
            Handled::Yes
        }
        Key::PageUp => {
            let lines = visible_lines(chrome);
            if modifiers.shift {
                chrome.editor.extend_lines(-lines);
            } else {
                chrome.editor.move_lines(-lines);
            }
            chrome.touched();
            Handled::Yes
        }
        Key::PageDown => {
            let lines = visible_lines(chrome);
            if modifiers.shift {
                chrome.editor.extend_lines(lines);
            } else {
                chrome.editor.move_lines(lines);
            }
            chrome.touched();
            Handled::Yes
        }
        Key::Home => {
            let editor = &chrome.editor;
            match (modifiers.shift, modifiers.control || modifiers.meta) {
                (false, false) => editor.move_to_line_start(),
                (true, false) => editor.extend_to_line_start(),
                (false, true) => editor.move_to_start(),
                (true, true) => editor.extend_to_start(),
            }
            chrome.touched();
            Handled::Yes
        }
        Key::End => {
            let editor = &chrome.editor;
            match (modifiers.shift, modifiers.control || modifiers.meta) {
                (false, false) => editor.move_to_line_end(),
                (true, false) => editor.extend_to_line_end(),
                (false, true) => editor.move_to_end(),
                (true, true) => editor.extend_to_end(),
            }
            chrome.touched();
            Handled::Yes
        }
        Key::Character(text) => {
            if !types_text(modifiers) {
                return Handled::No;
            }
            chrome.editor.insert(text);
            chrome.touched();
            Handled::Yes
        }
        Key::Space => {
            if !types_text(modifiers) {
                return Handled::No;
            }
            chrome.editor.insert(" ");
            chrome.touched();
            Handled::Yes
        }
        _ => Handled::No,
    }
}

/// Inserts the text on the clipboard of the running app.
///
/// An app without a clipboard pastes nothing. The line breaks of the pasted
/// text are kept, because the area holds several lines.
fn paste_from_clipboard(chrome: &InputChrome) {
    if let Some(text) = clipboard::with(|slot| slot.get()).flatten() {
        chrome.editor.paste(&text);
        chrome.touched();
    }
}
