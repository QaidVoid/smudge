//! The rich text widget: one wrapping paragraph of styled spans.
//!
//! A [`RichText`] lays its spans out as one block, each span in its own
//! weight, slant and family, and paints each span in its own color. Colors
//! stay out of the layout, so a change that touches only colors repaints the
//! paragraph without laying it out again, and so does a theme switch. A span
//! with a link paints in the theme's accent color unless it sets its own,
//! draws an underline, shows the pointing cursor, and runs its handler when
//! it is clicked.
//!
//! Create one with [`rich_text`] and set its block style with the
//! [`RichTextExt`] builder methods. At mount it reads the ambient text style,
//! like a [`label`](crate::label) does.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use parley::{FontFamily, FontWeight};
use smudge_core::element::{Element, Value};
use smudge_core::geometry::{Color, Corners, Point, Size};
use smudge_core::paint::{GlyphRun, PaintOp};
use smudge_core::semantics::{Role, Semantics};
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::tree::Dirty;
use smudge_core::widget::{AvailableSpace, MeasureInput, PaintCx, Widget};
use smudge_core::window::Cursor;

use crate::glyphs::rich_glyph_runs;
use crate::layout::{RichLayout, SpanFont};
use crate::style::{TextStyle, current_text_style};

/// One run of text in a [`rich_text`] paragraph, with the style it sets
/// over the paragraph's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    /// The text the span shows.
    pub text: String,
    /// What the span sets over the paragraph's style.
    pub style: SpanStyle,
}

impl Span {
    /// Creates a span of `text` with `style`.
    pub fn new(text: impl Into<String>, style: SpanStyle) -> Span {
        Span {
            text: text.into(),
            style,
        }
    }

    /// Creates a link of `text` whose click runs `handler`, and which keeps
    /// running the newest code of `handler` across hot patches.
    ///
    /// ```
    /// use smudge_text::Span;
    ///
    /// let guide = Span::link("guide", || println!("open the guide"));
    /// ```
    pub fn link(text: impl Into<String>, handler: impl Fn() + 'static) -> Span {
        Span {
            text: text.into(),
            style: SpanStyle {
                link: Some(Rc::new(smudge_core::hot::wrap(handler))),
                ..SpanStyle::default()
            },
        }
    }
}

/// What one [`Span`] sets over the style of its paragraph.
///
/// The default sets nothing, so the span reads like the rest of the
/// paragraph. Two styles compare equal when every field does and their links
/// are the same handler, compared by pointer.
#[derive(Clone, Default)]
pub struct SpanStyle {
    /// The weight, or `None` for the paragraph's.
    pub weight: Option<FontWeight>,
    /// Whether the span is set in the italic face of its family.
    pub italic: bool,
    /// Whether the span is set in the generic monospace family.
    pub monospace: bool,
    /// The color, or `None` for the paragraph's text color, or for the
    /// theme's accent color when the span is a link.
    pub color: Option<ColorValue>,
    /// The handler a click on the span runs, which makes the span a link.
    ///
    /// A handler set here directly keeps the code it was built with until
    /// the view builds the spans again. [`Span::link`] wraps its handler so
    /// it runs the newest code after every hot patch.
    pub link: Option<Rc<dyn Fn()>>,
}

impl PartialEq for SpanStyle {
    fn eq(&self, other: &Self) -> bool {
        self.weight == other.weight
            && self.italic == other.italic
            && self.monospace == other.monospace
            && self.color == other.color
            && match (&self.link, &other.link) {
                (Some(first), Some(second)) => Rc::ptr_eq(first, second),
                (None, None) => true,
                _ => false,
            }
    }
}

impl fmt::Debug for SpanStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpanStyle")
            .field("weight", &self.weight)
            .field("italic", &self.italic)
            .field("monospace", &self.monospace)
            .field("color", &self.color)
            .field("link", &self.link.is_some())
            .finish()
    }
}

/// The link handler of each span of a paragraph, which the paragraph shares
/// with its pointer handlers.
type Links = Rc<RefCell<Vec<Option<Rc<dyn Fn()>>>>>;

/// A wrapping paragraph of styled spans.
///
/// Build one with [`rich_text`]. Like a label, it keeps the layout of its
/// spans and rebuilds it only when their text or font style, its block
/// style, or the offered width changes, and keeps the glyph runs of that
/// layout until the layout, the node's position or a resolved color
/// changes.
pub struct RichText {
    /// The spans the paragraph shows.
    spans: Rc<[Span]>,
    /// The text of every span, joined.
    text: String,
    /// The byte range of each span in [`RichText::text`].
    ranges: Vec<Range<usize>>,
    /// The block style, resolved from the ambient style and the explicit
    /// fields at mount.
    style: TextStyle,
    /// The style fields an explicit write set, which the ambient style
    /// leaves alone.
    explicit: ExplicitStyle,
    /// The layout of the spans, shared with the pointer handlers that look
    /// up the link under the pointer. `None` before the first measure and
    /// after every change that invalidates it.
    layout: Rc<RefCell<Option<RichLayout>>>,
    /// The width the lines were last broken at, `None` for max content, or
    /// `None` inside `None` when nothing is laid out.
    broken_width: Option<Option<f32>>,
    /// The size of the text wrapped at every opportunity, or `None` until it
    /// is asked for.
    min_content: Option<Size>,
    /// The link handler of each span, shared with the pointer handlers.
    links: Links,
    /// The paint operations of the current layout, with the origin and the
    /// colors they were built at.
    painted: RefCell<Option<Painted>>,
}

/// The paint operations of one layout, with the origin and the resolved
/// span colors they were built at.
struct Painted {
    /// The origin the operations are absolute to.
    origin: Point,
    /// The color of each span.
    colors: Vec<Color>,
    /// The glyph runs and the link underlines.
    ops: Vec<PaintOp>,
}

/// What a change of spans asks of the paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SpanChange {
    /// Nothing that shows changed.
    Nothing,
    /// Only colors changed, so the paragraph repaints.
    Colors,
    /// Text or font style changed, so the paragraph lays out again.
    Layout,
}

/// Compares two span lists and answers what the change from `old` to `new`
/// asks of the paragraph.
///
/// The text, the weight, the slant, the family and whether a span is a link
/// shape the layout. A color, and which handler a link runs, do not.
pub(crate) fn span_change(old: &[Span], new: &[Span]) -> SpanChange {
    if old.len() != new.len() {
        return SpanChange::Layout;
    }
    let mut change = SpanChange::Nothing;
    for (old, new) in old.iter().zip(new) {
        if old.text != new.text || span_font(&old.style) != span_font(&new.style) {
            return SpanChange::Layout;
        }
        if old.style.color != new.style.color {
            change = SpanChange::Colors;
        }
    }
    change
}

/// The font style `style` sets, which is what the layout depends on.
fn span_font(style: &SpanStyle) -> SpanFont {
    SpanFont {
        weight: style.weight,
        italic: style.italic,
        monospace: style.monospace,
        underline: style.link.is_some(),
    }
}

impl RichText {
    /// Creates an empty paragraph that shares `layout` and `links` with its
    /// pointer handlers.
    fn new(layout: Rc<RefCell<Option<RichLayout>>>, links: Links) -> RichText {
        RichText {
            spans: Rc::from([]),
            text: String::new(),
            ranges: Vec::new(),
            style: TextStyle::default(),
            explicit: ExplicitStyle::default(),
            layout,
            broken_width: None,
            min_content: None,
            links,
            painted: RefCell::new(None),
        }
    }

    /// Drops the layout, the paint operations built from it and the cached
    /// min-content size.
    fn invalidate_layout(&mut self) {
        *self.layout.borrow_mut() = None;
        self.broken_width = None;
        self.min_content = None;
        *self.painted.borrow_mut() = None;
    }

    /// Shapes the spans when no layout is kept, and breaks its lines at
    /// `max_width`.
    fn shape_and_break(&mut self, max_width: Option<f32>) {
        let mut layout = self.layout.borrow_mut();
        match &mut *layout {
            Some(layout) => layout.rebreak(max_width),
            None => {
                let fonts: Vec<(Range<usize>, SpanFont)> = self
                    .ranges
                    .iter()
                    .zip(self.spans.iter())
                    .map(|(range, span)| (range.clone(), span_font(&span.style)))
                    .collect();
                *layout = Some(RichLayout::new(&self.text, &self.style, &fonts, max_width));
            }
        }
        self.broken_width = Some(max_width);
        *self.painted.borrow_mut() = None;
    }

    /// Returns the color of the span `span` under the current theme: its own
    /// color, the accent color for a link, or otherwise `base`.
    fn span_color(span: &Span, base: Color, cx: &PaintCx<'_>) -> Color {
        match (&span.style.color, &span.style.link) {
            (Some(color), _) => color.resolve(cx.theme),
            (None, Some(_)) => ColorValue::Token(ColorToken::ACCENT).resolve(cx.theme),
            (None, None) => base,
        }
    }
}

impl Widget for RichText {
    /// Reports the joined text as the accessible value, as a label does.
    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            role: Role::Label,
            label: None,
            value: Some(self.text.clone()),
        })
    }

    /// Lays the spans out against the offered width and returns their size,
    /// answering the offered widths the way a label does.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of
    /// [`TextSystem::with`](crate::TextSystem::with), in particular when
    /// called from inside another `with` call on the same thread.
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        if input.available_width == AvailableSpace::MinContent && input.known_width.is_none() {
            if self.min_content.is_none() {
                let painted = self.broken_width;
                self.shape_and_break(Some(0.0));
                self.min_content = self.layout.borrow().as_ref().map(RichLayout::size);
                self.shape_and_break(painted.unwrap_or(None));
            }
            return self.min_content;
        }
        let max_width = match input.available_width {
            AvailableSpace::Definite(width) => Some(width),
            AvailableSpace::MaxContent => None,
            AvailableSpace::MinContent => Some(0.0),
        };
        if self.broken_width != Some(max_width) {
            self.shape_and_break(max_width);
        }
        self.layout.borrow().as_ref().map(RichLayout::size)
    }

    /// Appends the glyph runs of every span, each in its span's color, and
    /// the underline of every link.
    ///
    /// The operations are rebuilt when they were invalidated, when the node
    /// paints at another position, or when a resolved span color changed.
    /// Resolving a token marks the node to repaint on a theme switch.
    ///
    /// # Panics
    ///
    /// Panics when the layout or the paint operations are already borrowed
    /// mutably on this thread, which the paint pass cannot do: the pointer
    /// handlers that share the layout only read it, and never while a node
    /// paints.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let layout = self.layout.borrow();
        let Some(layout) = layout.as_ref() else {
            return;
        };
        let base = match &cx.style.paint.text_color {
            Some(color) => color.resolve(cx.theme),
            None => self.style.color.resolve(cx.theme),
        };
        let origin = cx.rect.origin();
        let mut painted = self.painted.borrow_mut();
        let current = painted.as_ref().is_some_and(|painted| {
            painted.origin == origin
                && painted.colors.len() == self.spans.len()
                && self
                    .spans
                    .iter()
                    .zip(&painted.colors)
                    .all(|(span, color)| Self::span_color(span, base, cx) == *color)
        });
        if !current {
            let colors: Vec<Color> = self
                .spans
                .iter()
                .map(|span| Self::span_color(span, base, cx))
                .collect();
            let (runs, underlines) = rich_glyph_runs(layout, origin, &colors);
            let mut ops: Vec<PaintOp> = runs
                .into_iter()
                .map(|run: GlyphRun| PaintOp::Glyphs(Rc::new(run)))
                .collect();
            ops.extend(
                underlines
                    .into_iter()
                    .map(|(rect, color)| PaintOp::FillRect {
                        rect,
                        radius: Corners::ZERO,
                        color,
                    }),
            );
            *painted = Some(Painted {
                origin,
                colors,
                ops,
            });
        }
        if let Some(painted) = painted.as_ref() {
            out.extend(painted.ops.iter().cloned());
        }
    }
}

/// Creates a paragraph that shows `spans`, wrapped as one block.
///
/// The spans can be fixed or read from signals. A change that touches only
/// span colors repaints the paragraph without laying it out, and any change
/// to the text, the font style or which spans are links lays it out again.
///
/// A span with a link paints in the theme's accent color unless it sets a
/// color, draws an underline, and shows the pointing cursor while the
/// pointer is over it. A press and a release over the same link run its
/// handler once. The paragraph responds to the pointer over its links
/// alone, so a press or a hover anywhere else reaches the nearest ancestor
/// that responds. The accessible value is the joined text of every span.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_text::{FontWeight, Span, SpanStyle, rich_text};
///
/// let paragraph = rich_text(Rc::from([
///     Span::new("Read the ", SpanStyle::default()),
///     Span::new(
///         "guide",
///         SpanStyle {
///             link: Some(Rc::new(|| println!("open the guide"))),
///             ..SpanStyle::default()
///         },
///     ),
///     Span::new(
///         " first.",
///         SpanStyle {
///             weight: Some(FontWeight::BOLD),
///             ..SpanStyle::default()
///         },
///     ),
/// ]));
/// ```
pub fn rich_text(spans: impl Into<Value<Rc<[Span]>>>) -> Element<RichText> {
    let layout: Rc<RefCell<Option<RichLayout>>> = Rc::new(RefCell::new(None));
    let links: Links = Rc::new(RefCell::new(Vec::new()));
    let pressed: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let link_at: Rc<dyn Fn(Point) -> Option<usize>> = {
        let layout = Rc::clone(&layout);
        let links = Rc::clone(&links);
        Rc::new(move |point: Point| {
            let span = layout.borrow().as_ref()?.span_at(point.x, point.y)?;
            links.borrow().get(span)?.as_ref()?;
            Some(span)
        })
    };

    let down_link = Rc::clone(&link_at);
    let down_pressed = Rc::clone(&pressed);
    let up_links = Rc::clone(&links);
    let region_link = Rc::clone(&link_at);
    // The ambient style binding is installed first, so its update runs before
    // the updates of the builder methods and an explicit field wins over it.
    // The cursor shows only where the paragraph responds, over its links.
    Element::new(RichText::new(Rc::clone(&layout), Rc::clone(&links)))
        .bind(AmbientStyleKey, current_text_style, apply_ambient_style)
        .bind(SpansKey, spans, set_spans)
        .cursor(Cursor::Pointer)
        .responds_at(move |point| region_link(point).is_some())
        .on_pointer_down(move |point| down_pressed.set(down_link(point)))
        .on_pointer_up(move |point| {
            let Some(span) = pressed.take() else {
                return;
            };
            if link_at(point) != Some(span) {
                return;
            }
            // The handler runs with no borrow held, so it can write signals
            // that hand the paragraph new spans.
            let handler = up_links.borrow().get(span).cloned().flatten();
            if let Some(handler) = handler {
                handler();
            }
        })
}

/// Builder methods that set the block style of a [`rich_text`] element.
///
/// Import the trait to use them. Every method sets one field of the
/// paragraph's text style, which every span starts from. The remaining
/// fields come from the ambient style at mount. The text color of the spans
/// that set none is a paint property of the node, set with
/// [`text_color`](smudge_core::element::Element::text_color).
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_text::{FontWeight, RichTextExt, Span, SpanStyle, rich_text};
///
/// let paragraph = rich_text(Rc::from([Span::new("Hello", SpanStyle::default())]))
///     .text_size(18.0)
///     .weight(FontWeight::SEMI_BOLD);
/// ```
pub trait RichTextExt: Sized {
    /// Sets the font size in logical pixels, which lays the paragraph out
    /// again.
    fn text_size(self, size: impl Into<Value<f32>>) -> Self;

    /// Sets the weight of the spans that set none, which lays the paragraph
    /// out again.
    fn weight(self, weight: FontWeight) -> Self;

    /// Sets the family of the spans that are not monospace, which lays the
    /// paragraph out again.
    fn family(self, family: FontFamily<'static>) -> Self;
}

/// The key of a paragraph's ambient text style binding.
struct AmbientStyleKey;

/// The key of a paragraph's spans binding.
struct SpansKey;

/// The key of a paragraph's font size binding.
struct TextSizeKey;

/// The key of a paragraph's weight binding.
struct WeightKey;

/// The key of a paragraph's family binding.
struct FamilyKey;

impl RichTextExt for Element<RichText> {
    fn text_size(self, size: impl Into<Value<f32>>) -> Self {
        self.bind(TextSizeKey, size, set_text_size)
    }

    fn weight(self, weight: FontWeight) -> Self {
        self.bind(WeightKey, Value::Static(weight), set_weight)
    }

    fn family(self, family: FontFamily<'static>) -> Self {
        self.bind(FamilyKey, Value::Static(family), set_family)
    }
}

/// Which fields of the block style an explicit write has set.
#[derive(Clone, Copy, Default)]
struct ExplicitStyle {
    /// The family was set by [`RichTextExt::family`].
    family: bool,
    /// The size was set by [`RichTextExt::text_size`].
    size: bool,
    /// The weight was set by [`RichTextExt::weight`].
    weight: bool,
}

/// Writes `spans` into the paragraph, and lays it out again or repaints it
/// as far as the change asks.
fn set_spans(rich: &mut RichText, spans: Rc<[Span]>) -> Dirty {
    if Rc::ptr_eq(&rich.spans, &spans) {
        return Dirty::Clean;
    }
    let change = span_change(&rich.spans, &spans);
    {
        let mut links = rich.links.borrow_mut();
        links.clear();
        links.extend(spans.iter().map(|span| span.style.link.clone()));
    }
    if change == SpanChange::Layout {
        rich.text.clear();
        rich.ranges.clear();
        for span in spans.iter() {
            let start = rich.text.len();
            rich.text.push_str(&span.text);
            rich.ranges.push(start..rich.text.len());
        }
        rich.spans = spans;
        rich.invalidate_layout();
        return Dirty::Layout;
    }
    rich.spans = spans;
    match change {
        SpanChange::Colors => Dirty::Paint,
        _ => Dirty::Clean,
    }
}

/// Writes `size` into the block style and lays the paragraph out again when
/// it changed.
fn set_text_size(rich: &mut RichText, size: f32) -> Dirty {
    rich.explicit.size = true;
    if rich.style.size == size {
        return Dirty::Clean;
    }
    rich.style.size = size;
    rich.invalidate_layout();
    Dirty::Layout
}

/// Writes `weight` into the block style and lays the paragraph out again
/// when it changed.
fn set_weight(rich: &mut RichText, weight: FontWeight) -> Dirty {
    rich.explicit.weight = true;
    if rich.style.weight == weight {
        return Dirty::Clean;
    }
    rich.style.weight = weight;
    rich.invalidate_layout();
    Dirty::Layout
}

/// Writes `family` into the block style and lays the paragraph out again
/// when it changed.
fn set_family(rich: &mut RichText, family: FontFamily<'static>) -> Dirty {
    rich.explicit.family = true;
    if rich.style.family == family {
        return Dirty::Clean;
    }
    rich.style.family = family;
    rich.invalidate_layout();
    Dirty::Layout
}

/// Fills the fields of the block style that no explicit write has set from
/// the ambient style, which is read once at mount.
fn apply_ambient_style(rich: &mut RichText, mut ambient: TextStyle) -> Dirty {
    let explicit = rich.explicit;
    if explicit.family {
        ambient.family = rich.style.family.clone();
    }
    if explicit.size {
        ambient.size = rich.style.size;
    }
    if explicit.weight {
        ambient.weight = rich.style.weight;
    }
    if ambient == rich.style {
        return Dirty::Clean;
    }
    rich.style = ambient;
    rich.invalidate_layout();
    Dirty::Layout
}

#[cfg(test)]
#[path = "rich_text_tests.rs"]
mod tests;
