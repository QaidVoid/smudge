//! Behavior tests for the per-thread text system.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests.

use crate::layout::TextLayout;
use crate::style::{TextStyle, current_text_style, provide_text_style};
use crate::system::TextSystem;
use smudge_core::reactive::Owner;

/// Installs a bundled-only system on this test thread and makes a new owner
/// current, mirroring what the frame loop provides.
fn setup() -> Owner {
    TextSystem::bundled_only().install();
    let owner = Owner::new();
    owner.set();
    owner
}

#[test]
fn bundled_layout_is_identical_across_runs() {
    let _owner = setup();

    let style = TextStyle::default();
    let text = "The quick brown fox jumps over the lazy dog and keeps \
                running until the sentence wraps onto several lines.";
    let first = TextLayout::new(text, &style, Some(200.0));
    let second = TextLayout::new(text, &style, Some(200.0));

    assert_eq!(
        first.size(),
        second.size(),
        "the same text and style must lay out to the same size"
    );
    assert_eq!(first.line_count(), second.line_count());
    assert!(
        first.line_count() > 1,
        "the paragraph must wrap at 200 px, found {} lines",
        first.line_count()
    );

    insta::assert_snapshot!(format!(
        "{} lines, {:.2} x {:.2} px",
        first.line_count(),
        first.size().width,
        first.size().height
    ));
}

#[test]
fn runs_lay_out_in_visual_order_across_directions() {
    let _owner = setup();

    // "abc " is left-to-right, the Hebrew word for peace is right-to-left.
    let text = "abc \u{5e9}\u{5dc}\u{5d5}\u{5dd}";
    let style = TextStyle::default();
    let layout = TextLayout::new(text, &style, None);

    let runs: Vec<_> = layout.glyph_runs().collect();
    assert_eq!(runs.len(), 2, "the direction change splits the line");

    assert!(
        !runs[0].run().is_rtl(),
        "the latin run comes first and runs left to right"
    );
    assert_eq!(runs[0].run().text_range(), 0..4);

    assert!(runs[1].run().is_rtl(), "the hebrew run runs right to left");
    assert_eq!(runs[1].run().text_range(), 4..12);

    assert!(
        runs[1].offset() > runs[0].offset(),
        "runs are ordered along the line, found offsets {} and {}",
        runs[0].offset(),
        runs[1].offset()
    );
    assert_eq!(layout.line_count(), 1);
}

#[test]
fn current_style_defaults_then_follows_the_provider() {
    let _owner = setup();

    assert_eq!(
        current_text_style(),
        TextStyle::default(),
        "without a provider the style is the default"
    );

    let style = TextStyle {
        size: 22.0,
        ..TextStyle::default()
    };
    provide_text_style(style);

    assert_eq!(
        current_text_style().size,
        22.0,
        "the provided style reaches the current owner's descendants"
    );
}
