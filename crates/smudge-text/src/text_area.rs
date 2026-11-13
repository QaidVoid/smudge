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
//! builder methods. The area fills the width it is offered and, without a
//! height from the style builders, grows with its wrapped text, never
//! shorter than one line or [`TextAreaExt::min_lines`] lines. Give it a
//! height to make it clip and scroll instead.

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
use smudge_core::widget::{AvailableSpace, MeasureInput, PaintCx, Panel, Widget};
use smudge_core::window::Cursor;

use crate::chrome::{
    Direction, InputChrome, ParagraphCache, Placeholder, SELECTION_ALPHA, apply_edit_count,
    apply_move, apply_outside_write, caret_node, copy_selection, cut_selection, handle_commit,
    handle_focus, handle_move, handle_preedit, handle_press, handle_release, handle_wheel,
    is_shortcut, outside_write, paint_paragraphs, paint_selection, types_text,
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
    /// The glyph runs of the editor's text, with everything they were built
    /// from.
    runs: RefCell<Option<ParagraphCache>>,
    /// The height of the wrapped text as the body last measured it.
    measured: Rc<Cell<f32>>,
    /// Bumped when an edit changed the height of the wrapped text, which
    /// lays the body out again in the same frame.
    relayout: RwSignal<u64>,
}

impl TextArea {
    /// Creates an area with an empty placeholder and the default style
    /// until the mount reads the ambient one, sharing the measured height
    /// and the relayout counter with its body.
    fn new(chrome: Rc<InputChrome>, measured: Rc<Cell<f32>>, relayout: RwSignal<u64>) -> TextArea {
        TextArea {
            chrome,
            placeholder: RefCell::new(Placeholder::new()),
            style: TextStyle::default(),
            runs: RefCell::new(None),
            measured,
            relayout,
        }
    }

    /// Lays the body out again when the wrapped text no longer has the
    /// height the body last measured, so an edit that adds or removes a
    /// line resizes the area in the frame that shows it, and one that does
    /// not runs no layout.
    fn relayout_if_resized(&self) {
        if self.chrome.editor.layout_size().height != self.measured.get() {
            self.relayout.update(|count| *count += 1);
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

    /// Paints the field's background and border, the way a panel does.
    ///
    /// The text and the selection paint in [`Widget::paint_inside`], clipped
    /// to the border, so scrolled or overflowing text never covers it.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        Panel.paint(cx, out);
    }

    /// Appends the selection, the underline under a composing preedit, the
    /// text or the placeholder, in that order, for the paragraphs the
    /// content box shows.
    ///
    /// Painting also records the content box, which the pointer handlers
    /// and the next edit read, and places the caret against it. An edit has
    /// placed the caret already, so this only moves it when the box itself
    /// changed, which lands one frame later like every size-driven change.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`](crate::TextSystem::with),
    /// or when the glyph run cache is already borrowed on this thread,
    /// which the paint pass cannot do.
    fn paint_inside(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let color = self.text_color(cx);
        let inset = Point {
            x: cx.content.x0 - cx.rect.x0,
            y: cx.content.y0 - cx.rect.y0,
        };
        self.chrome.painted.set(Some((cx.content, color, inset)));
        // The body measured the text at this width already; a layout that
        // gave the body another width than the content box still paints
        // the text wrapped to the box it shows in.
        self.chrome
            .editor
            .set_wrap_width(Some(cx.content.width().max(0.0)));
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
        paint_selection(
            &self.chrome,
            text_origin,
            (scroll_y, scroll_y + cx.content.height()),
            selection_color,
            out,
        );

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
/// Without a height from the style builders, the area is as tall as its
/// text wrapped at the area's width, and grows and shrinks with it in the
/// frame an edit lands, never below its one-line or
/// [`TextAreaExt::min_lines`] floor. The layout engine measures the text at
/// the width it gives the area, in the same pass, so the area shows the
/// right height from its first frame and right after a width change. A
/// height set with a style builder replaces that: the area keeps the
/// height, and the text clips and scrolls inside it.
///
/// The wheel scrolls the text while the pointer is over the area, and the
/// area takes a wheel event that moves its text, so a scroll view around the
/// area does not scroll with it. A wheel that cannot move the text, because
/// it fits or sits at its limit in the wheel's direction, passes on to the
/// area's ancestors, so a page around the area keeps scrolling.
pub fn text_area(value: RwSignal<String>) -> Element<TextArea> {
    let chrome = Rc::new(InputChrome::new(value.clone(), true));
    let measured = Rc::new(Cell::new(0.0));
    let relayout = RwSignal::new(0u64);
    let relayout_source = relayout.clone();

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

    let body = Element::new(Body {
        chrome: Rc::clone(&chrome),
        measured: Rc::clone(&measured),
    })
    .width(Length::pct(100.0))
    .bind(
        RelayoutKey,
        move || relayout_source.get(),
        |_, _| Dirty::Layout,
    );

    Element::new(TextArea::new(Rc::clone(&chrome), measured, relayout))
        .clip(true)
        .cursor(Cursor::Text)
        .column()
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
        .child(body)
        .child(caret_node(&caret_chrome))
}

/// The childless node inside an area that the layout engine measures.
///
/// The area itself carries its body and its caret, and the layout engine
/// measures childless nodes only, so the body is what tells the engine how
/// tall the text wraps at the width the area gives it, in the pass that
/// gives that width. The area paints the text over its own content box;
/// the body paints nothing.
struct Body {
    /// The state the area shares with its handlers and caret.
    chrome: Rc<InputChrome>,
    /// The height the last measure answered, which the area compares an
    /// edit's height against.
    measured: Rc<Cell<f32>>,
}

impl Widget for Body {
    /// Wraps the editor at the offered width and returns the wrapped text's
    /// width and height.
    ///
    /// A definite width re-breaks the text's lines without shaping them
    /// again, and the same width as before changes nothing. A request for
    /// the intrinsic width answers zero, since an area fills the width it is
    /// offered, with the height the text has at the width it last wrapped
    /// at.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`](crate::TextSystem::with).
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        let offered = input.known_width.or(match input.available_width {
            AvailableSpace::Definite(width) => Some(width),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        });
        let editor = &self.chrome.editor;
        if let Some(width) = offered {
            editor.set_wrap_width(Some(width.max(0.0)));
        }
        let height = editor.layout_size().height;
        self.measured.set(height);
        Some(Size {
            width: offered.unwrap_or(0.0).max(0.0),
            height,
        })
    }

    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
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
    /// taller when its text or a style builder's height needs more. The
    /// text scrolls inside whatever height the area ends up with.
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

/// The key of the body's relayout binding.
struct RelayoutKey;

impl TextAreaExt for Element<TextArea> {
    fn placeholder(self, text: impl Into<Value<String>>) -> Self {
        self.bind(PlaceholderKey, text, set_placeholder)
    }

    fn min_lines(self, lines: impl Into<Value<u32>>) -> Self {
        let lines = lines.into();
        self.min_height(move || line_floor(lines.clone()))
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
    area.relayout_if_resized();
    Dirty::Layout
}

/// Applies an outside write to the bound signal, which arrives as `None`
/// when the write was the editor's own.
fn apply_value(area: &mut TextArea, value: Option<String>) -> Dirty {
    match value {
        Some(value) => {
            let dirty = apply_outside_write(&area.chrome, &value);
            area.relayout_if_resized();
            dirty
        }
        None => Dirty::Clean,
    }
}

/// Applies an editor state change the handlers made, and lays the body out
/// again when the change resized the text, so an edit that adds or removes
/// a line resizes the area in the frame that shows it.
fn apply_edits(area: &mut TextArea, _edits: u64) -> Dirty {
    let dirty = apply_edit_count(&area.chrome);
    area.relayout_if_resized();
    dirty
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
