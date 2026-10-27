//! Behavior tests for the label's measure cache.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests.

use smudge_core::geometry::{Rect, Size};
use smudge_core::paint::PaintOp;
use smudge_core::style::Style;
use smudge_core::theme::Theme;
use smudge_core::widget::{AvailableSpace, MeasureInput, PaintCx, Widget};

use crate::label::{Label, set_text, set_text_size};
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
