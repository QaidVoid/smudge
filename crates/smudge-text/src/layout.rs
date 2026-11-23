//! A laid out block of text.
//!
//! [`TextLayout`] wraps parley's layout so that callers shape one style of
//! text against a maximum width and read back the resulting size and glyph
//! runs. Parley stays an implementation detail of the layout itself: the
//! rest of Smudge sees this wrapper, the font style types this crate
//! re-exports, and, through the run iterator, parley's own public run types.

use std::ops::Range;

use crate::glyphs::ellipsis_glyph;
use crate::style::TextStyle;
use crate::system::TextSystem;
use parley::PositionedLayoutItem;
use parley::StyleProperty;
use parley::fontique::Synthesis;
use parley::style::LineHeight;
use smudge_core::geometry::{Point, Rect, Size};

/// A block of text laid out in logical pixels against one style.
///
/// The layout is computed eagerly by [`TextLayout::new`] and never changes
/// afterwards. A new layout replaces an old one rather than being edited. It
/// carries no color: layout runs without a theme, and the caller paints the
/// glyph runs in whatever color it resolves when it paints.
pub struct TextLayout {
    /// The shaped and line-broken text.
    pub(crate) layout: parley::Layout<()>,
    /// The truncation the layout was last cut to, or `None` when it shows
    /// every line.
    ///
    /// The text stays shaped and broken whole: truncation only decides what
    /// [`TextLayout::size`] measures and which lines and glyphs the
    /// conversion reports, so a later re-break drops it for free.
    pub(crate) truncation: Option<Truncation>,
    /// The ellipsis of the style's primary font, which truncation falls
    /// back to when the font of the run it cuts maps none. `None` until a cut
    /// first needs it, and `Some(None)` when that font maps none either.
    ///
    /// It is resolved at most once per shaped text, since a style change
    /// lays the text out anew.
    fallback_ellipsis: Option<Option<Ellipsis>>,
}

/// The truncation of a laid out text block: how many lines survive and how
/// the last of them is cut.
#[derive(Clone, Debug)]
pub(crate) struct Truncation {
    /// The number of leading lines the truncated layout keeps.
    pub(crate) kept_lines: usize,
    /// The cut of the last kept line, or `None` when that line keeps all of
    /// itself: it has no glyph runs to cut, or the layout runs right to left
    /// and only drops lines.
    pub(crate) cut: Option<Cut>,
    /// The width the truncated layout reports.
    pub(crate) width: f32,
    /// The height the truncated layout reports.
    pub(crate) height: f32,
}

/// The cut of one line: where its glyphs stop and the ellipsis that replaces
/// the rest.
#[derive(Clone, Debug)]
pub(crate) struct Cut {
    /// The index of the glyph run the cut falls into, counted over the glyph
    /// runs of the line in visual order. The runs before it survive whole.
    pub(crate) run: usize,
    /// The number of leading glyphs of that run the line keeps, which always
    /// ends on a cluster boundary.
    pub(crate) glyphs: usize,
    /// The pen position the ellipsis starts at, relative to the layout's
    /// left edge.
    pub(crate) x: f32,
    /// The baseline of the cut line, relative to the layout's top edge.
    pub(crate) baseline: f32,
    /// The ellipsis appended after the kept glyphs, or `None` when no font
    /// maps one and the line ends without it.
    pub(crate) ellipsis: Option<Ellipsis>,
}

/// The ellipsis glyph truncation appends in place of cut text.
#[derive(Clone, Debug)]
pub(crate) struct Ellipsis {
    /// The font the glyph comes from.
    pub(crate) font: parley::FontData,
    /// The font size the glyph paints at.
    pub(crate) size: f32,
    /// The embolden and slant the face needs for the style that asked for
    /// the ellipsis.
    pub(crate) synthesis: Synthesis,
    /// The variable instance the glyph paints at, as
    /// [`GlyphRun::coords`](smudge_core::paint::GlyphRun::coords) carries it.
    pub(crate) coords: Vec<i16>,
    /// The glyph identifier of the horizontal ellipsis, or of the full stop
    /// when the font lacks one.
    pub(crate) glyph: u32,
    /// How many times the glyph paints: once for the ellipsis, three times
    /// for the full stops that stand in for it.
    pub(crate) dots: u32,
    /// The advance of one painted glyph in logical pixels.
    pub(crate) advance: f32,
}

impl Ellipsis {
    /// The advance of the whole ellipsis, every painted dot included.
    pub(crate) fn width(&self) -> f32 {
        self.advance * self.dots as f32
    }
}

/// The longest prefix of a line found so far that fits with its ellipsis.
struct Candidate {
    /// The glyph run the prefix ends in.
    run: usize,
    /// The glyphs of that run the prefix keeps.
    glyphs: usize,
    /// The pen position after the prefix.
    x: f32,
    /// The pen position after the prefix and its ellipsis, measured from the
    /// line's start.
    end: f32,
}

impl TextLayout {
    /// Lays out `text` with `style`, wrapping lines at `max_width` logical
    /// pixels, or not at all when it is `None`.
    ///
    /// Layout runs against the current thread's [`TextSystem`].
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`], in particular
    /// when called from inside another `with` call on the same thread.
    pub fn new(text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout {
        TextSystem::with(|system| {
            Self::build(
                text,
                style,
                max_width,
                &mut system.fonts,
                &mut system.layouts,
            )
        })
    }

    /// Lays out `text` with `style` against `max_width` using the font and
    /// layout contexts the caller already holds.
    ///
    /// [`TextLayout::new`] is this over the current thread's [`TextSystem`].
    /// The editor keeps one `with` call open across its work and builds its
    /// paragraph layouts through this, which a nested `with` would panic on.
    pub(crate) fn build(
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
        fonts: &mut parley::FontContext,
        layouts: &mut parley::LayoutContext<()>,
    ) -> TextLayout {
        // Layout units are logical pixels; scaling to a device happens in
        // the renderer, not here. Coordinates stay fractional so that a
        // renderer can quantize them itself.
        #[cfg(test)]
        SHAPES.with(|shapes| shapes.set(shapes.get() + 1));
        let mut builder = layouts.ranged_builder(fonts, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(style.family.clone()));
        builder.push_default(StyleProperty::FontSize(style.size));
        builder.push_default(StyleProperty::FontWeight(style.weight));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
            style.line_height,
        )));

        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        TextLayout {
            layout,
            truncation: None,
            fallback_ellipsis: None,
        }
    }

    /// Re-wraps the laid out text at `max_width` without shaping it again.
    ///
    /// Shaping runs once per text and style in [`TextLayout::new`]; the width
    /// only decides where lines break, so a layout engine that asks for the
    /// same text at several widths in one pass pays for the breaks alone.
    /// The lines start over, so a truncation cut by [`TextLayout::truncate`]
    /// is dropped.
    pub(crate) fn rebreak(&mut self, max_width: Option<f32>) {
        self.layout.break_all_lines(max_width);
        self.truncation = None;
    }

    /// Cuts the layout to at most `max_lines` lines, ending the last kept
    /// line with an ellipsis that fits `width`.
    ///
    /// Lines after the `max_lines` first are dropped. When more lines than
    /// that result at `width`, or the last kept line is wider than `width`,
    /// the last kept line keeps the longest prefix of whole clusters that
    /// fits the width once the ellipsis is counted, and the ellipsis
    /// replaces the rest of the line. Whitespace never ends the kept prefix.
    /// The ellipsis is the horizontal ellipsis, or three full stops where a
    /// font lacks it, from the font of the run it ends, or from the style's
    /// primary font when that font maps neither. When no font maps one, the
    /// line ends without it. It stands alone when the width cannot fit even
    /// it.
    ///
    /// Without a width the last kept line stays whole and the ellipsis
    /// follows it.
    ///
    /// The text stays laid out whole, so a later [`TextLayout::rebreak`]
    /// undoes the cut without reshaping. A layout whose dominant direction
    /// is right-to-left keeps its first lines without an ellipsis, since the
    /// ellipsis would have to lead the line rather than close it.
    ///
    /// `style` must be the style the layout was built with.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`] when the style's
    /// ellipsis has to be resolved, in particular when called from inside
    /// another `with` call on the same thread.
    pub(crate) fn truncate(
        &mut self,
        max_lines: Option<u32>,
        width: Option<f32>,
        style: &TextStyle,
    ) {
        self.truncate_resolving(max_lines, width, style, &mut ellipsis_glyph);
    }

    /// Cuts the layout as [`TextLayout::truncate`] does, with `own`
    /// resolving the ellipsis of a run's own font and the style's ellipsis
    /// standing in where it resolves none.
    ///
    /// The style's ellipsis is laid out the first time a cut needs it and
    /// kept with the shaped text, so later cuts and re-breaks reuse it.
    pub(crate) fn truncate_resolving(
        &mut self,
        max_lines: Option<u32>,
        width: Option<f32>,
        style: &TextStyle,
        own: &mut dyn FnMut(&parley::Run<'_, ()>) -> Option<Ellipsis>,
    ) {
        let mut fallback = self.fallback_ellipsis.take();
        self.truncate_with(max_lines, width, &mut |run| {
            own(run).or_else(|| {
                fallback
                    .get_or_insert_with(|| style_ellipsis(style))
                    .clone()
            })
        });
        self.fallback_ellipsis = fallback;
    }

    /// Cuts the layout as [`TextLayout::truncate`] does, with the ellipsis
    /// that `ellipsis_for` resolves for a run's font, size, synthesis and
    /// instance, or none when it returns `None`.
    pub(crate) fn truncate_with(
        &mut self,
        max_lines: Option<u32>,
        width: Option<f32>,
        ellipsis_for: &mut dyn FnMut(&parley::Run<'_, ()>) -> Option<Ellipsis>,
    ) {
        self.truncation = None;
        let Some(max_lines) = max_lines else {
            return;
        };
        let kept_lines = (max_lines as usize).min(self.layout.len());
        if kept_lines == 0 {
            if !self.layout.is_empty() {
                self.truncation = Some(Truncation {
                    kept_lines: 0,
                    cut: None,
                    width: 0.0,
                    height: 0.0,
                });
            }
            return;
        }
        let Some(line) = self.layout.get(kept_lines - 1) else {
            return;
        };
        let overflows = width.is_some_and(|width| line_width(&line) > width);
        if self.layout.len() == kept_lines && !overflows {
            return;
        }
        let height = self.kept_height(kept_lines);
        if self.layout.is_rtl() {
            self.truncation = Some(Truncation {
                kept_lines,
                cut: None,
                width: self.kept_width(kept_lines),
                height,
            });
            return;
        }
        let Some((cut, end)) = cut_line(&line, width.unwrap_or(f32::INFINITY), ellipsis_for) else {
            // The cut line has no glyph runs, so there is nothing to cut and
            // no ellipsis to place; the lines after it still go.
            self.truncation = Some(Truncation {
                kept_lines,
                cut: None,
                width: self.kept_width(kept_lines),
                height,
            });
            return;
        };
        // The cut line reports the clusters it keeps plus the ellipsis, so
        // the lines before it alone decide the widest kept line.
        self.truncation = Some(Truncation {
            kept_lines,
            cut: Some(cut),
            width: self.kept_width(kept_lines - 1).max(end),
            height,
        });
    }

    /// Returns the widest advance of the first `kept_lines` lines, without
    /// the whitespace that hangs past a line's end, which is the measure
    /// [`parley::Layout::width`] reports.
    fn kept_width(&self, kept_lines: usize) -> f32 {
        self.layout
            .lines()
            .take(kept_lines)
            .map(|line| line_width(&line))
            .fold(0.0f32, f32::max)
    }

    /// Returns the summed line height of the first `kept_lines` lines.
    fn kept_height(&self, kept_lines: usize) -> f32 {
        self.layout
            .lines()
            .take(kept_lines)
            .map(|line| line.metrics().line_height)
            .sum()
    }

    /// Returns the width and height of the laid out text in logical pixels.
    ///
    /// The width is the longest line, without hanging whitespace. The height
    /// is the sum of the line heights. A layout cut by truncation reports
    /// the kept lines and the cut line's glyphs plus its ellipsis, so
    /// callers measure what paint draws.
    pub fn size(&self) -> Size {
        match &self.truncation {
            Some(truncation) => Size {
                width: truncation.width,
                height: truncation.height,
            },
            None => Size {
                width: self.layout.width(),
                height: self.layout.height(),
            },
        }
    }

    /// Returns the number of lines the text was broken into.
    ///
    /// A layout cut by truncation reports the lines it keeps.
    pub fn line_count(&self) -> usize {
        match &self.truncation {
            Some(truncation) => truncation.kept_lines,
            None => self.layout.len(),
        }
    }

    /// Iterates the glyph runs of every line, in visual order.
    ///
    /// A glyph run carries its font, size and positioned glyphs, and no color:
    /// the caller supplies it when converting runs with `glyph_runs_at`. The
    /// item type is parley's, and its coordinates are relative to the top
    /// left of the layout. The runs of lines a truncation dropped are not
    /// yielded.
    pub fn glyph_runs(&self) -> impl Iterator<Item = parley::GlyphRun<'_, ()>> + Clone + '_ {
        let kept_lines = self
            .truncation
            .as_ref()
            .map_or(usize::MAX, |truncation| truncation.kept_lines);
        self.layout.lines().take(kept_lines).flat_map(|line| {
            line.items().filter_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => Some(run),
                PositionedLayoutItem::InlineBox(_) => None,
            })
        })
    }

    /// Returns the rectangle under every glyph run of every kept line, in
    /// logical pixels offset by `origin`, for a widget that draws an
    /// underline beneath laid-out text.
    ///
    /// Each rectangle sits at the font's own underline offset and carries
    /// the font's underline thickness, at least one logical pixel. The
    /// whitespace a wrapped line ends with hangs past its end edge and is
    /// not underlined, and on the line a truncation cut, the underline ends
    /// where the ellipsis begins and covers only the runs that survive.
    pub fn underlines(&self, origin: Point) -> Vec<Rect> {
        let truncation = self.truncation.as_ref();
        let kept_lines = truncation.map_or(usize::MAX, |truncation| truncation.kept_lines);
        let rtl = self.layout.is_rtl();
        let mut underlines = Vec::new();
        for (line_index, line) in self.layout.lines().enumerate().take(kept_lines) {
            let cut = truncation
                .filter(|_| line_index + 1 == kept_lines)
                .and_then(|truncation| truncation.cut.as_ref());
            let hanging = line.metrics().hanging_advance;
            let items = line.items().count();
            let mut run_index = 0usize;
            for (at, item) in line.items().enumerate() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                // `Cut.run` counts the glyph runs of the line alone, the way
                // the paint conversion does, while the trim indices count
                // every item. No layout carries an inline box today, so the
                // two countings agree.
                if cut.is_some_and(|cut| run_index > cut.run) {
                    break;
                }
                let cut_run = cut.is_some_and(|cut| run_index == cut.run);
                run_index += 1;
                let metrics = run.run().font_metrics();
                let top = origin.y + run.baseline() - metrics.underline_offset;
                let bottom = top + metrics.underline_size.max(1.0);
                let mut left = origin.x + run.offset();
                let mut right = left + run.advance();
                trim_hanging_whitespace(rtl, at, items, hanging, &mut left, &mut right);
                if let Some(cut) = cut
                    && cut_run
                {
                    right = right.min(origin.x + cut.x);
                }
                if right > left {
                    underlines.push(Rect::new(left, top, right, bottom));
                }
            }
        }
        underlines
    }
}

/// Returns the advance of `line` without the whitespace that hangs past
/// its end, which is the measure [`parley::Layout::width`] reports.
fn line_width(line: &parley::Line<'_, ()>) -> f32 {
    let metrics = line.metrics();
    metrics.advance - metrics.hanging_advance
}

/// Trims the whitespace a wrapped line hangs past its end edge from the
/// edges of the run `at` of `items`, in a line that reads `rtl` or not. The
/// underline painters call this for every run, so keep their callers in
/// [`TextLayout::underlines`] and [`crate::glyphs::rich_glyph_runs`] in step
/// through this one helper.
pub(crate) fn trim_hanging_whitespace(
    rtl: bool,
    at: usize,
    items: usize,
    hanging: f32,
    left: &mut f32,
    right: &mut f32,
) {
    if rtl && at == 0 {
        *left = (*left + hanging).min(*right);
    } else if !rtl && at + 1 == items {
        *right = (*right - hanging).max(*left);
    }
}

/// Finds the longest prefix of whole clusters of `line` that fits `width`
/// together with the ellipsis of the run it ends in, and returns the cut
/// with the pen position after its ellipsis. Returns `None` when the line
/// has no glyph runs.
///
/// A cluster run together with the ligature continuations that share its
/// glyphs forms one unit, so the cut never splits a cluster or a ligature.
/// A prefix that ends in whitespace is not a cut, so the ellipsis never
/// follows a space or a line break. The empty prefix always is one: an
/// ellipsis wider than the width stands alone.
///
/// A label's text has one style, so each glyph run covers its whole run and
/// the run's clusters account for every glyph it paints.
fn cut_line(
    line: &parley::Line<'_, ()>,
    width: f32,
    ellipsis_for: &mut dyn FnMut(&parley::Run<'_, ()>) -> Option<Ellipsis>,
) -> Option<(Cut, f32)> {
    let mut best: Option<Candidate> = None;
    let mut best_ellipsis: Option<Ellipsis> = None;
    let mut spent = 0.0f32;
    let mut run_index = 0usize;
    'line: for item in line.items() {
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            continue;
        };
        let run = glyph_run.run();
        let ellipsis = ellipsis_for(run);
        let dots = ellipsis.as_ref().map_or(0.0, Ellipsis::width);
        if best.is_none() {
            best = Some(Candidate {
                run: run_index,
                glyphs: 0,
                x: glyph_run.offset(),
                end: dots,
            });
            best_ellipsis.clone_from(&ellipsis);
        }
        let run_start = spent;
        let rtl = run.is_rtl();
        let mut glyphs = 0usize;
        let mut unit_glyphs = 0usize;
        let mut unit_advance = 0.0f32;
        let mut unit_blank = true;
        let mut clusters = run.visual_clusters().peekable();
        while let Some(cluster) = clusters.next() {
            unit_glyphs += cluster.glyphs().count();
            unit_advance += cluster.advance();
            unit_blank &= cluster.is_space_or_nbsp() || cluster.is_hard_line_break();
            // In visual order a left-to-right ligature starts its unit and a
            // right-to-left one ends it.
            let complete = if rtl {
                !cluster.is_ligature_continuation()
            } else {
                clusters
                    .peek()
                    .is_none_or(|next| !next.is_ligature_continuation())
            };
            if !complete {
                continue;
            }
            if spent + unit_advance > width {
                break 'line;
            }
            spent += unit_advance;
            glyphs += unit_glyphs;
            if !unit_blank && spent + dots <= width {
                if best.as_ref().is_some_and(|best| best.run != run_index) {
                    best_ellipsis.clone_from(&ellipsis);
                }
                best = Some(Candidate {
                    run: run_index,
                    glyphs,
                    x: glyph_run.offset() + (spent - run_start),
                    end: spent + dots,
                });
            }
            unit_glyphs = 0;
            unit_advance = 0.0;
            unit_blank = true;
        }
        run_index += 1;
    }
    let best = best?;
    Some((
        Cut {
            run: best.run,
            glyphs: best.glyphs,
            x: best.x,
            baseline: line.metrics().baseline,
            ellipsis: best_ellipsis,
        },
        best.end,
    ))
}

/// Resolves the ellipsis of `style`'s primary font by laying the ellipsis
/// out with the style, or `None` when the font the style resolves it to
/// maps neither an ellipsis nor a full stop.
pub(crate) fn style_ellipsis(style: &TextStyle) -> Option<Ellipsis> {
    let layout = TextLayout::new("\u{2026}", style, None);
    let run = layout.glyph_runs().next()?;
    ellipsis_glyph(run.run())
}

#[cfg(test)]
thread_local! {
    /// How many times this thread shaped text, which tests read to tell a
    /// re-break from a reshape.
    pub(crate) static SHAPES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The font style one span of a [`RichLayout`] sets over the base style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SpanFont {
    /// The weight, or `None` for the base style's.
    pub(crate) weight: Option<parley::FontWeight>,
    /// Whether the span is set in the family's italic face.
    pub(crate) italic: bool,
    /// Whether the span is set in the generic monospace family.
    pub(crate) monospace: bool,
    /// Whether the span is underlined, which a link is.
    pub(crate) underline: bool,
}

/// A block of text whose spans each set their own font style, laid out in
/// logical pixels.
///
/// Every glyph run carries the index of the span it belongs to as its brush,
/// so parley splits the runs where a span ends, even between two spans that
/// differ only in color, and paint colors each run by its span. Like
/// [`TextLayout`], it carries no color itself.
pub(crate) struct RichLayout {
    /// The shaped and line-broken text.
    pub(crate) layout: parley::Layout<u32>,
}

impl RichLayout {
    /// Lays out `text` with `base`, giving the byte range of each span the
    /// font style it sets, and wraps lines at `max_width` logical pixels, or
    /// not at all when it is `None`.
    ///
    /// The spans' ranges must lie on character boundaries of `text`.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`TextSystem::with`], in particular
    /// when called from inside another `with` call on the same thread.
    pub(crate) fn new(
        text: &str,
        base: &TextStyle,
        spans: &[(Range<usize>, SpanFont)],
        max_width: Option<f32>,
    ) -> RichLayout {
        TextSystem::with(|system| {
            let mut builder =
                system
                    .rich_layouts
                    .ranged_builder(&mut system.fonts, text, 1.0, false);
            builder.push_default(StyleProperty::FontFamily(base.family.clone()));
            builder.push_default(StyleProperty::FontSize(base.size));
            builder.push_default(StyleProperty::FontWeight(base.weight));
            builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
                base.line_height,
            )));
            for (index, (range, font)) in spans.iter().enumerate() {
                let brush = u32::try_from(index).unwrap_or(u32::MAX);
                builder.push(StyleProperty::Brush(brush), range.clone());
                if let Some(weight) = font.weight {
                    builder.push(StyleProperty::FontWeight(weight), range.clone());
                }
                if font.italic {
                    builder.push(
                        StyleProperty::FontStyle(parley::FontStyle::Italic),
                        range.clone(),
                    );
                }
                if font.monospace {
                    builder.push(
                        StyleProperty::FontFamily(parley::FontFamily::from(
                            parley::GenericFamily::Monospace,
                        )),
                        range.clone(),
                    );
                }
                if font.underline {
                    builder.push(StyleProperty::Underline(true), range.clone());
                }
            }
            let mut layout = builder.build(text);
            layout.break_all_lines(max_width);
            RichLayout { layout }
        })
    }

    /// Re-wraps the laid out text at `max_width` without shaping it again.
    pub(crate) fn rebreak(&mut self, max_width: Option<f32>) {
        self.layout.break_all_lines(max_width);
    }

    /// Returns the size of the laid out text in logical pixels.
    pub(crate) fn size(&self) -> Size {
        Size {
            width: self.layout.width(),
            height: self.layout.height(),
        }
    }

    /// Returns the span under the point `(x, y)`, relative to the layout's
    /// top left, or `None` when the point is over no glyph.
    ///
    /// parley maps a point above the first line to that line, and one below
    /// the last line to that one, so the point is checked against the text's
    /// height first.
    pub(crate) fn span_at(&self, x: f32, y: f32) -> Option<usize> {
        if !(0.0..self.layout.height()).contains(&y) {
            return None;
        }
        let (cluster, _) = parley::Cluster::from_point_exact(&self.layout, x, y)?;
        Some(cluster.style().brush as usize)
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
