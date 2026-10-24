//! Behavior tests for theme tokens.
//!
//! Every test drives the same frame pipeline a window does, through the
//! harness, and checks what the surface shows against a harness that mounts
//! the expected colors plainly.

use smudge_core::element::{Element, IntoElement};
use smudge_core::geometry::{Color, Corners, PixelRect};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, RadiusToken, Theme};
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_test::Harness;

/// The literal background of the panel that no token touches.
const LITERAL: Color = Color::rgb(10, 20, 30);

/// A root with a token-colored panel and a literal-colored panel beside it.
///
/// The layout pass sizes the root to the whole viewport, whatever its style
/// says, so the panels sit under the root. The token panel comes first and
/// covers the top left forty by twenty pixels.
fn token_scene() -> impl IntoElement {
    panels(panel().background(ColorToken::SURFACE))
}

/// A root with two plainly colored panels, `surface` in the token's place.
fn plain_scene(surface: Color) -> impl IntoElement {
    panels(panel().background(surface))
}

/// Sizes the first panel and puts the literal one beside it.
fn panels(first: Element<Panel>) -> impl IntoElement {
    panel()
        .child(first.width(px(40.0)).height(px(20.0)))
        .child(panel().width(px(40.0)).height(px(20.0)).background(LITERAL))
}

/// A harness that mounts `surface` where the token panel sits, for pixel
/// comparisons.
fn plain_harness(surface: Color) -> Harness {
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || plain_scene(surface));
    harness.frame();
    harness
}

#[test]
fn switching_the_theme_repaints_only_token_using_nodes_and_damages_their_bounds() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, token_scene);
    assert!(harness.frame().full, "the first frame repaints everything");

    harness.app().set_theme(Theme::dark());
    let frame = harness.frame();

    assert_eq!(frame.nodes_painted, 1, "only the token-using node repaints");
    assert!(!frame.layout_ran, "a color change never lays out again");
    assert_eq!(
        frame.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 40,
            height: 20,
        }],
        "the damage is exactly the token-using node's bounds"
    );
    harness.assert_matches(&plain_harness(Theme::dark().color(ColorToken::SURFACE)));
}

#[test]
fn a_literal_color_is_unaffected_by_a_theme_switch() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || plain_scene(LITERAL));
    harness.frame();
    let before = plain_harness(LITERAL);

    harness.app().set_theme(Theme::dark());
    let frame = harness.frame();

    assert_eq!(
        frame.nodes_painted, 0,
        "no node uses a token, so nothing repaints"
    );
    assert!(
        frame.damage.is_empty(),
        "a theme switch that nothing reflects damages nothing"
    );
    harness.assert_matches(&before);
}

#[test]
fn bind_theme_follows_the_signal_it_reads() {
    let (dark, set_dark) = reactive::signal_local(false);
    let mut harness = Harness::new(100.0, 60.0, 1.0, token_scene);
    harness.app().bind_theme(move || {
        if dark.get() {
            Theme::dark()
        } else {
            Theme::light()
        }
    });
    harness.frame();
    harness.assert_matches(&plain_harness(Theme::light().color(ColorToken::SURFACE)));

    set_dark.set(true);
    let frame = harness.frame();
    assert_eq!(
        frame.nodes_painted, 1,
        "the switched theme repaints the token user"
    );
    harness.assert_matches(&plain_harness(Theme::dark().color(ColorToken::SURFACE)));

    set_dark.set(false);
    let frame = harness.frame();
    assert_eq!(
        frame.nodes_painted, 1,
        "switching back repaints the token user again"
    );
    harness.assert_matches(&plain_harness(Theme::light().color(ColorToken::SURFACE)));
}

/// A widget whose paint resolves the accent token, or paints `literal` when
/// it holds one, so theme tracking can be checked against a widget that
/// consults the theme inside its own paint and nowhere else.
struct PaintToken(Option<Color>);

impl Widget for PaintToken {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let color = match self.0 {
            Some(literal) => literal,
            None => cx.theme.color(ColorToken::ACCENT),
        };
        out.push(PaintOp::FillRect {
            rect: cx.rect,
            radius: Corners::ZERO,
            color,
        });
    }
}

/// A root whose only child is the paint-time token widget, sized to the top
/// left forty by twenty pixels.
fn dot_scene(literal: Option<Color>) -> impl IntoElement {
    panel().child(
        Element::new(PaintToken(literal))
            .width(px(40.0))
            .height(px(20.0)),
    )
}

/// A harness that mounts the dot painting `literal` plainly, for pixel
/// comparisons.
fn dot_harness(literal: Option<Color>) -> Harness {
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || dot_scene(literal));
    harness.frame();
    harness
}

#[test]
fn a_widget_that_resolves_a_token_only_in_its_paint_repaints_on_a_theme_switch() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || dot_scene(None));
    harness.frame();

    harness.app().set_theme(Theme::dark());
    let frame = harness.frame();

    assert_eq!(
        frame.nodes_painted, 1,
        "the paint-time token user repaints although its style names no token"
    );
    assert_eq!(
        frame.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 40,
            height: 20,
        }],
        "the damage is exactly the widget's bounds"
    );
    harness.assert_matches(&dot_harness(Some(Theme::dark().color(ColorToken::ACCENT))));
}

#[test]
fn a_node_that_paints_nothing_is_not_repainted_by_a_theme_switch() {
    let mut harness = Harness::new(100.0, 60.0, 1.0, || {
        panel().child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .corner_radius(RadiusToken::MEDIUM),
        )
    });
    harness.frame();

    harness.app().set_theme(Theme::dark());
    let report = harness.frame();

    assert_eq!(
        report.nodes_painted, 0,
        "a panel with a token radius but nothing to fill or stroke shows no theme value"
    );
}
