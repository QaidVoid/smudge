use crate::geometry::Color;
use crate::theme::{ColorToken, ColorValue, RadiusToken, RadiusValue, Theme};

/// Every built-in color token, for tests that walk them all.
const COLOR_TOKENS: [ColorToken; 9] = [
    ColorToken::SURFACE,
    ColorToken::SURFACE_HOVER,
    ColorToken::SURFACE_PRESSED,
    ColorToken::TEXT,
    ColorToken::TEXT_MUTED,
    ColorToken::ACCENT,
    ColorToken::ACCENT_TEXT,
    ColorToken::BORDER,
    ColorToken::FOCUS_RING,
];

/// Every built-in radius token, for tests that walk them all.
const RADIUS_TOKENS: [RadiusToken; 3] =
    [RadiusToken::SMALL, RadiusToken::MEDIUM, RadiusToken::LARGE];

#[test]
fn builtin_themes_define_every_builtin_token() {
    for theme in [Theme::light(), Theme::dark()] {
        for token in COLOR_TOKENS {
            assert!(
                theme.colors.contains_key(token.0),
                "the theme is missing the color token {token:?}"
            );
        }
        for token in RADIUS_TOKENS {
            assert!(
                theme.radii.contains_key(token.0),
                "the theme is missing the radius token {token:?}"
            );
        }
    }
}

#[test]
fn light_and_dark_disagree_about_the_surface_color() {
    assert_ne!(
        Theme::light().color(ColorToken::SURFACE),
        Theme::dark().color(ColorToken::SURFACE),
        "a theme switch that leaves the surface color alone changes nothing"
    );
}

#[test]
fn a_token_resolves_against_the_theme_it_is_given() {
    let mut theme = Theme::light();
    theme
        .colors
        .insert(ColorToken::ACCENT.0, Color::rgb(9, 8, 7));
    theme.radii.insert(RadiusToken::LARGE.0, 12.0);

    assert_eq!(
        ColorValue::from(ColorToken::ACCENT).resolve(&theme),
        Color::rgb(9, 8, 7)
    );
    assert_eq!(RadiusValue::from(RadiusToken::LARGE).resolve(&theme), 12.0);
}

#[test]
fn a_literal_resolves_to_itself_under_any_theme() {
    let theme = Theme::dark();

    assert_eq!(
        ColorValue::from(Color::rgb(1, 2, 3)).resolve(&theme),
        Color::rgb(1, 2, 3)
    );
    assert_eq!(RadiusValue::from(4.5).resolve(&theme), 4.5);
}

#[test]
fn consulting_the_theme_does_not_make_two_themes_unequal() {
    let consulted = Theme::light();
    consulted.color(ColorToken::SURFACE);
    consulted.radius(RadiusToken::LARGE);

    assert_eq!(consulted, Theme::light(), "equality compares token values");
}
