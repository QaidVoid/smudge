//! Behavior tests for the label's measure cache.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests.

use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::paint::PaintOp;
use smudge_core::style::Style;
use smudge_core::theme::Theme;
use smudge_core::widget::{AvailableSpace, MeasureInput, PaintCx, Widget};

use crate::label::{Label, set_max_lines, set_text, set_text_size};
use crate::layout::{SHAPES, TextLayout};
use crate::style::TextStyle;
use crate::system::TextSystem;

/// Text that wraps onto several lines at the widths the tests offer.
const TEXT: &str = "several words that wrap onto more than one line";

/// Returns a measure call that offers `width` and no fixed size.
fn offer(width: AvailableSpace) -> MeasureInput {
    MeasureInput {
        known_width: None,
        known_height: None,
        available_width: width,
        available_height: AvailableSpace::MaxContent,
        scale: 1.0,
    }
}

/// Returns the glyph ids and positions `label` paints, in paint order.
fn painted_glyphs(label: &Label) -> Vec<(u32, f32, f32)> {
    let style = Style::default();
    let theme = Theme::light();
    let rect = Rect::new(0.0, 0.0, 400.0, 200.0);
    let cx = PaintCx {
        rect,
        content: rect,
        style: &style,
        theme: &theme,
    };
    let mut ops = Vec::new();
    label.paint(&cx, &mut ops);
    ops.iter()
        .filter_map(|op| match op {
            PaintOp::Glyphs(run) => Some(run),
            _ => None,
        })
        .flat_map(|run| run.glyphs.iter().map(|glyph| (glyph.id, glyph.x, glyph.y)))
        .collect()
}

#[test]
fn a_min_content_measure_leaves_the_painted_lines_alone() {
    TextSystem::bundled_only().install();
    let mut measured = Label::new(TEXT.to_owned());
    let size = measured.measure(offer(AvailableSpace::Definite(150.0)));
    measured.measure(offer(AvailableSpace::MinContent));
    let mut reference = Label::new(TEXT.to_owned());
    let reference_size = reference.measure(offer(AvailableSpace::Definite(150.0)));

    assert_eq!(size, reference_size);
    assert_eq!(painted_glyphs(&measured), painted_glyphs(&reference));
}

#[test]
fn measuring_at_several_widths_paints_the_last_one() {
    TextSystem::bundled_only().install();
    let mut measured = Label::new(TEXT.to_owned());
    for width in [
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(60.0),
        AvailableSpace::MinContent,
        AvailableSpace::Definite(300.0),
        AvailableSpace::Definite(120.0),
    ] {
        measured.measure(offer(width));
    }
    let mut reference = Label::new(TEXT.to_owned());
    reference.measure(offer(AvailableSpace::Definite(120.0)));

    assert_eq!(painted_glyphs(&measured), painted_glyphs(&reference));
}

/// Measures `label` the way a layout pass does, at a definite width, at min
/// content and at the definite width again, and returns both answers.
fn measure_pass(label: &mut Label) -> (Option<Size>, Option<Size>) {
    label.measure(offer(AvailableSpace::Definite(150.0)));
    let min = label.measure(offer(AvailableSpace::MinContent));
    let size = label.measure(offer(AvailableSpace::Definite(150.0)));
    (size, min)
}

#[test]
fn a_text_change_measures_and_paints_the_new_text() {
    TextSystem::bundled_only().install();
    let mut changed = Label::new(TEXT.to_owned());
    measure_pass(&mut changed);
    set_text(
        &mut changed,
        String::from("a different and much shorter text"),
    );
    let answers = measure_pass(&mut changed);
    let mut fresh = Label::new(String::from("a different and much shorter text"));
    let fresh_answers = measure_pass(&mut fresh);

    assert_eq!(answers, fresh_answers);
    assert_eq!(painted_glyphs(&changed), painted_glyphs(&fresh));
}

#[test]
fn a_style_change_measures_and_paints_the_new_style() {
    TextSystem::bundled_only().install();
    let mut changed = Label::new(TEXT.to_owned());
    measure_pass(&mut changed);
    set_text_size(&mut changed, 22.0);
    let answers = measure_pass(&mut changed);
    let mut fresh = Label::new(TEXT.to_owned());
    set_text_size(&mut fresh, 22.0);
    let fresh_answers = measure_pass(&mut fresh);

    assert_eq!(answers, fresh_answers);
    assert_eq!(painted_glyphs(&changed), painted_glyphs(&fresh));
}

/// The glyph identifier the font maps the horizontal ellipsis to, shaped
/// with the same default style the tests lay out with.
fn ellipsis_glyph_id() -> u32 {
    let style = TextStyle::default();
    let ellipsis = TextLayout::new("\u{2026}", &style, None);
    let runs = ellipsis.glyph_runs_at(Point { x: 0.0, y: 0.0 }, Color::BLACK);
    runs[0].glyphs[0].id
}

/// A label capped to one line paints the ellipsis after the kept glyphs,
/// reports a single line's height, and still fits the offered width.
#[test]
fn a_cap_of_one_line_paints_the_ellipsis_and_reports_one_line() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(1));
    let (size, _) = measure_pass(&mut capped);
    let size = size.expect("the label measures at a definite width");

    let glyphs = painted_glyphs(&capped);
    assert!(!glyphs.is_empty());
    assert_eq!(glyphs.last().unwrap().0, ellipsis_glyph_id());
    // A single line's height is exact: parley accumulates the height in f64
    // and this reports one line, so a fresh one-line layout matches bit for
    // bit.
    let one_line = TextLayout::new("one line", &TextStyle::default(), None).size();
    assert_eq!(size.height, one_line.height);
    assert!(size.width <= 150.0);
}

/// The accessible value keeps the full text when the label is capped.
#[test]
fn the_accessible_value_keeps_the_full_text_when_capped() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(1));
    measure_pass(&mut capped);

    let semantics = capped.semantics().expect("the label is accessible");
    assert_eq!(semantics.value.as_deref(), Some(TEXT));
}

/// A cap without a width limit leaves the single line whole, since nothing
/// is dropped and no ellipsis is cut.
#[test]
fn a_cap_without_a_width_limit_shows_the_whole_line() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(1));
    let size = capped
        .measure(offer(AvailableSpace::MaxContent))
        .expect("the label measures at max content");

    let mut whole = Label::new(TEXT.to_owned());
    let whole_size = whole
        .measure(offer(AvailableSpace::MaxContent))
        .expect("the label measures at max content");

    assert_eq!(size, whole_size);
    assert_eq!(painted_glyphs(&capped), painted_glyphs(&whole));
}

/// Raising the cap lays the dropped lines back in.
#[test]
fn raising_the_cap_relays_the_label_out() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(1));
    let (capped_size, _) = measure_pass(&mut capped);
    set_max_lines(&mut capped, None);
    let (raised_size, _) = measure_pass(&mut capped);
    let mut whole = Label::new(TEXT.to_owned());
    let (whole_size, _) = measure_pass(&mut whole);

    assert!(capped_size.unwrap().height < whole_size.unwrap().height);
    assert_eq!(raised_size, whole_size);
    assert_eq!(painted_glyphs(&capped), painted_glyphs(&whole));
}

/// A cap of zero measures nothing and paints nothing.
#[test]
fn a_cap_of_zero_reports_an_empty_label() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(0));
    let (size, _) = measure_pass(&mut capped);
    let size = size.expect("the label measures at a definite width");

    assert_eq!(size.height, 0.0);
    assert!(painted_glyphs(&capped).is_empty());
}

/// A cap of two or more keeps the full text's min-content height, since
/// min-content answers what the text needs rather than what the cap shows.
#[test]
fn a_capped_labels_min_content_keeps_the_full_height() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(2));
    let capped_min = capped.measure(offer(AvailableSpace::MinContent));
    let mut whole = Label::new(TEXT.to_owned());
    let whole_min = whole.measure(offer(AvailableSpace::MinContent));

    assert_eq!(capped_min, whole_min);
    let (shown, _) = measure_pass(&mut capped);
    assert!(shown.unwrap().height < whole_min.unwrap().height);
}

/// A label capped to one line can narrow to its ellipsis, so a row can
/// shrink it past its longest word, and reports one line of height.
#[test]
fn a_one_line_cap_narrows_to_about_one_ellipsis() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(String::from("extraordinarily wide words"));
    set_max_lines(&mut capped, Some(1));
    let min = capped
        .measure(offer(AvailableSpace::MinContent))
        .expect("the label measures at min content");

    let style = TextStyle::default();
    let ellipsis = TextLayout::new("\u{2026}", &style, None).size();
    assert!(min.width > 0.0);
    assert!(
        min.width <= ellipsis.width + 0.01,
        "min-content width {} is wider than one ellipsis, {}",
        min.width,
        ellipsis.width
    );
    assert_eq!(min.height, ellipsis.height);
}

/// Narrowing a capped label and changing its cap re-break the shaped text
/// and never shape it again.
#[test]
fn narrowing_or_recapping_a_label_never_reshapes_it() {
    TextSystem::bundled_only().install();
    let mut capped = Label::new(TEXT.to_owned());
    set_max_lines(&mut capped, Some(1));
    measure_pass(&mut capped);

    let shapes = SHAPES.with(|shapes| shapes.get());
    for width in [120.0, 90.0, 60.0] {
        capped.measure(offer(AvailableSpace::Definite(width)));
    }
    capped.measure(offer(AvailableSpace::MinContent));
    set_max_lines(&mut capped, Some(2));
    measure_pass(&mut capped);
    set_max_lines(&mut capped, None);
    measure_pass(&mut capped);
    set_max_lines(&mut capped, Some(1));
    let answers = measure_pass(&mut capped);
    assert_eq!(
        SHAPES.with(|shapes| shapes.get()),
        shapes,
        "nothing reshaped"
    );

    let mut fresh = Label::new(TEXT.to_owned());
    set_max_lines(&mut fresh, Some(1));
    let fresh_answers = measure_pass(&mut fresh);
    assert_eq!(answers, fresh_answers);
    assert_eq!(painted_glyphs(&capped), painted_glyphs(&fresh));
}
