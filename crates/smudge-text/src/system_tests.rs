//! Behavior tests for the per-thread text system.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests. The one test that
//! reads the host's fonts says so in its name and gates on them.

use crate::layout::TextLayout;
use crate::style::{TextStyle, current_text_style, provide_text_style};
use crate::system::TextSystem;
use parley::FontFamily;
use parley::FontWeight;
use parley::GenericFamily;
use smudge_core::geometry::Color;
use smudge_core::geometry::Point;
use smudge_core::paint::FontBlob;
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

/// The font the first run of `layout` paints with, or `None` when it paints
/// no runs.
fn first_font(layout: &TextLayout) -> Option<FontBlob> {
    layout
        .glyph_runs_at(Point { x: 0.0, y: 0.0 }, Color::BLACK)
        .into_iter()
        .next()
        .map(|run| run.font)
}

/// A style that names the generic system interface family at `weight`.
fn system_ui(weight: FontWeight) -> TextStyle {
    TextStyle {
        family: FontFamily::from(GenericFamily::SystemUi),
        weight,
        ..TextStyle::default()
    }
}

/// Without system fonts, SystemUi resolves to the bundled sans, in both of
/// its weights, as the default style does.
#[cfg(feature = "bundled-fonts")]
#[test]
fn system_ui_falls_back_to_the_bundled_sans() {
    let _owner = setup();
    for weight in [FontWeight::NORMAL, FontWeight::BOLD] {
        let bundled = TextStyle {
            weight,
            ..TextStyle::default()
        };
        let expected = first_font(&TextLayout::new("Smudge", &bundled, None));
        assert!(expected.is_some(), "the bundled sans lays text out");
        assert_eq!(
            first_font(&TextLayout::new("Smudge", &system_ui(weight), None)),
            expected,
            "SystemUi at {weight:?} resolves to the bundled sans"
        );
    }
}

/// Whether `font` has variation axes, so that one file carries every
/// weight.
#[cfg(feature = "system-fonts")]
fn is_variable(font: &FontBlob) -> bool {
    use skrifa::MetadataProvider;
    let data: &[u8] = (*font.data).as_ref();
    skrifa::FontRef::from_index(data, font.index).is_ok_and(|font| !font.axes().is_empty())
}

/// A machine with fonts of its own resolves SystemUi to them, and the
/// weights it carries pick the faces that carry them, with the bundled sans
/// staying the last resort.
///
/// A machine where SystemUi resolves to the bundled sans has nothing to
/// check, so the test leaves early there. So does one where SystemUi is a
/// variable font, whose weights all come from one file and draw at its
/// default instance.
#[cfg(feature = "system-fonts")]
#[test]
fn system_ui_resolves_to_the_system_faces_that_carry_its_weights() {
    TextSystem::with_system_fonts().install();

    let picks = |weight| first_font(&TextLayout::new("Smudge", &system_ui(weight), None));
    let bundled = first_font(&TextLayout::new("Smudge", &TextStyle::default(), None));

    let Some(regular) = picks(FontWeight::NORMAL) else {
        return;
    };
    let Some(bundled) = bundled else {
        return;
    };
    if regular == bundled {
        return;
    }

    let medium = picks(FontWeight::MEDIUM);
    let semi_bold = picks(FontWeight::SEMI_BOLD);
    let bold = picks(FontWeight::BOLD);

    if bold.as_ref() == Some(&regular) && is_variable(&regular) {
        eprintln!(
            "skipped: SystemUi resolves to a variable font, which carries every weight in one \
             file and draws at its default instance"
        );
        return;
    }

    assert_ne!(
        bold.as_ref(),
        Some(&bundled),
        "SystemUi answers bold with a system face"
    );
    assert_ne!(
        bold.as_ref(),
        Some(&regular),
        "the system resolves the weights apart"
    );
    assert_eq!(
        picks(FontWeight::NORMAL).as_ref(),
        Some(&regular),
        "a weight is stable"
    );
    // Where the system carries a face for a lighter weight, it is a system
    // face too, and not the bundled fallback.
    if medium.as_ref() != Some(&regular) {
        assert_ne!(
            medium.as_ref(),
            Some(&bundled),
            "the medium face is the system's"
        );
    }
    if semi_bold != bold {
        assert_ne!(
            semi_bold.as_ref(),
            Some(&bundled),
            "the semibold face is the system's"
        );
    }
}
