//! Behavior tests for theme tokens.
//!
//! Every test drives the same frame pipeline a window does, through the
//! harness, and checks what the surface shows against a harness that mounts
//! the expected colors plainly.

use smudge_core::element::{Element, IntoElement};
use smudge_core::geometry::{Color, Corners, PixelRect};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{self, Owner, prelude::*};
use smudge_core::style::px;
use smudge_core::theme::{ColorScheme, ColorToken, RadiusToken, Theme};
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
    plain_harness_on(Color::WHITE, surface)
}

/// A plain harness that starts from `clear`, for comparisons against a
/// surface whose clear color the app switched.
fn plain_harness_on(clear: Color, surface: Color) -> Harness {
    let mut harness =
        Harness::with_clear_color(100.0, 60.0, 1.0, clear, move || plain_scene(surface));
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

#[test]
fn following_the_color_scheme_switches_the_theme_and_the_clear_color() {
    let dark_clear = Color::rgb(16, 17, 21);
    let mut harness = Harness::new(100.0, 60.0, 1.0, token_scene);
    harness
        .app()
        .follow_color_scheme(Theme::light(), Color::WHITE, Theme::dark(), dark_clear);
    let frame = harness.frame();
    assert_eq!(
        frame.clear_color,
        Color::WHITE,
        "the light side paints the light clear color"
    );
    harness.assert_matches(&plain_harness(Theme::light().color(ColorToken::SURFACE)));

    harness.app().set_color_scheme(ColorScheme::Dark);
    let frame = harness.frame();
    assert!(
        frame.full,
        "a clear color change repaints the whole surface"
    );
    assert_eq!(frame.clear_color, dark_clear);
    harness.assert_matches(&plain_harness_on(
        dark_clear,
        Theme::dark().color(ColorToken::SURFACE),
    ));

    harness.app().set_color_scheme(ColorScheme::Light);
    let frame = harness.frame();
    assert_eq!(frame.clear_color, Color::WHITE);
    harness.assert_matches(&plain_harness(Theme::light().color(ColorToken::SURFACE)));
}

#[test]
fn a_translucent_clear_color_of_the_follower_is_used_opaque() {
    // Presentation copies only the color channels, so a translucent clear
    // color would fail the frame that switches to it; the follower takes
    // it opaque from the start instead.
    let mut harness = Harness::new(100.0, 60.0, 1.0, token_scene);
    harness.app().follow_color_scheme(
        Theme::light(),
        Color::rgba(250, 250, 250, 10),
        Theme::dark(),
        Color::rgba(16, 17, 21, 200),
    );
    assert_eq!(harness.frame().clear_color, Color::rgb(250, 250, 250));

    harness.app().set_color_scheme(ColorScheme::Dark);

    assert_eq!(harness.frame().clear_color, Color::rgb(16, 17, 21));
}

#[test]
fn a_scheme_reported_before_the_follow_sets_the_start() {
    let dark_clear = Color::rgb(16, 17, 21);
    let mut harness = Harness::new(100.0, 60.0, 1.0, token_scene);
    // The desktop's report can arrive before the app asks to follow it.
    harness.app().set_color_scheme(ColorScheme::Dark);
    harness
        .app()
        .follow_color_scheme(Theme::light(), Color::WHITE, Theme::dark(), dark_clear);
    let frame = harness.frame();
    assert_eq!(
        frame.clear_color, dark_clear,
        "the follower starts on the report it already holds"
    );
    harness.assert_matches(&plain_harness_on(
        dark_clear,
        Theme::dark().color(ColorToken::SURFACE),
    ));
}

#[test]
fn a_first_follow_applies_the_clear_color_the_window_started_without() {
    // hot.rs starts its window on a dark clear color; a system in light mode
    // that has not reported yet must still land on the light side.
    let started = Color::rgb(16, 17, 21);
    let mut harness = Harness::with_clear_color(100.0, 60.0, 1.0, started, token_scene);
    harness
        .app()
        .follow_color_scheme(Theme::light(), Color::WHITE, Theme::dark(), started);
    let frame = harness.frame();
    assert_eq!(
        frame.clear_color,
        Color::WHITE,
        "a follow with no report yet applies the light clear color"
    );
    harness.assert_matches(&plain_harness(Theme::light().color(ColorToken::SURFACE)));
}

#[test]
fn a_later_follow_names_the_themes_the_binding_paints() {
    let mut light2 = Theme::light();
    light2
        .colors
        .insert(ColorToken::SURFACE.0, Color::rgb(90, 10, 10));
    let mut dark2 = Theme::dark();
    dark2
        .colors
        .insert(ColorToken::SURFACE.0, Color::rgb(5, 60, 70));
    let mut harness = Harness::new(100.0, 60.0, 1.0, token_scene);
    harness.app().follow_color_scheme(
        Theme::light(),
        Color::WHITE,
        Theme::dark(),
        Color::rgb(16, 17, 21),
    );
    harness.frame();

    let (light_clear, dark_clear) = (Color::rgb(1, 2, 3), Color::rgb(4, 5, 6));
    harness
        .app()
        .follow_color_scheme(light2.clone(), light_clear, dark2.clone(), dark_clear);
    let frame = harness.frame();
    assert_eq!(
        frame.clear_color, light_clear,
        "the replacement names a clear color the next frame paints"
    );
    harness.assert_matches(&plain_harness_on(
        light_clear,
        light2.color(ColorToken::SURFACE),
    ));

    // Both sides of the replacement are live: the scheme flips to the
    // replacement's dark pair, then back to its light pair.
    harness.app().set_color_scheme(ColorScheme::Dark);
    let frame = harness.frame();
    assert_eq!(
        frame.clear_color, dark_clear,
        "the replacement's dark clear color paints after the flip"
    );
    harness.assert_matches(&plain_harness_on(
        dark_clear,
        dark2.color(ColorToken::SURFACE),
    ));

    harness.app().set_color_scheme(ColorScheme::Light);
    let frame = harness.frame();
    assert_eq!(
        frame.clear_color, light_clear,
        "the replacement's light clear color paints after the flip back"
    );
    harness.assert_matches(&plain_harness_on(
        light_clear,
        light2.color(ColorToken::SURFACE),
    ));
}

#[test]
fn a_follower_survives_a_remount_and_keeps_reporting() {
    let dark_clear = Color::rgb(16, 17, 21);
    let mut harness = Harness::with_clear_color(100.0, 60.0, 1.0, Color::WHITE, token_scene);
    // The follow runs under an owner the caller drops, as a window's setup
    // code might; the follower must root its signal with the app, not
    // with whoever was current when the call landed.
    let caller = Owner::new();
    caller.with(|| {
        harness
            .app()
            .follow_color_scheme(Theme::light(), Color::WHITE, Theme::dark(), dark_clear);
    });
    drop(caller);
    harness.frame();

    // The report lands before the remount, and must still drive the frame
    // after the build's owner was replaced.
    harness.app().set_color_scheme(ColorScheme::Dark);
    harness.app().remount(token_scene);
    let frame = harness.frame();
    assert_eq!(frame.clear_color, dark_clear);
    harness.assert_matches(&plain_harness_on(
        dark_clear,
        Theme::dark().color(ColorToken::SURFACE),
    ));

    harness.app().set_color_scheme(ColorScheme::Light);
    let frame = harness.frame();
    assert_eq!(frame.clear_color, Color::WHITE);
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
