//! State shared by the editing widgets.
//!
//! [`TextInput`](crate::TextInput) and [`TextArea`](crate::TextArea) drive
//! an [`Editor`] the same way: both paint the selection, the text and a
//! placeholder, keep a caret node that blinks on its own, and turn presses,
//! drags, double clicks, input-method events and the clipboard into edits.
//! This module holds what the two share. Each widget owns its key handling,
//! its paste rule and its measurement.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use smudge_core::clipboard;
use smudge_core::element::Element;
use smudge_core::geometry::{Color, Corners, Point, Rect};
use smudge_core::input::{Handled, Modifiers};
use smudge_core::paint::GlyphRun;
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::timer::{self, TimerHandle};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Widget};
use web_time::Instant;

use crate::input::Editor;
use crate::layout::TextLayout;
use crate::style::TextStyle;

/// How long the caret stays visible or hidden in one blink.
pub(crate) const BLINK_PERIOD: Duration = Duration::from_millis(530);

/// How long after a click a second click still counts as a double click.
pub(crate) const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(400);

/// How close to the first click a double click has to land, in logical
/// pixels.
pub(crate) const DOUBLE_CLICK_DISTANCE: f32 = 4.0;

/// The width of the caret, in logical pixels.
pub(crate) const CARET_WIDTH: f32 = 1.0;

/// How opaque the selection is drawn, out of 255.
pub(crate) const SELECTION_ALPHA: u8 = 96;

/// What the caret node paints, placed by the editing widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CaretPaint {
    /// The caret's rectangle in paint coordinates.
    pub(crate) rect: Rect,
    /// The color the caret draws with.
    pub(crate) color: Color,
}

/// The state one editing widget shares between its widget, its handlers and
/// its caret node.
pub(crate) struct InputChrome {
    /// The editor the widget drives.
    pub(crate) editor: Editor,
    /// Bumped whenever the editor's state changed, so the widget repaints
    /// and reports a new caret even when the text did not change.
    pub(crate) edits: RwSignal<u64>,
    /// What the caret node paints, placed by an edit before the frame
    /// paints and again by the widget's paint when its box changed.
    pub(crate) caret: RwSignal<Option<CaretPaint>>,
    /// The blink phase: `true` while the caret shows.
    pub(crate) caret_on: RwSignal<bool>,
    /// Whether the widget holds the focus, which is when its caret and its
    /// selection show at all.
    pub(crate) focused: RwSignal<bool>,
    /// The widget's content box in paint coordinates, the color its caret
    /// drew with and the box's origin relative to the widget's rectangle, as
    /// the last paint left them, or `None` before the first paint.
    ///
    /// The pointer handlers read the origin to turn positions relative to
    /// the widget into the editor's coordinates, and an edit places the
    /// caret against the box before the frame paints, so the caret moves in
    /// the same frame as the text.
    pub(crate) painted: Cell<Option<(Rect, Color, Point)>>,
    /// How far the text is scrolled left, in logical pixels.
    ///
    /// A wrapped area never scrolls sideways except past an unbreakable
    /// word.
    pub(crate) scroll_x: Cell<f32>,
    /// How far the text is scrolled up, in logical pixels, which stays at
    /// zero in a one-line input.
    pub(crate) scroll_y: Cell<f32>,
    /// Whether the text scrolls up and down, which only a multi-line area
    /// does: a one-line input keeps its first line at the top even while a
    /// preedit holds a line break.
    pub(crate) scrolls_vertically: bool,
    /// The height of one line of the text, as the last paint measured it.
    /// Zero before the first paint.
    pub(crate) line_height: Cell<f32>,
    /// Whether the pointer is held down on the widget, which turns moves
    /// into selection extensions.
    pub(crate) dragging: Cell<bool>,
    /// When and where the last click landed, for double click detection.
    pub(crate) last_click: Cell<Option<(Instant, Point)>>,
    /// The handle of the blink timer, present only while focused.
    pub(crate) blink: RefCell<Option<TimerHandle>>,
    /// Whether placing the caret scrolls the text to keep it visible. A
    /// wheel turns this off, so the paint after it keeps the scroll the
    /// wheel chose, and the next caret change turns it back on.
    pub(crate) follow_caret: Cell<bool>,
    /// The selection rectangles the last paint built, kept so the next
    /// paint fills the same buffer.
    pub(crate) selection: RefCell<Vec<Rect>>,
}

impl InputChrome {
    /// Creates the state of a widget that edits `value`, with the caret
    /// hidden until the widget takes the focus.
    ///
    /// A multi-line widget scrolls up and down and its editor keeps line
    /// breaks; a one-line widget does neither, and its editor turns every
    /// line break into a space.
    pub(crate) fn new(value: RwSignal<String>, multi_line: bool) -> InputChrome {
        InputChrome {
            editor: if multi_line {
                Editor::new(value)
            } else {
                Editor::single_line(value)
            },
            edits: RwSignal::new(0),
            caret: RwSignal::new(None),
            caret_on: RwSignal::new(true),
            focused: RwSignal::new(false),
            painted: Cell::new(None),
            scroll_x: Cell::new(0.0),
            scroll_y: Cell::new(0.0),
            scrolls_vertically: multi_line,
            line_height: Cell::new(0.0),
            dragging: Cell::new(false),
            last_click: Cell::new(None),
            blink: RefCell::new(None),
            follow_caret: Cell::new(true),
            selection: RefCell::new(Vec::new()),
        }
    }

    /// Restarts the blink after an edit: the caret shows at once and the
    /// blink phase starts over.
    ///
    /// The edit counter rises so the next frame places the caret and
    /// repaints the widget together.
    pub(crate) fn touched(&self) {
        self.follow_caret.set(true);
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
    pub(crate) fn start_blink(&self) {
        let mut blink = self.blink.borrow_mut();
        if blink.is_some() {
            return;
        }
        *blink = Some(blink_timer(self.caret_on.clone()));
    }

    /// Stops the blink timer.
    pub(crate) fn stop_blink(&self) {
        *self.blink.borrow_mut() = None;
    }

    /// Scrolls the text so the caret stays inside `content`, reports what
    /// the caret node paints, and returns the scroll that was applied.
    ///
    /// Each axis is nudged only as far as the caret needs, and never past
    /// the ends of the text. Text that fits leaves the range empty, so the
    /// scroll stays at zero. After a wheel, the scroll is only kept within
    /// the text until the caret changes again. The report is written only
    /// when it changes, so an idle field asks for no frames.
    pub(crate) fn place(&self, content: Rect, color: Color) -> (f32, f32) {
        let caret = self.editor.cursor_geometry(CARET_WIDTH);
        let follow = caret.filter(|_| self.follow_caret.get());
        let (max_scroll_x, max_scroll_y) = self.max_scroll(content);
        let mut scroll_x = self.scroll_x.get();
        if let Some(caret) = follow {
            if caret.x0 + CARET_WIDTH > scroll_x + content.width() {
                scroll_x = caret.x0 + CARET_WIDTH - content.width();
            }
            if caret.x0 < scroll_x {
                scroll_x = caret.x0;
            }
        }
        let scroll_x = scroll_x.clamp(0.0, max_scroll_x);
        self.scroll_x.set(scroll_x);

        let mut scroll_y = self.scroll_y.get();
        if let Some(caret) = follow {
            if caret.y1 > scroll_y + content.height() {
                scroll_y = caret.y1 - content.height();
            }
            if caret.y0 < scroll_y {
                scroll_y = caret.y0;
            }
        }
        let scroll_y = scroll_y.clamp(0.0, max_scroll_y);
        self.scroll_y.set(scroll_y);

        let paint = caret.map(|caret| CaretPaint {
            rect: Rect::new(
                content.x0 + caret.x0 - scroll_x,
                content.y0 + caret.y0 - scroll_y,
                content.x0 + caret.x0 - scroll_x + CARET_WIDTH,
                content.y0 + caret.y1 - scroll_y,
            ),
            color,
        });
        if self.caret.read(|current| *current) != paint {
            self.caret.set(paint);
        }
        (scroll_x, scroll_y)
    }

    /// Returns how far the text can scroll left and up inside `content`.
    ///
    /// Text that fits gives zero, and a one-line widget never scrolls up.
    fn max_scroll(&self, content: Rect) -> (f32, f32) {
        let layout = self.editor.layout_size();
        // A caret at the very end of the text sits at the layout's right
        // edge, so the horizontal scroll has to reach one caret width past
        // the text for that caret to fit inside the field.
        let x = (layout.width + CARET_WIDTH - content.width()).max(0.0);
        let y = if self.scrolls_vertically {
            (layout.height - content.height()).max(0.0)
        } else {
            0.0
        };
        (x, y)
    }

    /// Places the caret against the last painted content box, for an edit
    /// that lands before the frame paints.
    ///
    /// Before the first paint there is nothing to place against, and the
    /// first paint places the caret itself.
    pub(crate) fn place_before_paint(&self) {
        if let Some((content, color, _)) = self.painted.get() {
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

/// A panel that paints the caret of a focused editing widget.
///
/// The widget places the caret and reports whether it is shown, and the
/// caret's bindings pick the values up. A blink writes one signal, repaints
/// one small node, and damages its rectangle alone.
pub(crate) struct Caret {
    /// What to paint, or `None` before the widget has painted.
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

/// The key of the caret's placed state binding.
struct CaretPaintKey;

/// The key of the caret's blink state binding.
struct CaretShownKey;

/// Builds the caret node of one editing widget.
///
/// The node is absolutely positioned and measures nothing: its widget paints
/// the caret from the placed state, so moving the caret never runs the
/// layout engine.
pub(crate) fn caret_node(chrome: &Rc<InputChrome>) -> Element<Caret> {
    let paint_chrome = Rc::clone(chrome);
    let shown_chrome = Rc::clone(chrome);
    Element::new(Caret {
        paint: None,
        shown: false,
    })
    .absolute()
    .bind(
        CaretPaintKey,
        move || paint_chrome.caret.get(),
        apply_caret_paint,
    )
    .bind(
        CaretShownKey,
        move || shown_chrome.caret_on.get() && shown_chrome.focused.get(),
        apply_caret_shown,
    )
}

/// Edits the pointer press makes: place the caret, or select a word on a
/// double click, and start a drag.
pub(crate) fn handle_press(chrome: &InputChrome, position: Point) {
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
pub(crate) fn handle_move(chrome: &InputChrome, position: Point) {
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
pub(crate) fn handle_release(chrome: &InputChrome, _position: Point) {
    chrome.dragging.set(false);
}

/// Turns `position`, relative to the widget's rectangle, into the editor's
/// own coordinates.
pub(crate) fn layout_position(chrome: &InputChrome, position: Point) -> (f32, f32) {
    let origin = chrome
        .painted
        .get()
        .map(|(_, _, inset)| inset)
        .unwrap_or_default();
    (
        position.x - origin.x + chrome.scroll_x.get(),
        position.y - origin.y + chrome.scroll_y.get(),
    )
}

/// Scrolls the text by a wheel's `delta`, in logical pixels, leaving the
/// caret where it is, and returns whether the text moved.
///
/// The scroll stays within the text, and the caret scrolls out of view with
/// the text it sits in until the next caret change brings it back. A wheel
/// that cannot move the text, because it fits or already sits at its limit
/// in the wheel's direction, is not handled, so it reaches the ancestors.
pub(crate) fn handle_wheel(chrome: &InputChrome, delta: Point) -> Handled {
    let Some((content, _, _)) = chrome.painted.get() else {
        return Handled::No;
    };
    let (max_x, max_y) = chrome.max_scroll(content);
    let (old_x, old_y) = (chrome.scroll_x.get(), chrome.scroll_y.get());
    let step = |delta: f32| if delta.is_finite() { delta } else { 0.0 };
    let x = (old_x + step(delta.x)).clamp(0.0, max_x);
    let y = (old_y + step(delta.y)).clamp(0.0, max_y);
    if x == old_x && y == old_y {
        return Handled::No;
    }
    chrome.follow_caret.set(false);
    chrome.scroll_x.set(x);
    chrome.scroll_y.set(y);
    chrome.edits.update(|edits| *edits += 1);
    Handled::Yes
}

/// Starts or stops the blink and the caret as the focus moves.
pub(crate) fn handle_focus(chrome: &InputChrome, focused: bool) {
    if focused {
        chrome.follow_caret.set(true);
    } else {
        // The platform drops its input context when the field loses the
        // focus, so the composition dies with it instead of outliving it.
        chrome.editor.compose("", None);
    }
    chrome.focused.set(focused);
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
pub(crate) fn handle_commit(chrome: &InputChrome, text: &str) {
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
pub(crate) fn handle_preedit(chrome: &InputChrome, text: &str, cursor: Option<(usize, usize)>) {
    chrome.editor.compose(text, cursor);
    chrome.touched();
}

/// Returns whether `modifiers` hold the platform's command key as a
/// shortcut, which excludes AltGr typing a character.
pub(crate) fn is_shortcut(modifiers: Modifiers) -> bool {
    modifiers.command() && !types_text(modifiers)
}

/// Returns whether a character key pressed with `modifiers` types its
/// character rather than running a shortcut.
///
/// Option types characters on macOS, and AltGr, which arrives as Control
/// and Alt together, types them elsewhere. Any other Control, Alt or Meta
/// combination is a shortcut and bubbles to the ancestors.
pub(crate) fn types_text(modifiers: Modifiers) -> bool {
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
pub(crate) enum Direction {
    /// Towards the start of the line.
    Left,
    /// Towards the end of the line.
    Right,
}

/// Moves or extends for one arrow key press.
pub(crate) fn apply_move(chrome: &InputChrome, extend: bool, by_word: bool, direction: Direction) {
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
/// An app without a clipboard, or a field with a collapsed caret, copies
/// nothing.
pub(crate) fn copy_selection(chrome: &InputChrome) {
    if let Some(text) = chrome.editor.selected_text() {
        clipboard::with(|slot| slot.set(text));
    }
}

/// Copies the selected text to the clipboard and deletes it from the editor.
///
/// The deletion happens only when the copy found a clipboard to land on, so
/// an app without one keeps the text it could not hand off. Returns whether
/// anything was cut.
pub(crate) fn cut_selection(chrome: &InputChrome) -> bool {
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

/// Applies an outside write to the bound signal of an editing widget.
///
/// The editor compares the text first, so the write that an edit itself
/// made lands as nothing and only a real replacement moves the caret.
pub(crate) fn apply_outside_write(chrome: &InputChrome, value: &str) -> Dirty {
    if chrome.editor.has_text(value) {
        return Dirty::Clean;
    }
    chrome.editor.set_text(value);
    chrome.follow_caret.set(true);
    chrome.place_before_paint();
    Dirty::Paint
}

/// Reads the bound signal for an editing widget's binding, tracking it, and
/// returns its text only when it differs from the editor's.
///
/// An edit writes the signal too, and the comparison lets that write land
/// without copying the text: only an outside write hands a copy to
/// [`apply_outside_write`].
pub(crate) fn outside_write(chrome: &InputChrome, value: &RwSignal<String>) -> Option<String> {
    value.with(|text| (!chrome.editor.has_text(text)).then(|| text.clone()))
}

/// Applies an editor state change the handlers made, and places the caret
/// so it moves in the same frame as the text.
pub(crate) fn apply_edit_count(chrome: &InputChrome) -> Dirty {
    chrome.place_before_paint();
    Dirty::Paint
}

/// Returns the squared distance between two points.
pub(crate) fn distance_squared(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

/// Appends the selection of the editor's text for the lines that meet
/// `visible`, in text coordinates, as fills in `color` placed at `origin`.
///
/// Only a widget that holds the focus shows its selection, so of several
/// fields only the one being edited shows one. The editor keeps the
/// selection while the widget is unfocused, and it shows again when the
/// focus comes back. The focus change repaints the widget through the edit
/// counter [`handle_focus`] bumps.
///
/// The rectangles go through the chrome's kept buffer, so a steady paint
/// allocates nothing.
pub(crate) fn paint_selection(
    chrome: &InputChrome,
    origin: Point,
    visible: (f32, f32),
    color: Color,
    out: &mut Vec<PaintOp>,
) {
    if !chrome.focused.read(|focused| *focused) {
        return;
    }
    let mut rects = chrome.selection.borrow_mut();
    chrome.editor.selection_rects(visible, &mut rects);
    out.extend(rects.iter().map(|area| PaintOp::FillRect {
        rect: Rect::new(
            origin.x + area.x0,
            origin.y + area.y0,
            origin.x + area.x1,
            origin.y + area.y1,
        ),
        radius: Corners::ZERO,
        color,
    }));
}

/// The glyph runs of the editor's text, with what they were built from.
pub(crate) struct ParagraphCache {
    /// One entry per painted paragraph, in the order the last paint
    /// yielded them.
    paragraphs: Vec<PaintedParagraph>,
    /// The buffer the paint before last filled, which the next paint
    /// swaps in and fills again, so a steady frame allocates nothing.
    scratch: Vec<PaintedParagraph>,
    /// The origin the glyph positions are absolute to.
    origin: Point,
    /// The color every run was built with.
    color: Color,
}

/// The glyph runs of one paragraph of the editor.
struct PaintedParagraph {
    /// The paragraph's index in the editor.
    index: usize,
    /// The paragraph's offset from the top of the text, which an edit
    /// above changes without laying this paragraph out again.
    y: f32,
    /// The editor's build count when the paragraph's layout was built,
    /// which changes when the paragraph is laid out again.
    stamp: u64,
    /// The runs, shared by the cache entries that reuse them and by the
    /// paint operations that show them.
    runs: Rc<Vec<Rc<GlyphRun>>>,
}

/// Appends the glyph runs of the editor's text for the paragraphs whose
/// vertical range meets `visible`, in text coordinates, rebuilding only
/// what changed.
///
/// Paragraphs entirely outside the range paint nothing and cache nothing,
/// so a tall document paints its viewport alone. A paragraph's runs are
/// reused while they were built at the same origin and color, the
/// paragraph sits where it sat, and its layout was not built again.
pub(crate) fn paint_paragraphs(
    cache: &mut Option<ParagraphCache>,
    editor: &Editor,
    origin: Point,
    color: Color,
    visible: (f32, f32),
    out: &mut Vec<PaintOp>,
) {
    let stale = match cache {
        Some(cached) => cached.origin != origin || cached.color != color,
        None => true,
    };
    if stale {
        *cache = Some(ParagraphCache {
            paragraphs: Vec::new(),
            scratch: Vec::new(),
            origin,
            color,
        });
    }
    let cache = cache.as_mut().expect("the cache is present");
    std::mem::swap(&mut cache.paragraphs, &mut cache.scratch);
    cache.paragraphs.clear();
    let previous = &cache.scratch;
    let painted = &mut cache.paragraphs;
    let mut index = 0usize;
    editor.paragraphs(|paragraph| {
        let paragraph_index = index;
        index += 1;
        let offset = paragraph.offset();
        let top = offset.y;
        let bottom = top + paragraph.size().height;
        if bottom < visible.0 || top > visible.1 {
            return;
        }
        let stamp = paragraph.stamp();
        let cached = previous
            .iter()
            .find(|entry| entry.index == paragraph_index && entry.y == top && entry.stamp == stamp)
            .map(|entry| entry.runs.clone());
        let runs = cached.unwrap_or_else(|| {
            let absolute = Point {
                x: origin.x + offset.x,
                y: origin.y + offset.y,
            };
            Rc::new(
                paragraph
                    .glyph_runs_at(absolute, color)
                    .into_iter()
                    .map(Rc::new)
                    .collect::<Vec<_>>(),
            )
        });
        painted.push(PaintedParagraph {
            index: paragraph_index,
            y: top,
            stamp,
            runs,
        });
    });
    for paragraph in &cache.paragraphs {
        out.extend(paragraph.runs.iter().cloned().map(PaintOp::Glyphs));
    }
}

/// The text a field shows while it holds none.
pub(crate) struct Placeholder {
    /// The text itself.
    text: String,
    /// The layout of the text, or `None` before it is needed and after
    /// every change that invalidates it.
    layout: Option<TextLayout>,
    /// The glyph runs, with the origin and the resolved color they were
    /// built at.
    runs: Option<CachedRuns>,
}

/// The glyph runs of one piece of text, with the origin and the resolved
/// color they were built at.
struct CachedRuns {
    /// The origin the glyph positions are absolute to.
    origin: Point,
    /// The color every run was built with.
    color: Color,
    /// The runs, shared by the paint operations that show them.
    runs: Vec<Rc<GlyphRun>>,
}

impl Placeholder {
    /// Creates an empty placeholder.
    pub(crate) fn new() -> Placeholder {
        Placeholder {
            text: String::new(),
            layout: None,
            runs: None,
        }
    }

    /// Returns the text shown while the field is empty.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text and drops what was built from the old one.
    pub(crate) fn set_text(&mut self, text: String) {
        if self.text == text {
            return;
        }
        self.text = text;
        self.layout = None;
        self.runs = None;
    }

    /// Drops everything that depends on the text style.
    pub(crate) fn invalidate_style(&mut self) {
        self.layout = None;
        self.runs = None;
    }

    /// Appends the placeholder's glyph runs, laid out once and rebuilt when
    /// the origin or the color moved.
    pub(crate) fn paint(
        &mut self,
        style: &TextStyle,
        origin: Point,
        color: Color,
        out: &mut Vec<PaintOp>,
    ) {
        if self.layout.is_none() {
            self.layout = Some(TextLayout::new(&self.text, style, None));
        }
        let stale = match &self.runs {
            Some(cached) => cached.origin != origin || cached.color != color,
            None => true,
        };
        if stale {
            let runs = self
                .layout
                .as_ref()
                .expect("the layout was just built or is cached")
                .glyph_runs_at(origin, color)
                .into_iter()
                .map(Rc::new)
                .collect();
            self.runs = Some(CachedRuns {
                origin,
                color,
                runs,
            });
        }
        if let Some(cached) = &self.runs {
            out.extend(cached.runs.iter().cloned().map(PaintOp::Glyphs));
        }
    }
}
