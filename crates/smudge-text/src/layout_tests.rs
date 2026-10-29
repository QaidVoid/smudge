//! Behavior tests for the laid out text block.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests.

use crate::layout::{TextLayout, line_width, style_ellipsis};
use crate::style::TextStyle;
use crate::system::TextSystem;
use proptest::prelude::*;
use smudge_core::geometry::{Point, Size};

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

/// The glyph identifier the bundled font maps the horizontal ellipsis to.
fn ellipsis_glyph_id() -> u32 {
    let style = TextStyle::default();
    glyph_positions(&TextLayout::new("\u{2026}", &style, None))[0].0
}

/// Truncating to one line ends the line in the ellipsis glyph, fits the
/// width it was cut at, and reports the height of a single line.
#[test]
fn one_line_truncation_ends_in_the_ellipsis_glyph() {
    setup();
    let style = TextStyle::default();
    let text = "the quick brown fox jumps over the lazy dog";
    let mut layout = TextLayout::new(text, &style, Some(120.0));
    assert!(layout.line_count() > 1);
    layout.truncate(Some(1), Some(120.0), &style);

    assert_eq!(layout.line_count(), 1);
    assert!(layout.size().width <= 120.0);
    let positions = glyph_positions(&layout);
    assert_eq!(positions.last().unwrap().0, ellipsis_glyph_id());
    assert_eq!(
        layout.size().height,
        TextLayout::new("one line", &style, None).size().height
    );
}

/// The cut keeps the glyphs that fit before the ellipsis rather than the
/// whole line or nothing at all.
///
/// The width leaves room for a whole line of text minus a couple of pixels,
/// so the ellipsis cannot follow the last line's glyphs and the cut has to
/// give some back.
#[test]
fn the_cut_keeps_the_glyphs_that_fit_before_the_ellipsis() {
    setup();
    let style = TextStyle::default();
    let text = "wordthatfits wordthatislong";
    let line_width = TextLayout::new("wordthatfits", &style, None).size().width;
    let width = line_width + 2.0;
    let mut layout = TextLayout::new(text, &style, Some(width));
    assert!(layout.line_count() > 1);
    layout.truncate(Some(1), Some(width), &style);

    let positions = glyph_positions(&layout);
    assert!(positions.len() > 1, "some glyphs survive the cut");
    assert!(positions.len() < 13, "not every glyph survives the cut");
    assert_eq!(positions.last().unwrap().0, ellipsis_glyph_id());
    assert!(layout.size().width <= width);
}

/// Truncating a re-broken layout matches truncating a fresh one, so a
/// narrowed capped label pays for the break and the cut, not for shaping.
#[test]
fn rebreaking_then_truncating_matches_a_fresh_truncated_layout() {
    setup();
    let style = TextStyle::default();
    let text = "several words of text that wrap at narrow widths and need a cut";

    let mut rebroken = TextLayout::new(text, &style, Some(250.0));
    rebroken.rebreak(Some(110.0));
    rebroken.truncate(Some(2), Some(110.0), &style);
    let mut fresh = TextLayout::new(text, &style, Some(110.0));
    fresh.truncate(Some(2), Some(110.0), &style);

    assert_eq!(rebroken.size(), fresh.size());
    assert_eq!(rebroken.line_count(), fresh.line_count());
    assert_eq!(glyph_positions(&rebroken), glyph_positions(&fresh));
}

/// A truncation survives a re-break at another width: the cut is dropped
/// with the lines and a re-applied cut matches a fresh layout again.
#[test]
fn rebreaking_drops_the_cut_and_retruncating_restores_it() {
    setup();
    let style = TextStyle::default();
    let text = "the quick brown fox jumps over the lazy dog";

    let mut cut = TextLayout::new(text, &style, Some(120.0));
    cut.truncate(Some(1), Some(120.0), &style);
    cut.rebreak(Some(200.0));
    assert_eq!(cut.line_count(), cut.layout.len(), "the cut is dropped");

    cut.truncate(Some(1), Some(200.0), &style);
    let mut fresh = TextLayout::new(text, &style, Some(200.0));
    fresh.truncate(Some(1), Some(200.0), &style);
    assert_eq!(cut.size(), fresh.size());
    assert_eq!(glyph_positions(&cut), glyph_positions(&fresh));
}

/// Truncating to four lines reports the height of four lines.
#[test]
fn four_lines_report_four_lines_of_height() {
    setup();
    let style = TextStyle::default();
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let mut layout = TextLayout::new(text, &style, Some(90.0));
    let full = layout.size();
    let lines = layout.line_count();
    assert!(lines >= 5);
    layout.truncate(Some(4), Some(90.0), &style);

    let per_line = full.height / lines as f32;
    assert!((layout.size().height - 4.0 * per_line).abs() < 0.01);
    assert!(layout.size().height < full.height);
}

/// A cap of zero keeps no lines: the size is empty and nothing paints,
/// with or without a width to fit.
#[test]
fn a_cap_of_zero_keeps_nothing() {
    setup();
    let style = TextStyle::default();
    let mut layout = TextLayout::new("some text", &style, Some(120.0));
    layout.truncate(Some(0), Some(120.0), &style);
    assert_eq!(layout.line_count(), 0);
    assert_eq!(
        layout.size(),
        Size {
            width: 0.0,
            height: 0.0
        }
    );
    assert!(glyph_positions(&layout).is_empty());

    let mut unbounded = TextLayout::new("some\ntext", &style, None);
    unbounded.truncate(Some(0), None, &style);
    assert_eq!(unbounded.line_count(), 0);
    assert!(glyph_positions(&unbounded).is_empty());
}

/// The cut line's own width reports no more once the cut shortens it, so a
/// first word wider than the offered width fits after truncation.
#[test]
fn the_cut_shortens_an_overflowing_first_word() {
    setup();
    let style = TextStyle::default();
    let word = TextLayout::new("extraordinarily", &style, None)
        .size()
        .width;
    let width = word * 0.6;
    let mut layout = TextLayout::new("extraordinarily wide", &style, Some(width));
    assert!(layout.line_count() > 1);
    layout.truncate(Some(1), Some(width), &style);

    assert!(layout.size().width <= width);
    let positions = glyph_positions(&layout);
    assert_eq!(positions.last().unwrap().0, ellipsis_glyph_id());
}

/// A line that runs in both directions is cut to the width regardless of
/// which run the cut lands in, and the ellipsis closes the line.
#[test]
fn an_embedded_right_to_left_run_still_fits_the_cut() {
    setup();
    let style = TextStyle::default();
    // The Hebrew word splits the line into two runs, so the cut lands in
    // the second run and the ellipsis hangs off it.
    let text = "abc \u{5e9}\u{5dc}\u{5d5}\u{5dd} defgh";
    let width = TextLayout::new("abc \u{5e9}\u{5dc}\u{5d5}\u{5dd}", &style, None)
        .size()
        .width
        + 2.0;
    let mut layout = TextLayout::new(text, &style, Some(width));
    assert!(layout.line_count() > 1);
    layout.truncate(Some(1), Some(width), &style);

    assert_eq!(layout.line_count(), 1);
    assert!(layout.size().width <= width);
    let positions = glyph_positions(&layout);
    assert_eq!(positions.last().unwrap().0, ellipsis_glyph_id());
}

/// The cut falls back to a narrower ellipsis when giving glyphs back walks
/// the cut into another run, so a glyph the wider ellipsis displaced comes
/// back when the narrower one fits.
///
/// The font list sends the fi ligature to the second font, so the line has
/// two runs and the cut can cross between them.
#[test]
fn the_cut_recovers_glyphs_the_ellipsis_displaced() {
    setup();
    let style = TextStyle {
        family: parley::FontFamily::Source("Noto Sans Mono, Noto Sans".to_owned().into()),
        size: 14.0,
        ..TextStyle::default()
    };
    let text = "abcdefghi\u{fb01}\nmore";
    let width = 84.5;
    let mut layout = TextLayout::new(text, &style, Some(width));
    assert!(layout.line_count() > 1);
    layout.truncate(Some(1), Some(width), &style);

    let positions = glyph_positions(&layout);
    assert_eq!(positions.last().unwrap().0, ellipsis_glyph_id());
    assert!(
        positions.len() >= 10,
        "the kept glyphs survive the cut, found {}",
        positions.len()
    );
    assert!(layout.size().width <= width);
}

/// The rightmost edge of the ink `layout` paints, measured from the
/// layout's left edge.
///
/// Glyphs without ink, such as the whitespace that hangs past a line's end,
/// paint nothing and do not count.
fn painted_extent(layout: &TextLayout) -> f32 {
    use skrifa::MetadataProvider;
    let origin = Point { x: 10.0, y: 20.0 };
    let mut extent = 0.0f32;
    for run in layout.glyph_runs_at(origin, smudge_core::geometry::Color::BLACK) {
        let data: &[u8] = (*run.font.data).as_ref();
        let font = skrifa::FontRef::from_index(data, run.font.index).expect("a valid font");
        let metrics = font.glyph_metrics(
            skrifa::instance::Size::new(run.size),
            skrifa::instance::LocationRef::default(),
        );
        for glyph in &run.glyphs {
            let Some(bounds) = metrics.bounds(skrifa::GlyphId::new(glyph.id)) else {
                continue;
            };
            if bounds.x_max > bounds.x_min {
                extent = extent.max(glyph.x - origin.x + bounds.x_max);
            }
        }
    }
    extent
}

/// The number of distinct baselines `layout` paints glyphs on.
fn painted_lines(layout: &TextLayout) -> usize {
    let mut baselines: Vec<f32> = glyph_positions(layout)
        .iter()
        .map(|glyph| glyph.2)
        .collect();
    baselines.sort_by(f32::total_cmp);
    baselines.dedup();
    baselines.len()
}

/// Words in several scripts, with clusters of more than one glyph, a
/// ligature only the second font of a font list maps, and a right-to-left
/// word, so the cut meets several runs and directions.
const WORDS: &[&str] = &[
    "a",
    "word",
    "naïve",
    "q\u{301}x",
    "\u{1f1f3}\u{1f1f5}\u{1f1ef}\u{1f1f5}",
    "\u{5e9}\u{5dc}\u{5d5}\u{5dd}",
    "\u{fb01}ne",
    "\u{3a9}\u{3bc}\u{3ad}\u{3b3}\u{3b1}",
    "x",
];

/// No truncated layout paints past the width it was cut at or on more
/// lines than its cap, over random texts in several scripts and fonts as
/// well as random widths. Widths stay above the longest word's advance,
/// and inputs where the line breaker overflows a line the cut keeps whole
/// are skipped, since only the last kept line of a left-to-right layout is
/// cut.
#[test]
fn truncated_layouts_never_paint_past_the_width_or_the_cap() {
    setup();
    let styles = [
        TextStyle::default(),
        TextStyle {
            family: parley::FontFamily::Source("Noto Sans Mono, Noto Sans".to_owned().into()),
            ..TextStyle::default()
        },
    ];

    proptest!(|(
        words in proptest::collection::vec(proptest::sample::select(WORDS), 1..14),
        width in 70.0..200.0f32,
        lines in 1u32..4,
        mono in proptest::bool::ANY,
    )| {
        let style = &styles[usize::from(mono)];
        let text = words.join(" ");
        let mut layout = TextLayout::new(&text, style, Some(width));
        // Only the last kept line of a left-to-right layout is cut, so a
        // line the breaker already overflowed elsewhere is not the cut's.
        // An unaligned right-to-left line paints its hanging whitespace at
        // its start, so its whole advance has to fit.
        let kept = (lines as usize).min(layout.line_count());
        let rtl = layout.layout.is_rtl();
        let whole = if rtl { kept } else { kept - 1 };
        prop_assume!(layout.layout.lines().take(whole).all(|line| {
            let painted = if rtl { line.metrics().advance } else { line_width(&line) };
            painted <= width
        }));
        layout.truncate(Some(lines), Some(width), style);
        prop_assert!(
            layout.line_count() <= lines as usize,
            "{} lines reported for a cap of {lines} on {text:?}",
            layout.line_count()
        );
        prop_assert!(
            painted_lines(&layout) <= lines as usize,
            "{} lines painted for a cap of {lines} on {text:?}",
            painted_lines(&layout)
        );
        prop_assert!(
            painted_extent(&layout) <= width + 0.01,
            "{lines} lines at width {width} paint to {} for {text:?}",
            painted_extent(&layout)
        );
        prop_assert!(
            layout.size().width <= width + 0.01,
            "{lines} lines at width {width} report {} for {text:?}",
            layout.size().width
        );
    });
}

/// A right-to-left layout keeps its line cap. It ends without an ellipsis,
/// since the ellipsis would have to lead the line rather than close it.
#[test]
fn a_right_to_left_layout_keeps_its_line_cap() {
    setup();
    let style = TextStyle::default();
    let text = "السريع البني القفز فوق الكلب الكسول";
    let mut layout = TextLayout::new(text, &style, Some(60.0));
    assert!(layout.line_count() > 1);

    layout.truncate(Some(1), Some(60.0), &style);

    assert_eq!(layout.line_count(), 1);
    assert_eq!(painted_lines(&layout), 1);
    assert_eq!(
        layout.size().height,
        TextLayout::new("one line", &style, None).size().height
    );
    assert!(
        glyph_positions(&layout)
            .iter()
            .all(|glyph| glyph.0 != ellipsis_glyph_id()),
        "no ellipsis closes a right-to-left line"
    );
}

/// The cut keeps or drops a grapheme cluster whole. Every flag is one
/// cluster of two glyphs, so a cut that splits one paints an odd number of
/// glyphs before the ellipsis.
#[test]
fn the_cut_never_splits_a_cluster() {
    setup();
    let style = TextStyle::default();
    let flags = "\u{1f1f3}\u{1f1f5}".repeat(16);
    let ellipsis = ellipsis_glyph_id();
    for step in 0..120 {
        let width = 20.0 + step as f32 * 0.75;
        let mut layout = TextLayout::new(&flags, &style, Some(width));
        layout.truncate(Some(1), Some(width), &style);
        let kept = glyph_positions(&layout)
            .iter()
            .filter(|glyph| glyph.0 != ellipsis)
            .count();
        assert_eq!(kept % 2, 0, "a cut at width {width} split a flag");
    }
}

/// A cap without a width limit keeps the whole first line and closes it
/// with the ellipsis, since nothing limits how wide the line may be.
#[test]
fn a_cap_without_a_width_keeps_the_whole_first_line() {
    setup();
    let style = TextStyle::default();
    let mut layout = TextLayout::new("Hi there\nsecond line", &style, None);
    layout.truncate(Some(1), None, &style);

    let mut expected: Vec<u32> = glyph_positions(&TextLayout::new("Hi there", &style, None))
        .iter()
        .map(|glyph| glyph.0)
        .collect();
    expected.push(ellipsis_glyph_id());
    let painted: Vec<u32> = glyph_positions(&layout)
        .iter()
        .map(|glyph| glyph.0)
        .collect();
    assert_eq!(painted, expected);
}

/// A single line wider than the width is cut too, so a capped label with
/// one long word can narrow to its ellipsis.
#[test]
fn a_single_line_wider_than_the_width_is_cut() {
    setup();
    let style = TextStyle::default();
    let word = TextLayout::new("extraordinarily", &style, None)
        .size()
        .width;
    let width = word * 0.5;
    let mut layout = TextLayout::new("extraordinarily", &style, Some(width));
    assert_eq!(layout.line_count(), 1);
    layout.truncate(Some(1), Some(width), &style);

    assert_eq!(
        glyph_positions(&layout).last().unwrap().0,
        ellipsis_glyph_id()
    );
    assert!(painted_extent(&layout) <= width);
    assert!(layout.size().width <= width);
}

/// A cut whose fonts map no ellipsis still keeps the cap: the kept lines
/// end without one and still fit the width.
#[test]
fn a_cut_without_any_ellipsis_still_keeps_the_cap() {
    setup();
    let style = TextStyle::default();
    let text = "the quick brown fox jumps over the lazy dog";
    let mut layout = TextLayout::new(text, &style, Some(120.0));
    assert!(layout.line_count() > 1);
    layout.truncate_with(Some(1), Some(120.0), &mut |_, _| None);

    assert_eq!(layout.line_count(), 1);
    assert_eq!(painted_lines(&layout), 1);
    assert!(painted_extent(&layout) <= 120.0);
    assert!(
        glyph_positions(&layout)
            .iter()
            .all(|glyph| glyph.0 != ellipsis_glyph_id())
    );
}

/// An ellipsis from another font than the cut run's paints in a run of its
/// own, with that font, right after the kept clusters.
#[test]
fn an_ellipsis_from_another_font_paints_with_that_font() {
    setup();
    let style = TextStyle::default();
    let mono = TextStyle {
        family: parley::FontFamily::Source("Noto Sans Mono".to_owned().into()),
        ..TextStyle::default()
    };
    let foreign = style_ellipsis(&mono).expect("the bundled mono maps an ellipsis");
    let text = "the quick brown fox jumps over the lazy dog";
    let mut layout = TextLayout::new(text, &style, Some(120.0));
    layout.truncate_with(Some(1), Some(120.0), &mut |_, _| Some(foreign.clone()));

    let runs = layout.glyph_runs_at(
        Point { x: 10.0, y: 20.0 },
        smudge_core::geometry::Color::BLACK,
    );
    let last = runs.last().expect("the cut line paints");
    let mono_runs = TextLayout::new("\u{2026}", &mono, None).glyph_runs_at(
        Point { x: 0.0, y: 0.0 },
        smudge_core::geometry::Color::BLACK,
    );
    assert_eq!(
        last.font, mono_runs[0].font,
        "the ellipsis paints with its own font"
    );
    assert_eq!(last.glyphs.len(), 1);
    assert_eq!(last.glyphs[0].id, foreign.glyph);
    assert_ne!(runs[0].font, last.font);
    assert!(painted_extent(&layout) <= 120.0);
}

/// The fallback ellipsis comes from the style's primary font.
#[test]
fn the_fallback_ellipsis_comes_from_the_styles_font() {
    setup();
    let ellipsis = style_ellipsis(&TextStyle::default()).expect("the bundled sans maps one");
    assert_eq!(ellipsis.glyph, ellipsis_glyph_id());
    assert_eq!(ellipsis.dots, 1);
}
