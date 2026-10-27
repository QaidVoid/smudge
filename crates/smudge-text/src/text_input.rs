//! The text input: one line of editable text bound to a signal.
//!
//! A [`TextInput`] drives an [`Editor`], paints the selection, the text and
//! a placeholder, and keeps a caret node that blinks on its own. The caret
//! is a separate node so that a blink, which changes two pixels' worth of
//! caret, damages the caret's rectangle alone instead of the whole input.
//!
//! Create one with [`text_input`] and shape it with the [`TextInputExt`]
//! builder methods. Editing is by grapheme, and with control or alt by word;
//! a mouse press places the caret, a drag selects and a double click selects
//! a word. Every edit writes the bound signal, and an outside write to the
//! signal replaces the text and moves the caret to its end. Undo and redo
//! walk back and forth through the edits, a typed word at a time. While an
//! input method composes, the preedit is drawn underlined and reaches the
//! value only when it is committed, and the platform's command modifier
//! with C, X and V copies, cuts and pastes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use parley::Generation;
use smudge_core::clipboard;
use smudge_core::element::{Element, Value};
use smudge_core::geometry::{Color, Corners, Point, Rect, Size};
use smudge_core::input::{Handled, Key, KeyEvent, Modifiers};
use smudge_core::paint::GlyphRun;
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, Semantics};
use smudge_core::theme::ColorToken;
use smudge_core::timer::{self, TimerHandle};
use smudge_core::tree::Dirty;
use smudge_core::widget::{AvailableSpace, MeasureInput, PaintCx, Widget};
use web_time::Instant;

use crate::input::Editor;
use crate::layout::TextLayout;
use crate::style::{TextStyle, current_text_style};

/// How long the caret stays visible or hidden in one blink.
const BLINK_PERIOD: Duration = Duration::from_millis(530);

/// How long after a click a second click still counts as a double click.
const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(400);

/// How close to the first click a double click has to land, in logical
/// pixels.
const DOUBLE_CLICK_DISTANCE: f32 = 4.0;

/// The width of the caret, in logical pixels.
const CARET_WIDTH: f32 = 1.0;

/// How opaque the selection is drawn, out of 255.
const SELECTION_ALPHA: u8 = 96;

/// What the caret node paints, placed by the input.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CaretPaint {
    /// The caret's rectangle in window coordinates.
    rect: Rect,
    /// The color the caret draws with.
    color: Color,
}

/// The state one input shares between its widget, its handlers and its caret
/// node.
struct InputChrome {
    /// The editor the input drives.
    editor: Editor,
    /// Bumped whenever the editor's state changed, so the input repaints and
    /// reports a new caret even when the text did not change.
    edits: RwSignal<u64>,
    /// What the caret node paints, placed by an edit before the frame
    /// paints and again by the input's paint when its box changed.
    caret: RwSignal<Option<CaretPaint>>,
    /// The blink phase: `true` while the caret shows.
    caret_on: RwSignal<bool>,
    /// Whether the input holds the focus, which is when a caret shows at all.
    caret_shown: RwSignal<bool>,
    /// The input's content box and the color its caret drew with, as the
    /// last paint left them, or `None` before the first paint.
    ///
    /// The pointer handlers read the box to turn window coordinates into the
    /// editor's, and an edit places the caret against it before the frame
    /// paints, so the caret moves in the same frame as the text.
    painted: Cell<Option<(Rect, Color)>>,
    /// How far the text is scrolled left, in logical pixels.
    scroll_x: Cell<f32>,
    /// Whether the pointer is held down on the input, which turns moves into
    /// selection extensions.
    dragging: Cell<bool>,
    /// When and where the last click landed, for double click detection.
    last_click: Cell<Option<(Instant, Point)>>,
    /// The handle of the blink timer, present only while focused.
    blink: RefCell<Option<TimerHandle>>,
}

impl InputChrome {
    /// Restarts the blink after an edit: the caret shows at once and the
    /// blink phase starts over.
    ///
    /// The edit counter rises so the next frame places the caret and repaints
    /// the input together.
    fn touched(&self) {
        self.caret_on.set(true);
        let mut blink = self.blink.borrow_mut();
        if blink.is_some() {
            let on = self.caret_on.clone();
            *blink = Some(blink_timer(on));
        }
        drop(blink);
        self.edits.update(|edits| *edits += 1);
    }

    /// Starts the blink timer, when none runs yet.
    fn start_blink(&self) {
        let mut blink = self.blink.borrow_mut();
        if blink.is_some() {
            return;
        }
        *blink = Some(blink_timer(self.caret_on.clone()));
    }

    /// Stops the blink timer.
    fn stop_blink(&self) {
        *self.blink.borrow_mut() = None;
    }

    /// Scrolls the text so the caret stays inside `content`, reports what
    /// the caret node paints, and returns the scroll that was applied.
    ///
    /// The caret is nudged in only as far as it needs to be, and the scroll
    /// never passes the ends of the text. The report is written only when it
    /// changes, so an idle input asks for no frames.
    fn place(&self, content: Rect, color: Color) -> f32 {
        let caret = self.editor.cursor_geometry(CARET_WIDTH);
        let layout_width = self.editor.layout_size().width;
        // A caret at the very end of the text sits at the layout's right
        // edge, so the scroll has to reach one caret width past the text
        // for that caret to fit inside the field.
        let max_scroll = (layout_width + CARET_WIDTH - content.width()).max(0.0);
        let mut scroll = self.scroll_x.get();
        if let Some(caret) = caret {
            if caret.x0 + CARET_WIDTH > scroll + content.width() {
                scroll = caret.x0 + CARET_WIDTH - content.width();
            }
            if caret.x0 < scroll {
                scroll = caret.x0;
            }
        }
        let scroll = scroll.clamp(0.0, max_scroll);
        self.scroll_x.set(scroll);

        let paint = caret.map(|caret| CaretPaint {
            rect: Rect::new(
                content.x0 + caret.x0 - scroll,
                content.y0 + caret.y0,
                content.x0 + caret.x0 - scroll + CARET_WIDTH,
                content.y0 + caret.y1,
            ),
            color,
        });
        if self.caret.read(|current| *current) != paint {
            self.caret.set(paint);
        }
        scroll
    }

    /// Places the caret against the last painted content box, for an edit
    /// that lands before the frame paints.
    ///
    /// Before the first paint there is nothing to place against, and the
    /// first paint places the caret itself.
    fn place_before_paint(&self) {
        if let Some((content, color)) = self.painted.get() {
            self.place(content, color);
        }
    }
}

/// Builds the timer that toggles the blink phase.
fn blink_timer(on: RwSignal<bool>) -> TimerHandle {
    timer::every(BLINK_PERIOD, move || {
        on.update(|shown| *shown = !*shown);
    })
}

/// A panel that paints the caret of a focused text input.
///
/// The input places the caret and reports whether it is shown, and the
/// caret's bindings pick the values up. A blink writes one signal, repaints
/// one small node, and damages its rectangle alone.
struct Caret {
    /// What to paint, or `None` before the input has painted.
    paint: Option<CaretPaint>,
    /// Whether the caret shows at all: focused and in the visible half of a
    /// blink.
    shown: bool,
}

impl Widget for Caret {
    fn paint(&self, _cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        if let (true, Some(paint)) = (self.shown, self.paint) {
            out.push(PaintOp::FillRect {
                rect: paint.rect,
                radius: Corners::ZERO,
                color: paint.color,
            });
        }
    }
}

/// Writes what the caret paints into the caret node.
fn apply_caret_paint(caret: &mut Caret, paint: Option<CaretPaint>) -> Dirty {
    if caret.paint == paint {
        return Dirty::Clean;
    }
    caret.paint = paint;
    Dirty::Paint
}

/// Writes whether the caret shows into the caret node.
fn apply_caret_shown(caret: &mut Caret, shown: bool) -> Dirty {
    if caret.shown == shown {
        return Dirty::Clean;
    }
    caret.shown = shown;
    Dirty::Paint
}

/// Builds the caret node of one input.
///
/// The node is absolutely positioned and measures nothing: its widget paints
/// the caret from the placed state, so moving the caret never runs the
/// layout engine.
fn caret_node(chrome: &Rc<InputChrome>) -> Element<Caret> {
    let paint_chrome = Rc::clone(chrome);
    let shown_chrome = Rc::clone(chrome);
    Element::new(Caret {
        paint: None,
        shown: false,
    })
    .absolute()
    .bind(move || paint_chrome.caret.get(), apply_caret_paint)
    .bind(
        move || shown_chrome.caret_on.get() && shown_chrome.caret_shown.get(),
        apply_caret_shown,
    )
}

/// One line of editable text.
///
/// Build one with [`text_input`]. The input keeps the editor and the glyph
/// runs of its current text, and rebuilds the runs only when the editor laid
/// its text out again, the node moved or the resolved text color changed.
pub struct TextInput {
    /// The state shared with the handlers and the caret node.
    chrome: Rc<InputChrome>,
    /// The text shown while the editor is empty.
    placeholder: String,
    /// The text style the editor lays out and the input paints with,
    /// resolved from the ambient style at mount.
    style: TextStyle,
    /// The layout of the placeholder, or `None` before it is needed and
    /// after every change that invalidates it.
    placeholder_layout: RefCell<Option<TextLayout>>,
    /// The glyph runs of the placeholder, with the origin and the resolved
    /// color they were built at.
    placeholder_runs: RefCell<Option<CachedRuns>>,
    /// The glyph runs of the editor's text, with everything they were built
    /// from.
    runs: RefCell<Option<TextRuns>>,
}

/// The glyph runs of one editor layout, with what they were built from.
struct TextRuns {
    /// The origin the glyph positions are absolute to.
    origin: Point,
    /// The color every run was built with.
    color: Color,
    /// How far the text was scrolled when the runs were built.
    scroll_x: f32,
    /// The editor generation the runs were built from.
    generation: Generation,
    /// The runs, shared by the paint operations that show them.
    runs: Vec<Rc<GlyphRun>>,
}

/// The glyph runs of the placeholder, with the origin and the resolved
/// color they were built at.
struct CachedRuns {
    /// The origin the glyph positions are absolute to.
    origin: Point,
    /// The color every run was built with.
    color: Color,
    /// The runs, shared by the paint operations that show them.
    runs: Vec<Rc<GlyphRun>>,
}

impl TextInput {
    /// Creates an input with an empty placeholder and the default style
    /// until the mount reads the ambient one.
    fn new(chrome: Rc<InputChrome>) -> TextInput {
        TextInput {
            chrome,
            placeholder: String::new(),
            style: TextStyle::default(),
            placeholder_layout: RefCell::new(None),
            placeholder_runs: RefCell::new(None),
            runs: RefCell::new(None),
        }
    }

    /// Drops everything that depends on the text style.
    fn invalidate_style(&mut self) {
        *self.placeholder_layout.borrow_mut() = None;
        *self.placeholder_runs.borrow_mut() = None;
        *self.runs.borrow_mut() = None;
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
        let label = (!self.placeholder.is_empty()).then(|| self.placeholder.clone());
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
    /// or when the glyph run caches are already borrowed on this thread,
    /// which the paint pass cannot do.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let color = self.text_color(cx);
        self.chrome.painted.set(Some((cx.content, color)));
        let scroll = self.chrome.place(cx.content, color);
        let text_origin = Point {
            x: cx.content.x0 - scroll,
            y: cx.content.y0,
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
            && !self.placeholder.is_empty()
        {
            let muted = cx.theme.color(ColorToken::TEXT_MUTED);
            self.paint_placeholder(text_origin, muted, out);
        } else {
            self.paint_text(text_origin, color, scroll, out);
        }
    }

    /// Reports the area the input method anchors to while the input holds
    /// the focus.
    ///
    /// The area is the text the user is editing, which is the preedit while
    /// one is being composed and the selection otherwise.
    fn ime_area(&self) -> Option<Rect> {
        let (content, _) = self.chrome.painted.get()?;
        let area = self.chrome.editor.ime_area();
        let scroll = self.chrome.scroll_x.get();
        Some(Rect::new(
            content.x0 + area.x0 - scroll,
            content.y0 + area.y0,
            content.x0 + area.x1 - scroll,
            content.y0 + area.y1,
        ))
    }
}

impl TextInput {
    /// Appends the glyph runs of the editor's text, rebuilding them when the
    /// editor laid its text out again, the node moved or the color changed.
    fn paint_text(&self, origin: Point, color: Color, scroll_x: f32, out: &mut Vec<PaintOp>) {
        let generation = self.chrome.editor.generation();
        let mut cache = self.runs.borrow_mut();
        let stale = match &*cache {
            Some(cached) => {
                cached.origin != origin
                    || cached.color != color
                    || cached.scroll_x != scroll_x
                    || cached.generation != generation
            }
            None => true,
        };
        if stale {
            let runs = self
                .chrome
                .editor
                .glyph_runs_at(origin, color)
                .into_iter()
                .map(Rc::new)
                .collect();
            *cache = Some(TextRuns {
                origin,
                color,
                scroll_x,
                generation,
                runs,
            });
        }
        if let Some(cached) = cache.as_ref() {
            out.extend(cached.runs.iter().cloned().map(PaintOp::Glyphs));
        }
    }

    /// Appends the glyph runs of the placeholder, which shows while the text
    /// is empty.
    fn paint_placeholder(&self, origin: Point, color: Color, out: &mut Vec<PaintOp>) {
        if self.placeholder_layout.borrow().is_none() {
            *self.placeholder_layout.borrow_mut() =
                Some(TextLayout::new(&self.placeholder, &self.style, None));
        }
        let mut cache = self.placeholder_runs.borrow_mut();
        let stale = match &*cache {
            Some(cached) => cached.origin != origin || cached.color != color,
            None => true,
        };
        if stale {
            let runs = self
                .placeholder_layout
                .borrow()
                .as_ref()
                .expect("the layout was just built or is cached")
                .glyph_runs_at(origin, color)
                .into_iter()
                .map(Rc::new)
                .collect();
            *cache = Some(CachedRuns {
                origin,
                color,
                runs,
            });
        }
        if let Some(cached) = cache.as_ref() {
            out.extend(cached.runs.iter().cloned().map(PaintOp::Glyphs));
        }
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
pub fn text_input(value: RwSignal<String>) -> Element<TextInput> {
    let chrome = Rc::new(InputChrome {
        editor: Editor::new(value.clone()),
        edits: RwSignal::new(0),
        caret: RwSignal::new(None),
        caret_on: RwSignal::new(true),
        caret_shown: RwSignal::new(false),
        painted: Cell::new(None),
        scroll_x: Cell::new(0.0),
        dragging: Cell::new(false),
        last_click: Cell::new(None),
        blink: RefCell::new(None),
    });

    let key_chrome = Rc::clone(&chrome);
    let press_chrome = Rc::clone(&chrome);
    let move_chrome = Rc::clone(&chrome);
    let up_chrome = Rc::clone(&chrome);
    let focus_chrome = Rc::clone(&chrome);
    let commit_chrome = Rc::clone(&chrome);
    let preedit_chrome = Rc::clone(&chrome);
    let edits_chrome = Rc::clone(&chrome);
    let caret_chrome = Rc::clone(&chrome);

    Element::new(TextInput::new(Rc::clone(&chrome)))
        .clip(true)
        .bind(current_text_style, apply_ambient_style)
        .bind(move || value.get(), apply_value)
        .bind(move || edits_chrome.edits.get(), apply_edits)
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

impl TextInputExt for Element<TextInput> {
    fn placeholder(self, text: impl Into<Value<String>>) -> Self {
        self.bind(text, set_placeholder)
    }
}

/// Applies the ambient text style to the editor and the placeholder.
fn apply_ambient_style(input: &mut TextInput, ambient: TextStyle) -> Dirty {
    if input.style == ambient {
        return Dirty::Clean;
    }
    input.style = ambient;
    input.chrome.editor.set_style(&input.style);
    input.invalidate_style();
    Dirty::Layout
}

/// Applies an outside write to the bound signal.
///
/// The editor compares the text first, so the write that an edit itself made
/// lands as nothing and only a real replacement moves the caret.
fn apply_value(input: &mut TextInput, value: String) -> Dirty {
    if input.chrome.editor.has_text(&value) {
        return Dirty::Clean;
    }
    input.chrome.editor.set_text(&value);
    input.chrome.place_before_paint();
    Dirty::Paint
}

/// Applies an editor state change the handlers made, and places the caret
/// so it moves in the same frame as the text.
fn apply_edits(input: &mut TextInput, _edits: u64) -> Dirty {
    input.chrome.place_before_paint();
    Dirty::Paint
}

/// Writes the placeholder into the input.
fn set_placeholder(input: &mut TextInput, text: String) -> Dirty {
    if input.placeholder == text {
        return Dirty::Clean;
    }
    input.placeholder = text;
    *input.placeholder_layout.borrow_mut() = None;
    *input.placeholder_runs.borrow_mut() = None;
    Dirty::Paint
}

/// Edits the pointer press makes: place the caret, or select a word on a
/// double click, and start a drag.
fn handle_press(chrome: &InputChrome, position: Point) {
    let now = timer::now();
    let double_click = chrome
        .last_click
        .get()
        .is_some_and(|(last_at, last_at_point)| {
            now.duration_since(last_at) <= DOUBLE_CLICK_TIME
                && distance_squared(position, last_at_point)
                    <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
        });
    chrome.last_click.set(Some((now, position)));
    chrome.dragging.set(true);

    let (x, y) = layout_position(chrome, position);
    if double_click {
        chrome.editor.select_word_at_point(x, y);
    } else {
        chrome.editor.move_to_point(x, y);
    }
    chrome.touched();
}

/// Extends the selection while the pointer is held down and moves.
fn handle_move(chrome: &InputChrome, position: Point) {
    if !chrome.dragging.get() {
        return;
    }
    // A press that turned into a drag is not the first half of a double
    // click, while the jitter of a steady click still is.
    if chrome.last_click.get().is_some_and(|(_, pressed_at)| {
        distance_squared(position, pressed_at) > DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
    }) {
        chrome.last_click.set(None);
    }
    let (x, y) = layout_position(chrome, position);
    chrome.editor.extend_to_point(x, y);
    chrome.touched();
}

/// Ends the drag when the button comes up.
fn handle_release(chrome: &InputChrome, _position: Point) {
    chrome.dragging.set(false);
}

/// Places the caret at `position`, in the editor's own coordinates.
fn layout_position(chrome: &InputChrome, position: Point) -> (f32, f32) {
    let origin = chrome
        .painted
        .get()
        .map(|(content, _)| content.origin())
        .unwrap_or_default();
    (
        position.x - origin.x + chrome.scroll_x.get(),
        position.y - origin.y,
    )
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

/// Returns whether `modifiers` hold the platform's command key as a
/// shortcut, which excludes AltGr typing a character.
fn is_shortcut(modifiers: Modifiers) -> bool {
    modifiers.command() && !types_text(modifiers)
}

/// Returns whether a character key pressed with `modifiers` types its
/// character rather than running a shortcut.
///
/// Option types characters on macOS, and AltGr, which arrives as Control
/// and Alt together, types them elsewhere. Any other Control, Alt or Meta
/// combination is a shortcut and bubbles to the ancestors.
fn types_text(modifiers: Modifiers) -> bool {
    #[cfg(target_os = "macos")]
    {
        !modifiers.meta && !modifiers.control
    }
    #[cfg(not(target_os = "macos"))]
    {
        let alt_gr = modifiers.control && modifiers.alt;
        !modifiers.meta && (alt_gr || (!modifiers.control && !modifiers.alt))
    }
}

/// The two directions a caret can move in.
#[derive(Copy, Clone)]
enum Direction {
    /// Towards the start of the line.
    Left,
    /// Towards the end of the line.
    Right,
}

/// Moves or extends for one arrow key press.
fn apply_move(chrome: &InputChrome, extend: bool, by_word: bool, direction: Direction) {
    let editor = &chrome.editor;
    match (extend, by_word, direction) {
        (false, false, Direction::Left) => editor.move_left(),
        (false, false, Direction::Right) => editor.move_right(),
        (false, true, Direction::Left) => editor.move_word_left(),
        (false, true, Direction::Right) => editor.move_word_right(),
        (true, false, Direction::Left) => editor.extend_left(),
        (true, false, Direction::Right) => editor.extend_right(),
        (true, true, Direction::Left) => editor.extend_word_left(),
        (true, true, Direction::Right) => editor.extend_word_right(),
    }
    chrome.touched();
}

/// Copies the selected text to the clipboard of the running app.
///
/// An app without a clipboard, or an input with a collapsed caret, copies
/// nothing.
fn copy_selection(chrome: &InputChrome) {
    if let Some(text) = chrome.editor.selected_text() {
        clipboard::with(|slot| slot.set(text));
    }
}

/// Copies the selected text to the clipboard and deletes it from the editor.
///
/// The deletion happens only when the copy found a clipboard to land on, so
/// an app without one keeps the text it could not hand off. Returns whether
/// anything was cut.
fn cut_selection(chrome: &InputChrome) -> bool {
    let Some(text) = chrome.editor.selected_text() else {
        return false;
    };
    let mut copied = false;
    clipboard::with(|slot| {
        slot.set(text);
        copied = true;
    });
    if copied {
        chrome.editor.delete_selection();
    }
    copied
}

/// Inserts the text on the clipboard of the running app.
///
/// An app without a clipboard pastes nothing.
fn paste_from_clipboard(chrome: &InputChrome) {
    if let Some(text) = clipboard::with(|slot| slot.get()).flatten() {
        chrome.editor.paste(&text);
        chrome.touched();
    }
}

/// Starts or stops the blink and the caret as the focus moves.
fn handle_focus(chrome: &InputChrome, focused: bool) {
    if !focused {
        // The platform drops its input context when the field loses the
        // focus, so the composition dies with it instead of outliving it.
        chrome.editor.compose("", None);
    }
    chrome.caret_shown.set(focused);
    if focused {
        chrome.caret_on.set(true);
        chrome.start_blink();
    } else {
        chrome.stop_blink();
    }
    chrome.edits.update(|edits| *edits += 1);
}

/// Inserts the text an input method committed.
///
/// A commit can arrive with no key event, so this is the only path some
/// input methods ever take. It can also arrive while the preedit is still
/// showing, and then it takes the preedit's place.
fn handle_commit(chrome: &InputChrome, text: &str) {
    if text.is_empty() {
        return;
    }
    chrome.editor.commit(text);
    chrome.touched();
}

/// Shows, updates or clears the text an input method is composing.
///
/// The preedit is drawn but never written to the bound signal, so a
/// canceled composition leaves the value as it was. An empty text is the
/// cancel, and it arrives when the composition is canceled, committed by a
/// route that sent its own commit, or dropped with the focus.
fn handle_preedit(chrome: &InputChrome, text: &str, cursor: Option<(usize, usize)>) {
    chrome.editor.compose(text, cursor);
    chrome.touched();
}

/// Returns the squared distance between two points.
fn distance_squared(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}
