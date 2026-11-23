//! Whole-frame behavior of the separator, driven through the headless
//! harness.
//!
//! Every test installs a bundled-only text system first, because the
//! windows show labels, so the layout never depends on the fonts installed
//! on the machine running the tests.

use smudge_core::element::IntoElement;
use smudge_core::geometry::{Rect, Size};
use smudge_core::style::{Align, pct, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};
use smudge_widgets::{column, row, separator};

/// The absolute rectangles of every visible node, in paint order.
fn node_rects(harness: &mut Harness) -> Vec<Rect> {
    let mut rects = Vec::new();
    harness
        .app()
        .visit_nodes(|_depth, _name, rect| rects.push(rect));
    rects
}

/// A window of 200 by 100 whose root panel shows `build`.
fn window<E: IntoElement>(build: impl Fn() -> E + 'static) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().align_items(Align::Start).child(build())
    });
    harness.frame();
    harness
}

#[test]
fn a_separator_in_a_column_runs_horizontally() {
    let mut harness = window(|| {
        column()
            .width(pct(100.0))
            .child(panel().height(px(30.0)))
            .child(separator())
            .child(panel().height(px(30.0)))
    });

    let rects = node_rects(&mut harness);
    assert_eq!(rects.len(), 5, "the column holds three children");
    let line = rects[3];
    assert!(
        (line.height() - 1.0).abs() < 0.01,
        "the line is one logical pixel thick: {line:?}"
    );
    assert!(
        (line.width() - 200.0).abs() < 0.01,
        "the line stretches across the column: {line:?}"
    );
    assert_eq!(line.y0, 30.0, "the line sits between the two sections");
}

#[test]
fn a_separator_in_a_row_runs_vertically() {
    let mut harness = window(|| {
        row()
            .height(px(60.0))
            .child(panel().width(px(40.0)))
            .child(separator())
            .child(panel().width(px(40.0)))
    });

    let rects = node_rects(&mut harness);
    assert_eq!(rects.len(), 5, "the row holds three children");
    let line = rects[3];
    assert!(
        (line.width() - 1.0).abs() < 0.01,
        "the line is one logical pixel thick: {line:?}"
    );
    assert!(
        (line.height() - 60.0).abs() < 0.01,
        "the line stretches down the row: {line:?}"
    );
    assert_eq!(line.x0, 40.0, "the line sits between the two sections");
}

#[test]
fn a_separator_stretches_across_a_start_aligned_column() {
    let mut harness = window(|| {
        column()
            .width(pct(100.0))
            .align_items(Align::Start)
            .child(panel().height(px(30.0)))
            .child(separator())
            .child(panel().height(px(30.0)))
    });

    let rects = node_rects(&mut harness);
    let line = rects[3];
    assert!(
        (line.width() - 200.0).abs() < 0.01,
        "the line stretches across the aligned column: {line:?}"
    );
    assert!(
        (line.height() - 1.0).abs() < 0.01,
        "the line is one logical pixel thick: {line:?}"
    );
}

#[test]
fn a_separator_paints_the_border_color() {
    let harness = window(|| {
        column()
            .width(pct(100.0))
            .child(panel().height(px(30.0)))
            .child(separator())
            .child(panel().height(px(30.0)))
    });

    // The line runs across the whole width at y 30, and the pixels above
    // and below it are the surface behind it.
    for x in [2, 100, 197] {
        assert_eq!(
            harness.pixel(x, 30),
            Some([204, 204, 204, 255]),
            "the line paints the light theme's border color at x {x}"
        );
        assert_ne!(harness.pixel(x, 29), Some([204, 204, 204, 255]));
        assert_ne!(harness.pixel(x, 31), Some([204, 204, 204, 255]));
    }
}

#[test]
fn a_separator_reaches_no_screen_reader() {
    let mut harness = window(|| {
        column()
            .width(pct(100.0))
            .child(label("Name"))
            .child(separator())
            .child(label("Address"))
    });

    let projected = harness.app().semantics_nodes();
    assert_eq!(
        projected.len(),
        2,
        "the two labels project, and the decorative line between them does not"
    );
}

/// A window that shows the separator between the sections of a form, in
/// `theme`, with a second one across a row.
fn states_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 140.0, 1.0, || {
        panel().child(
            column()
                .width(pct(100.0))
                .padding(px(12.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
                .align_items(Align::Start)
                .child(label("Account"))
                .child(separator())
                .child(label("Appearance"))
                .child(separator())
                .child(label("Regional"))
                .child(
                    row()
                        .gap(Size {
                            width: 8.0,
                            height: 0.0,
                        })
                        .child(label("Left"))
                        .child(separator())
                        .child(label("Right")),
                ),
        )
    });
    harness.app().set_theme(theme);
    harness.frame();
    harness
}

#[test]
fn the_states_of_a_separator_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::light()).png());
}

#[test]
fn the_states_of_a_separator_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::dark()).png());
}
