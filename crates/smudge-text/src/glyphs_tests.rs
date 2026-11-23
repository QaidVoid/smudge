//! Behavior tests for converting a layout into glyph runs.

use crate::layout::TextLayout;
use crate::style::TextStyle;
use crate::system::TextSystem;
use smudge_core::geometry::{Color, Point};

/// Installs a bundled-only system, so the runs carry the bundled fonts and no
/// layout depends on the fonts installed on the machine.
fn setup() {
    TextSystem::bundled_only().install();
}

#[test]
fn runs_carry_the_bundled_font_bytes() {
    setup();

    let layout = TextLayout::new("Hand", &TextStyle::default(), None);
    let runs = layout.glyph_runs_at(Point { x: 0.0, y: 0.0 }, Color::BLACK);

    assert!(!runs.is_empty(), "text with letters shapes to a run");
    assert!(
        runs.iter().all(|run| run.font.id == runs[0].font.id),
        "one style of text shapes to one font"
    );
    #[cfg(feature = "bundled-fonts")]
    assert_eq!(
        runs[0].font.data.as_ref().as_ref(),
        crate::fonts::SANS_REGULAR,
        "the default style resolves to the bundled sans face"
    );
}

#[test]
fn positions_are_absolute_and_follow_the_origin() {
    setup();

    let layout = TextLayout::new("Ay", &TextStyle::default(), None);
    let at_origin = layout.glyph_runs_at(Point { x: 0.0, y: 0.0 }, Color::BLACK);
    let moved = layout.glyph_runs_at(Point { x: 30.5, y: 12.25 }, Color::BLACK);

    let mut origin_glyphs = at_origin.into_iter().flat_map(|run| run.glyphs);
    for run in moved {
        for glyph in run.glyphs {
            let local = origin_glyphs.next().expect("both conversions see the text");
            assert_eq!(glyph.id, local.id);
            assert_eq!(glyph.x, local.x + 30.5);
            assert_eq!(glyph.y, local.y + 12.25);
        }
    }
}

#[test]
fn glyphs_claim_ink_and_whitespace_does_not() {
    setup();

    let text = TextLayout::new("Ay", &TextStyle::default(), None);
    let runs: Vec<_> = text.glyph_runs_at(Point { x: 5.0, y: 20.0 }, Color::BLACK);
    assert!(!runs.is_empty());
    for run in runs {
        assert!(
            !run.ink_bounds.is_empty(),
            "letters have outlines, so the run claims ink"
        );
    }

    let spaces = TextLayout::new("   ", &TextStyle::default(), None);
    for run in spaces.glyph_runs_at(Point { x: 5.0, y: 20.0 }, Color::BLACK) {
        assert!(
            run.ink_bounds.is_empty(),
            "a space has no outline, so the run claims no ink"
        );
    }
}

/// Collects the points of a glyph outline, control points included, in font
/// coordinates scaled to the size it was drawn at, where y grows up.
#[derive(Default)]
struct Points(Vec<(f32, f32)>);

impl skrifa::outline::OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.extend([(cx0, cy0), (x, y)]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.extend([(cx0, cy0), (cx1, cy1), (x, y)]);
    }
    fn close(&mut self) {}
}

#[test]
fn a_slanted_bold_run_claims_the_ink_of_its_widened_sheared_outlines() {
    use crate::layout::{RichLayout, SpanFont};
    use skrifa::MetadataProvider;
    use skrifa::instance::{LocationRef, Size};
    use skrifa::outline::DrawSettings;
    use smudge_core::paint::EMBOLDEN_MITER_LIMIT;

    setup();
    let text = "Wolf jumped |/{}";
    let mono = TextStyle {
        family: parley::FontFamily::named("Noto Sans Mono"),
        ..TextStyle::default()
    };
    let font = SpanFont {
        weight: Some(parley::FontWeight::BOLD),
        italic: true,
        monospace: false,
        underline: false,
    };
    let layout = RichLayout::new(text, &mono, &[(0..text.len(), font)], None);
    let (runs, _) = crate::glyphs::rich_glyph_runs(&layout, Point { x: 10.0, y: 10.0 }, &[]);

    assert!(!runs.is_empty());
    for run in &runs {
        assert!(run.embolden > 0.0, "the mono has no bold face");
        assert_eq!(run.skew, 14.0, "the mono has no italic face");
        let face = skrifa::FontRef::from_index(run.font.data.as_ref().as_ref(), run.font.index)
            .expect("the run's font parses");
        let outlines = face.outline_glyphs();
        let slope = run.skew.to_radians().tan();
        let reach = run.embolden * EMBOLDEN_MITER_LIMIT;
        for glyph in &run.glyphs {
            let mut points = Points::default();
            if let Some(outline) = outlines.get(skrifa::GlyphId::new(glyph.id)) {
                outline
                    .draw(
                        DrawSettings::unhinted(Size::new(run.size), LocationRef::default()),
                        &mut points,
                    )
                    .expect("the outline draws");
            }
            for (x, y) in points.0 {
                for (dx, dy) in [
                    (-reach, -reach),
                    (-reach, reach),
                    (reach, -reach),
                    (reach, reach),
                ] {
                    let (x, y) = (x + dx, y + dy);
                    let at = Point {
                        x: glyph.x + x + y * slope,
                        y: glyph.y - y,
                    };
                    let bounds = run.ink_bounds;
                    assert!(
                        at.x >= bounds.x0
                            && at.x <= bounds.x1
                            && at.y >= bounds.y0
                            && at.y <= bounds.y1,
                        "glyph {} reaches {at:?} outside the claimed {bounds:?}",
                        glyph.id
                    );
                }
            }
        }
    }
}

/// A link span of the whole `text`.
fn link_span(text: &str) -> crate::rich_text::Span {
    use std::rc::Rc;

    crate::rich_text::Span {
        text: String::from(text),
        style: crate::rich_text::SpanStyle {
            link: Some(Rc::new(|| {})),
            ..Default::default()
        },
    }
}

/// A wrapped paragraph's link underline ends before the whitespace the line
/// hangs past its end edge, and every run underlines something.
#[test]
fn a_link_span_underlines_its_runs_before_the_hanging_whitespace() {
    setup();

    let span = link_span("the quick brown fox jumps over the lazy dog");
    let style = TextStyle::default();
    let fonts = vec![(
        0..span.text.len(),
        crate::layout::SpanFont {
            underline: true,
            ..Default::default()
        },
    )];
    let width = crate::layout::RichLayout::new(&span.text, &style, &fonts, None)
        .size()
        .width;
    let layout = crate::layout::RichLayout::new(&span.text, &style, &fonts, Some(width / 2.0));
    let origin = Point { x: 3.0, y: 4.0 };

    let (runs, underlines) = super::rich_glyph_runs(&layout, origin, &[]);

    assert!(!runs.is_empty(), "the link span paints glyphs");
    assert!(
        !underlines.is_empty(),
        "a link span underlines the runs it covers"
    );
    assert!(
        underlines.iter().all(|(rect, _)| rect.width() > 0.0),
        "a whitespace-only run leaves no underline"
    );
    let line = layout.layout.lines().next().unwrap();
    let metrics = line.metrics();
    assert!(
        metrics.hanging_advance > 0.0,
        "the first line ends the wrap in whitespace"
    );
    let first_line_y0 = underlines[0].0.y0;
    let widest = underlines
        .iter()
        .filter(|(rect, _)| rect.y0 == first_line_y0)
        .map(|(rect, _)| rect.x1)
        .fold(f32::MIN, f32::max);
    assert!(
        (widest - origin.x - (metrics.advance - metrics.hanging_advance)).abs() < 0.01,
        "the first line's underline ends before the hanging whitespace: {widest}"
    );
}
