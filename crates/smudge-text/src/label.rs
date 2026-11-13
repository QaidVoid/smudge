//! The label widget: one block of static or reactive text.
//!
//! A [`Label`] lays its text out through the thread's [`TextSystem`] with one
//! [`TextStyle`], measures itself from the laid out lines, and paints the
//! resulting glyph runs. A change to the text, the font size, the weight,
//! the family or the line cap lays the label out again, and a change to the
//! color only repaints it.
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
    /// The most lines the label shows, or `None` for every line.
    max_lines: Option<u32>,
    /// The layout of [`Label::text`], or `None` before the first measure and
    /// after every change that invalidates it.
    ///
    /// The layout holds the shaped text once; measuring at another offered
    /// width re-breaks its lines instead of shaping the text again, which is
    /// what keeps a layout engine's repeated measure calls for one node cheap.
    layout: Option<TextLayout>,
    /// The width the lines of [`Label::layout`] were last broken at, `None`
    /// for max content, or `None` inside `None` when nothing is laid out.
    broken_width: Option<Option<f32>>,
    /// The size of the text wrapped at every opportunity, which is its
    /// min-content size, or `None` until it is asked for.
    ///
    /// It is cached apart from the line breaking, so answering a min-content
    /// request never depends on the width other calls measured at.
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
            max_lines: None,
            layout: None,
            broken_width: None,
            min_content: None,
            runs: RefCell::new(None),
        }
    }

    /// Drops the cached layout, the glyph runs built from it and the cached
    /// min-content size.
    fn invalidate_layout(&mut self) {
        self.layout = None;
        self.broken_width = None;
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
    /// and squeezing its siblings. A label capped to one line answers with
    /// its ellipsis alone and one line of height, since it can cut any text
    /// down to that. Under a cap of more lines, the min-content height stays
    /// the full text's, since it answers what the text needs rather than what
    /// the cap shows; the line cap reports its height at a definite width.
    ///
    /// The text is shaped once and kept; a call that offers another width
    /// than the last one re-breaks the shaped text's lines, because the
    /// layout engine measures one node at several widths within a pass. The
    /// min-content answer breaks the same shaped text at every opportunity
    /// and then breaks it back at the width the label was last measured at,
    /// so once the label has been measured at one, the answer never changes
    /// the lines paint draws. The glyph runs are dropped whenever the shaped text
    /// or its line breaking changed, so paint rebuilds them from the lines the
    /// last call left.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`](crate::TextSystem::with),
    /// in particular when called from inside another `with` call on the same
    /// thread.
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        if input.available_width == AvailableSpace::MinContent && input.known_width.is_none() {
            if self.min_content.is_none() {
                let painted = self.broken_width;
                self.shape_and_break(Some(0.0));
                let layout = self
                    .layout
                    .as_mut()
                    .expect("the layout was just built or is cached");
                if matches!(self.max_lines, Some(0 | 1)) {
                    layout.truncate(self.max_lines, Some(0.0), &self.style);
                }
                self.min_content = Some(layout.size());
                // Paint draws the lines the layout holds, so they go back to
                // the width the label was last measured at, truncated as the
                // label was last shown. A label measured nowhere yet goes
                // back to max content, so the cache key the next measure
                // reads never describes a state the label was not left in.
                self.shape_and_show(painted.unwrap_or(None));
            }
            return self.min_content;
        }
        let max_width = wrap_width(input.available_width);
        if self.broken_width != Some(max_width) {
            self.shape_and_show(max_width);
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
    /// Shapes [`Label::text`] with [`Label::style`] when no shaped layout is
    /// kept, breaks its lines at `max_width`, and cuts the lines to
    /// [`Label::max_lines`].
    ///
    /// Both outcomes change the lines, so the cached glyph runs, which bake
    /// the line positions in, are dropped here.
    fn shape_and_show(&mut self, max_width: Option<f32>) {
        self.shape_and_break(max_width);
        if let Some(layout) = &mut self.layout {
            layout.truncate(self.max_lines, max_width, &self.style);
        }
    }

    /// Shapes [`Label::text`] with [`Label::style`] when no shaped layout is
    /// kept, and breaks its lines at `max_width` without truncating them.
    ///
    /// The min-content measure breaks at every opportunity to read the
    /// longest unbreakable word off the lines, which truncation would
    /// shorten, so it asks for the untruncated break.
    fn shape_and_break(&mut self, max_width: Option<f32>) {
        match &mut self.layout {
            Some(layout) => layout.rebreak(max_width),
            None => {
                self.layout = Some(TextLayout::new(&self.text, &self.style, max_width));
            }
        }
        self.broken_width = Some(max_width);
        *self.runs.borrow_mut() = None;
    }

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
        .bind(AmbientStyleKey, current_text_style, apply_ambient_style)
        .bind(TextKey, text, set_text)
}

/// Builder methods that set the text style of a [`label`] element.
///
/// Import the trait to use them. The style types the methods name come from
/// this crate, so nothing else is needed:
///
/// ```
/// use smudge_text::FontWeight;
/// use smudge_text::LabelExt;
/// use smudge_text::label;
///
/// let element = label("Hello").text_size(18.0).weight(FontWeight::SEMI_BOLD);
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

    /// Caps the lines the label shows at `max_lines`.
    ///
    /// A layout that breaks into more lines, or whose last kept line is
    /// wider than the label, keeps its first `max_lines` and ends the last
    /// one with an ellipsis, cut back on a cluster boundary so the whole fits
    /// the width the label wraps at. A width below the ellipsis's own advance
    /// leaves it standing alone and past the width. Right-to-left text keeps
    /// its first lines without an ellipsis, and so does text in fonts that
    /// map neither an ellipsis nor a full stop. `None`, the default, shows
    /// every line. The accessible value keeps the full text either way. The
    /// cap can change what the label measures, so the node is laid out
    /// again, without shaping the text again.
    ///
    /// ```
    /// use smudge_text::LabelExt;
    /// use smudge_text::label;
    ///
    /// let capped = label("Hello").max_lines(Some(2));
    /// let uncapped = label("Hello").max_lines(None);
    /// ```
    fn max_lines(self, max_lines: impl Into<Value<Option<u32>>>) -> Self;
}

/// The key of a label's ambient text style binding.
struct AmbientStyleKey;

/// The key of a label's text binding.
struct TextKey;

/// The key of a label's font size binding.
struct TextSizeKey;

/// The key of a label's weight binding.
struct WeightKey;

/// The key of a label's family binding.
struct FamilyKey;

/// The key of a label's line cap binding.
struct MaxLinesKey;

impl LabelExt for Element<Label> {
    fn text_size(self, size: impl Into<Value<f32>>) -> Self {
        self.bind(TextSizeKey, size, set_text_size)
    }

    fn weight(self, weight: FontWeight) -> Self {
        self.bind(WeightKey, Value::Static(weight), set_weight)
    }

    fn family(self, family: FontFamily<'static>) -> Self {
        self.bind(FamilyKey, Value::Static(family), set_family)
    }

    fn max_lines(self, max_lines: impl Into<Value<Option<u32>>>) -> Self {
        self.bind(MaxLinesKey, max_lines, set_max_lines)
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

/// Writes the line cap into the label and relays it out when it changed.
///
/// The shaped text stays: only its lines are broken and cut again.
fn set_max_lines(label: &mut Label, max_lines: Option<u32>) -> Dirty {
    if label.max_lines == max_lines {
        return Dirty::Clean;
    }
    label.max_lines = max_lines;
    label.broken_width = None;
    label.min_content = None;
    *label.runs.borrow_mut() = None;
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

#[cfg(test)]
#[path = "label_tests.rs"]
mod tests;
