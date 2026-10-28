//! Behavior tests for the laid out text block.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests.

use crate::layout::TextLayout;
use crate::style::TextStyle;
use crate::system::TextSystem;
use proptest::prelude::*;
use smudge_core::geometry::Point;

/// Installs a bundled-only system on this test thread.
fn setup() {
    TextSystem::bundled_only().install();
}

/// Returns the positioned glyphs of every run, in paint order.
fn glyph_positions(layout: &TextLayout) -> Vec<(u32, f32, f32)> {
    let origin = Point { x: 10.0, y: 20.0 };
    let mut positions = Vec::new();
    for run in layout.glyph_runs_at(origin, smudge_core::geometry::Color::BLACK) {
        for glyph in &run.glyphs {
            positions.push((glyph.id, glyph.x, glyph.y));
        }
    }
    positions
}

/// A layout that was broken at one width and re-broken at another must equal
/// a layout that was shaped once and broken straight at the second width.
#[test]
fn rebreaking_at_another_width_matches_a_fresh_layout() {
    setup();
    let style = TextStyle::default();
    let text = "The quick brown fox jumps over the lazy dog and keeps \
                running until the sentence wraps onto several lines.";

    let mut rebroken = TextLayout::new(text, &style, Some(220.0));
    rebroken.rebreak(Some(90.0));
    let fresh = TextLayout::new(text, &style, Some(90.0));

    assert_eq!(rebroken.size(), fresh.size());
    assert_eq!(rebroken.line_count(), fresh.line_count());
    assert_eq!(glyph_positions(&rebroken), glyph_positions(&fresh));
}

/// Re-breaking must also reach max content, the single-line layout, without
/// reshaping.
#[test]
fn rebreaking_at_max_content_matches_a_fresh_layout() {
    setup();
    let style = TextStyle::default();
    let text = "words at several widths";

    let mut rebroken = TextLayout::new(text, &style, Some(40.0));
    rebroken.rebreak(None);
    let fresh = TextLayout::new(text, &style, None);

    assert_eq!(rebroken.size(), fresh.size());
    assert_eq!(rebroken.line_count(), fresh.line_count(), "one line");
    assert_eq!(glyph_positions(&rebroken), glyph_positions(&fresh));
}

/// Any width sequence must leave the layout where shaping once and breaking
/// straight at the last width would have left it.
#[test]
fn any_rebreak_sequence_matches_a_fresh_layout() {
    setup();
    let style = TextStyle::default();
    let text = "several words of text that wrap at every width the engine \
                could offer, some of them narrow and some of them wide";

    proptest!(|(widths in proptest::collection::vec(1.0..300.0f32, 1..6))| {
        let mut rebroken = TextLayout::new(text, &style, None);
        for width in &widths {
            rebroken.rebreak(Some(*width));
        }
        let last = widths[widths.len() - 1];
        let fresh = TextLayout::new(text, &style, Some(last));

        prop_assert_eq!(rebroken.size(), fresh.size());
        prop_assert_eq!(rebroken.line_count(), fresh.line_count());
        prop_assert_eq!(glyph_positions(&rebroken), glyph_positions(&fresh));
    });
}

/// Thai puts no spaces between words, so it can only wrap where the break
/// dictionary finds a word boundary.
#[cfg(feature = "complex-scripts")]
#[test]
fn thai_without_spaces_wraps_between_words() {
    setup();
    let style = TextStyle::default();
    let text = "ภาษาไทยเป็นภาษาที่ไม่มีการเว้นวรรคระหว่างคำ";
    let width = TextLayout::new(text, &style, None).size().width;

    let wrapped = TextLayout::new(text, &style, Some(width / 3.0));

    assert!(wrapped.line_count() > 1);
    assert!(wrapped.size().width <= width / 3.0);
}
