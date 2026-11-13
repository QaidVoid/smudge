//! The text input: one line of editable text bound to a signal.
//!
//! A [`TextInput`] drives an [`Editor`](crate::Editor), paints the selection, the text and
//! a placeholder, and keeps a caret node that blinks on its own. The caret
//! is a separate node so that a blink, which changes two pixels' worth of
//! caret, damages the caret's rectangle alone instead of the whole input.
//!
//! Create one with [`text_input`] and shape it with the [`TextInputExt`]
//! builder methods. Editing is by grapheme, and with control or alt by word;
//! a mouse press places the caret, a drag selects and a double click selects
//! a word. Every edit writes the bound signal, and an outside write to the
//! signal replaces the text and moves the caret to its end. The input holds
//! one line: a line break in an outside write, a paste or an input method's
//! commit becomes a space, and the signal is written with the line the
//! input kept. Undo and redo
//! walk back and forth through the edits, a typed word at a time. While an
//! input method composes, the preedit is drawn underlined and reaches the
//! value only when it is committed, and the platform's command modifier
//! with C, X and V copies, cuts and pastes.
//!
//! For text that spans several lines, use [`TextArea`](crate::TextArea)
//! instead.

use std::cell::RefCell;
use std::rc::Rc;

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
    Direction, InputChrome, ParagraphCache, Placeholder, SELECTION_ALPHA, apply_active,
    apply_edit_count, apply_move, apply_outside_write, caret_node, copy_selection, cut_selection,
    handle_commit, handle_focus, handle_move, handle_preedit, handle_press, handle_release,
    is_shortcut, outside_write, paint_paragraphs, paint_selection, types_text,
};
use crate::style::{TextStyle, current_text_style};

/// One line of editable text.
///
/// Build one with [`text_input`]. The input keeps the editor and the glyph
/// runs of its current text, and rebuilds the runs only when the editor laid
/// its text out again, the node moved or the resolved text color changed.
pub struct TextInput {
    /// The state shared with the handlers and the caret node.
    chrome: Rc<InputChrome>,
    /// The text shown while the editor is empty.
    placeholder: RefCell<Placeholder>,
    /// The text style the editor lays out and the input paints with,
    /// resolved from the ambient style at mount.
    style: TextStyle,
    /// The glyph runs of the editor's text, with everything they were built
    /// from.
    runs: RefCell<Option<ParagraphCache>>,
}

impl TextInput {
    /// Creates an input with an empty placeholder and the default style
    /// until the mount reads the ambient one.
    fn new(chrome: Rc<InputChrome>) -> TextInput {
        TextInput {
            chrome,
            placeholder: RefCell::new(Placeholder::new()),
            style: TextStyle::default(),
            runs: RefCell::new(None),
        }
    }

    /// Returns the color the text and the caret draw with: the node's text
    /// color when it sets one, otherwise the input's style color.
    fn text_color(&self, cx: &PaintCx<'_>) -> Color {
        match &cx.style.paint.text_color {
            Some(color) => color.resolve(cx.theme),
            None => self.style.color.resolve(cx.theme),
        }
    }
}

impl Widget for TextInput {
    /// Reports the field's placeholder as the accessible name and its text
    /// as the accessible value.
    ///
    /// An input with no placeholder reports no name, which leaves the
    /// announcement to the field's surroundings.
    fn semantics(&self) -> Option<Semantics> {
        let label = (!self.placeholder.borrow().text().is_empty())
            .then(|| self.placeholder.borrow().text().into());
        Some(Semantics {
            role: Role::TextInput,
            label,
            value: Some(self.chrome.editor.text()),
        })
    }

    /// Returns the size of one line of text.
    ///
    /// Offered space is filled on the x axis, which is what an input field
    /// conventionally does, and the intrinsic width is zero, so a field that
    /// is not stretched by its parent needs a width or a grow factor. The
    /// height is one line of the input's style. Neither depends on the text,
    /// so an edit repaints the input without laying it out again.
    ///
    /// That height also rides on the min-height style the constructor sets,
    /// because the layout engine measures childless nodes only and the input
    /// always carries its caret child.
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        let width = match input.available_width {
            AvailableSpace::Definite(width) => width.max(0.0),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => 0.0,
        };
        Some(Size {
            width,
            height: self.style.size * self.style.line_height,
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
    /// text or the placeholder, in that order.
    ///
    /// The caret is a child node of the input and paints itself after this,
    /// so the paint order is selection, then preedit underline, then glyphs,
    /// then caret. Painting also records the content box, which the pointer
    /// handlers and the next edit read, and places the caret against it. An
    /// edit has placed the caret already, so this only moves it when the box
    /// itself changed, which lands one frame later like every size-driven
    /// change.
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
            (f32::NEG_INFINITY, f32::INFINITY),
            selection_color,
            out,
        );

        // The composing text is underlined so the user can tell it apart
        // from text that is in the field for good.
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
            paint_paragraphs(
                &mut self.runs.borrow_mut(),
                &self.chrome.editor,
                text_origin,
                color,
                (f32::NEG_INFINITY, f32::INFINITY),
                out,
            );
        }
    }

    /// Reports the area the input method anchors to while the input holds
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

/// Creates an input that edits `value`.
///
/// The input writes every edit to `value`, and a write to `value` from
/// outside replaces the text and moves the caret to the end, so the two
/// always agree:
///
/// ```
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_text::text_input;
///
/// let name = RwSignal::new(String::from("Ada"));
/// let field = text_input(name.clone());
///
/// // An outside write replaces the text.
/// name.set(String::from("Grace"));
/// ```
///
/// The input is one line tall by its content and fills the width it is
/// offered, so give it a height through the style builders when the layout
/// needs one.
///
/// Only the field that holds the focus shows its selection. A field that
/// loses the focus keeps the selection and shows it again when the focus
/// returns. While the window is in the background, the focused field keeps
/// the focus but hides its caret and its selection and stops blinking, and
/// both show again when the window comes back.
pub fn text_input(value: RwSignal<String>) -> Element<TextInput> {
    let chrome = Rc::new(InputChrome::new(value.clone(), false));

    let key_chrome = Rc::clone(&chrome);
    let press_chrome = Rc::clone(&chrome);
    let move_chrome = Rc::clone(&chrome);
    let up_chrome = Rc::clone(&chrome);
    let focus_chrome = Rc::clone(&chrome);
    let active_chrome = Rc::clone(&chrome);
    let commit_chrome = Rc::clone(&chrome);
    let preedit_chrome = Rc::clone(&chrome);
    let edits_chrome = Rc::clone(&chrome);
    let caret_chrome = Rc::clone(&chrome);
    let value_chrome = Rc::clone(&chrome);

    Element::new(TextInput::new(Rc::clone(&chrome)))
        .clip(true)
        .cursor(Cursor::Text)
        .min_height(one_line)
        .bind(AmbientStyleKey, current_text_style, apply_ambient_style)
        .bind(
            ValueKey,
            move || outside_write(&value_chrome, &value),
            apply_value,
        )
        .bind(EditsKey, move || edits_chrome.edits.get(), apply_edits)
        .bind(
            ActiveKey,
            move || active_chrome.track_active(),
            apply_active,
        )
        .on_key(move |event| handle_key(&key_chrome, event))
        .on_pointer_down(move |position| handle_press(&press_chrome, position))
        .on_pointer_move(move |position| handle_move(&move_chrome, position))
        .on_pointer_up(move |position| handle_release(&up_chrome, position))
        .on_focus(move |focused| handle_focus(&focus_chrome, focused))
        .on_ime_commit(move |text| handle_commit(&commit_chrome, text))
        .on_ime_preedit(move |text, cursor| handle_preedit(&preedit_chrome, text, cursor))
        .child(caret_node(&caret_chrome))
}

/// Builder methods on a [`text_input`] element.
///
/// Import the trait to use them:
///
/// ```
/// use smudge_text::TextInputExt;
/// use smudge_text::text_input;
/// use smudge_core::reactive::RwSignal;
///
/// let field = text_input(RwSignal::new(String::new())).placeholder("Name");
/// ```
pub trait TextInputExt: Sized {
    /// Sets the text shown while the input is empty.
    ///
    /// The placeholder paints in the theme's muted text color, so a theme
    /// switch repaints it.
    fn placeholder(self, text: impl Into<Value<String>>) -> Self;
}

/// The key of the input's ambient text style binding.
struct AmbientStyleKey;

/// The key of the input's value binding.
struct ValueKey;

/// The key of the input's edit counter binding.
struct EditsKey;

/// The key of the input's binding that shows and hides its caret and selection.
struct ActiveKey;

/// The key of the input's placeholder binding.
struct PlaceholderKey;

impl TextInputExt for Element<TextInput> {
    fn placeholder(self, text: impl Into<Value<String>>) -> Self {
        self.bind(PlaceholderKey, text, set_placeholder)
    }
}

/// Returns the height of one line of the ambient text style.
///
/// The floor is read when the node mounts, the same moment the input reads
/// the ambient style for its own text.
fn one_line() -> Length {
    let style = current_text_style();
    Length::px(style.size * style.line_height)
}

/// Applies the ambient text style to the editor and the placeholder.
fn apply_ambient_style(input: &mut TextInput, ambient: TextStyle) -> Dirty {
    if input.style == ambient {
        return Dirty::Clean;
    }
    input.style = ambient;
    input.chrome.editor.set_style(&input.style);
    input.placeholder.borrow_mut().invalidate_style();
    *input.runs.borrow_mut() = None;
    Dirty::Layout
}

/// Applies an outside write to the bound signal, which arrives as `None`
/// when the write was the editor's own.
fn apply_value(input: &mut TextInput, value: Option<String>) -> Dirty {
    match value {
        Some(value) => apply_outside_write(&input.chrome, &value),
        None => Dirty::Clean,
    }
}

/// Applies an editor state change the handlers made.
fn apply_edits(input: &mut TextInput, _edits: u64) -> Dirty {
    apply_edit_count(&input.chrome)
}

/// Writes the placeholder into the input.
fn set_placeholder(input: &mut TextInput, text: String) -> Dirty {
    let mut placeholder = input.placeholder.borrow_mut();
    if placeholder.text() == text {
        return Dirty::Clean;
    }
    placeholder.set_text(text);
    Dirty::Paint
}

/// Edits the keyboard makes while the input holds the focus.
///
/// Printable text inserts; Backspace and Delete remove by grapheme, or by
/// word with control or alt held; Left and Right move by grapheme, or by
/// word; Home and End jump to the ends; Shift extends instead of moving; and
/// the platform's command modifier with A selects everything, with Z undoes,
/// with Shift and Z redoes, as Control and Y also does off macOS, and with
/// C, X and V copies, cuts and pastes. Keys the input does not take, among
/// them Enter and Tab, bubble to the ancestors.
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
        Key::Home => {
            if modifiers.shift {
                chrome.editor.extend_to_start();
            } else {
                chrome.editor.move_to_start();
            }
            chrome.touched();
            Handled::Yes
        }
        Key::End => {
            if modifiers.shift {
                chrome.editor.extend_to_end();
            } else {
                chrome.editor.move_to_end();
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
/// An app without a clipboard pastes nothing. Every line break in the
/// pasted text becomes one space, because the input's editor holds one
/// line: a paste from a multi-line source pastes its text in one line. A carriage return
/// and line feed together count as one break, so a Windows paste gains no
/// extra space where its text had one break.
fn paste_from_clipboard(chrome: &InputChrome) {
    if let Some(text) = smudge_core::clipboard::with(|slot| slot.get()).flatten() {
        chrome.editor.paste(&text);
        chrome.touched();
    }
}
