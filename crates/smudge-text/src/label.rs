//! The label widget: one block of static or reactive text.
//!
//! A [`Label`] lays its text out through the thread's [`TextSystem`] with one
//! [`TextStyle`], measures itself from the laid out lines, and paints the
//! resulting glyph runs. A change to the text, the font size, the weight or
//! the family lays the label out again, and a change to the color only
//! repaints it.
//!
//! Create one with [`label`] and shape it with the [`LabelExt`] builder
//! methods. At mount the label reads the ambient text style, which
//! [`provide_text_style`](crate::provide_text_style) sets, and fills every
//! style field that no builder method set. The color the glyphs paint with
//! resolves against the theme when the label paints, so a theme switch
//! repaints it: the node's
//! [`text_color`](smudge_core::element::Element::text_color) when it sets
//! one, which may name a theme token, and otherwise the ambient style's
//! color, which defaults to the theme's text token.

use std::cell::RefCell;
use std::rc::Rc;

use parley::FontFamily;
use parley::FontWeight;
use smudge_core::element::Element;
use smudge_core::element::Value;
use smudge_core::geometry::Color;
use smudge_core::geometry::Point;
use smudge_core::geometry::Size;
use smudge_core::paint::GlyphRun;
use smudge_core::paint::PaintOp;
use smudge_core::semantics::{Role, Semantics};
use smudge_core::tree::Dirty;
use smudge_core::widget::AvailableSpace;
use smudge_core::widget::MeasureInput;
use smudge_core::widget::PaintCx;
use smudge_core::widget::Widget;

use crate::layout::TextLayout;
use crate::style::TextStyle;
use crate::style::current_text_style;

/// A block of text laid out with one text style.
///
/// Build one with [`label`]. The label keeps the layout of its current text
/// and rebuilds it only when the text, the style or the offered width
/// changes, and keeps the glyph runs of that layout and rebuilds them only
/// when the layout, the node's position or the resolved text color changes.
pub struct Label {
    /// The text the label shows.
    text: String,
    /// The text style the label lays out and paints with, resolved from the
    /// ambient style and the explicit fields at mount.
    style: TextStyle,
    /// The style fields an explicit write set, which the ambient style leaves
    /// alone.
    explicit: ExplicitStyle,
    /// The layout of [`Label::text`], or `None` before the first measure and
    /// after every change that invalidates it.
    layout: Option<TextLayout>,
    /// The maximum width the current layout was built with, which keys the
    /// layout cache.
    laid_out_width: Option<f32>,
    /// The size of the text wrapped at every opportunity, which is its
    /// min-content size, or `None` until it is asked for.
    ///
    /// It is cached apart from [`Label::layout`], so answering a min-content
    /// request never replaces the layout the label paints.
    min_content: Option<Size>,
    /// The glyph runs of the current layout, with the origin and the resolved
    /// color they were built at. Paint operations share them through an `Rc`.
    runs: RefCell<Option<CachedRuns>>,
}

/// The glyph runs of one layout, with the origin and resolved color they were
/// built at.
struct CachedRuns {
    /// The origin the glyph positions are absolute to.
    origin: Point,
    /// The color every run was built with.
    color: Color,
    /// The runs, shared by the paint operations that show them.
    runs: Vec<Rc<GlyphRun>>,
}

impl Label {
    /// Creates a label with the default style until the mount reads the
    /// ambient one.
    fn new(text: String) -> Label {
        Label {
            text,
            style: TextStyle::default(),
            explicit: ExplicitStyle::default(),
            layout: None,
            laid_out_width: None,
            min_content: None,
            runs: RefCell::new(None),
        }
    }

    /// Drops the cached layout, the glyph runs built from it and the cached
    /// min-content size.
    fn invalidate_layout(&mut self) {
        self.layout = None;
        self.laid_out_width = None;
        self.min_content = None;
        *self.runs.borrow_mut() = None;
    }
}

impl Widget for Label {
    /// Reports the text as the accessible value.
    ///
    /// The role is what an accessibility backend maps a static text to, and
    /// such a node takes its name from its value there, so the text rides in
    /// the value and the label stays empty.
    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            role: Role::Label,
            label: None,
            value: Some(self.text.clone()),
        })
    }

    /// Lays the text out against the offered width and returns its size.
    ///
    /// A definite [`AvailableSpace::Definite`] wraps the text at that width,
    /// and [`AvailableSpace::MaxContent`] lays it out on one line. An
    /// [`AvailableSpace::MinContent`] request gets the text wrapped at every
    /// opportunity, whose width is its longest unbreakable word, so a label
    /// in a row too narrow for it can shrink and wrap instead of overflowing
    /// and squeezing its siblings. The layout is reused while the text, the
    /// style and the offered width are unchanged, and measure invalidates the
    /// cached glyph runs whenever it builds a new layout.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`](crate::TextSystem::with),
    /// in particular when called from inside another `with` call on the same
    /// thread.
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        if input.available_width == AvailableSpace::MinContent && input.known_width.is_none() {
            if self.min_content.is_none() {
                let wrapped = TextLayout::new(&self.text, &self.style, Some(0.0));
                self.min_content = Some(wrapped.size());
            }
            return self.min_content;
        }
        let max_width = wrap_width(input.available_width);
        if self.layout.is_none() || self.laid_out_width != max_width {
            self.layout = Some(TextLayout::new(&self.text, &self.style, max_width));
            self.laid_out_width = max_width;
            *self.runs.borrow_mut() = None;
        }
        let layout = self
            .layout
            .as_ref()
            .expect("the layout was just built or is cached");
        Some(layout.size())
    }

    /// Appends one [`PaintOp::Glyphs`] per run of the current layout.
    ///
    /// The runs are rebuilt when they were invalidated, when the node paints
    /// at another position, or when the resolved text color changed, because
    /// glyph positions and colors are baked into them. Each run carries the
    /// color the node's text color or the label's style resolves to under the
    /// current theme. When that color is a token, resolving it marks the node
    /// to repaint on a theme switch, and a literal color does not.
    ///
    /// # Panics
    ///
    /// Panics when the glyph runs are already borrowed on this thread, which
    /// the paint pass cannot do: it paints one node at a time and a widget
    /// never paints its own node again from inside the call.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let Some(layout) = &self.layout else {
            return;
        };
        let color = self.text_color(cx);
        let origin = cx.rect.origin();
        let mut cache = self.runs.borrow_mut();
        let rebuild = match &*cache {
            Some(cached) => cached.origin != origin || cached.color != color,
            None => true,
        };
        if rebuild {
            // The color is baked into the runs here rather than into the
            // layout, so a color change repaints the label without laying
            // its text out again.
            let runs = layout
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

impl Label {
    /// Returns the color the glyphs paint with: the node's text color when it
    /// sets one, otherwise the label's style color. Both may name a theme
    /// token, so both resolve against the theme that is current at paint
    /// time.
    fn text_color(&self, cx: &PaintCx<'_>) -> Color {
        match &cx.style.paint.text_color {
            Some(color) => color.resolve(cx.theme),
            None => self.style.color.resolve(cx.theme),
        }
    }
}

/// Creates a label that shows `text`.
///
/// The text can be fixed or read from signals:
///
/// ```
/// use smudge_core::reactive::{prelude::*, signal_local};
/// use smudge_text::label;
///
/// let (name, set_name) = signal_local("World".to_owned());
/// let greeting = label(move || format!("Hello, {}", name.get()));
/// set_name.set("Smudge".to_owned());
/// ```
pub fn label(text: impl Into<Value<String>>) -> Element<Label> {
    // The ambient style binding is installed first, so its update runs before
    // the updates of the builder methods and an explicit field wins over it.
    Element::new(Label::new(String::new()))
        .bind(current_text_style, apply_ambient_style)
        .bind(text, set_text)
}

/// Builder methods that set the text style of a [`label`] element.
///
/// Import the trait to use them:
///
/// ```
/// use smudge_text::LabelExt;
/// use smudge_text::label;
///
/// let element = label("Hello").text_size(18.0);
/// ```
///
/// Every method sets one field of the label's text style; the remaining
/// fields come from the ambient style at mount. The color the glyphs paint
/// with is a paint property of the node, set with
/// [`text_color`](smudge_core::element::Element::text_color), which needs no
/// import.
pub trait LabelExt: Sized {
    /// Sets the font size in logical pixels.
    ///
    /// The size changes what the label measures, so the node is laid out
    /// again.
    fn text_size(self, size: impl Into<Value<f32>>) -> Self;

    /// Sets the weight the family is matched with.
    ///
    /// The weight can change what the label measures, so the node is laid out
    /// again.
    fn weight(self, weight: FontWeight) -> Self;

    /// Sets the font family the text resolves to.
    ///
    /// The family can change what the label measures, so the node is laid out
    /// again.
    fn family(self, family: FontFamily<'static>) -> Self;
}

impl LabelExt for Element<Label> {
    fn text_size(self, size: impl Into<Value<f32>>) -> Self {
        self.bind(size, set_text_size)
    }

    fn weight(self, weight: FontWeight) -> Self {
        self.bind(Value::Static(weight), set_weight)
    }

    fn family(self, family: FontFamily<'static>) -> Self {
        self.bind(Value::Static(family), set_family)
    }
}

/// Which fields of the text style an explicit write has set.
///
/// The ambient style is read once at mount and fills only the other fields,
/// so an explicit size keeps its value when the ambient style differs.
#[derive(Clone, Copy, Default)]
struct ExplicitStyle {
    /// The family was set by [`LabelExt::family`].
    family: bool,
    /// The size was set by [`LabelExt::text_size`].
    size: bool,
    /// The weight was set by [`LabelExt::weight`].
    weight: bool,
}

/// Writes `text` into the label and relays it out when it changed.
fn set_text(label: &mut Label, text: String) -> Dirty {
    if label.text == text {
        return Dirty::Clean;
    }
    label.text = text;
    label.invalidate_layout();
    Dirty::Layout
}

/// Writes `size` into the label's style and relays it out when it changed.
fn set_text_size(label: &mut Label, size: f32) -> Dirty {
    label.explicit.size = true;
    if label.style.size == size {
        return Dirty::Clean;
    }
    label.style.size = size;
    label.invalidate_layout();
    Dirty::Layout
}

/// Writes `weight` into the label's style and relays it out when it changed.
fn set_weight(label: &mut Label, weight: FontWeight) -> Dirty {
    label.explicit.weight = true;
    if label.style.weight == weight {
        return Dirty::Clean;
    }
    label.style.weight = weight;
    label.invalidate_layout();
    Dirty::Layout
}

/// Writes `family` into the label's style and relays it out when it changed.
fn set_family(label: &mut Label, family: FontFamily<'static>) -> Dirty {
    label.explicit.family = true;
    if label.style.family == family {
        return Dirty::Clean;
    }
    label.style.family = family;
    label.invalidate_layout();
    Dirty::Layout
}

/// Fills the fields of the label's style that no explicit write has set from
/// the ambient style, which is read once at mount.
fn apply_ambient_style(label: &mut Label, mut ambient: TextStyle) -> Dirty {
    let explicit = label.explicit;
    if explicit.family {
        ambient.family = label.style.family.clone();
    }
    if explicit.size {
        ambient.size = label.style.size;
    }
    if explicit.weight {
        ambient.weight = label.style.weight;
    }
    if ambient == label.style {
        return Dirty::Clean;
    }
    label.style = ambient;
    label.invalidate_layout();
    Dirty::Layout
}

/// The maximum width a layout wraps at for the space offered on the x axis.
///
/// A definite space wraps the text at that width, a max-content request lays
/// it out on one line, and a min-content request wraps it at every
/// opportunity. `measure` answers min-content from its own cache before it
/// gets here, so the last arm only keeps the mapping right for any other
/// caller.
fn wrap_width(space: AvailableSpace) -> Option<f32> {
    match space {
        AvailableSpace::Definite(width) => Some(width),
        AvailableSpace::MaxContent => None,
        AvailableSpace::MinContent => Some(0.0),
    }
}
