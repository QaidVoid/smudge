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
