use crate::geometry::Color;
use crate::theme::{ColorToken, ColorValue, RadiusToken, RadiusValue, Theme};

/// Every built-in color token, for tests that walk them all.
const COLOR_TOKENS: [ColorToken; 11] = [
    ColorToken::SURFACE,
    ColorToken::SURFACE_HOVER,
    ColorToken::SURFACE_PRESSED,
    ColorToken::TEXT,
    ColorToken::TEXT_MUTED,
    ColorToken::ACCENT,
    ColorToken::ACCENT_HOVER,
    ColorToken::ACCENT_PRESSED,
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

/// Returns the WCAG contrast ratio between two opaque colors, where 1.0 is
/// no contrast and 21.0 is black on white.
fn contrast_ratio(a: Color, b: Color) -> f64 {
    fn luminance(color: Color) -> f64 {
        fn channel(value: u8) -> f64 {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }
    let (lighter, darker) = (luminance(a), luminance(b));
    let (lighter, darker) = if lighter >= darker {
        (lighter, darker)
    } else {
        (darker, lighter)
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[test]
fn text_tokens_contrast_with_the_surface_in_both_themes() {
    // The ratios the WCAG 2.x level AA body-text and large-text bars name.
    // Both built-in themes clear them with room to spare, so a token change
    // that makes default text hard to read on the default surface fails
    // here.
    for (name, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let surface = theme.color(ColorToken::SURFACE);
        let ratio = contrast_ratio(theme.color(ColorToken::TEXT), surface);
        assert!(
            ratio >= 4.5,
            "TEXT on SURFACE contrasts {ratio:.2} under {name}"
        );
        let muted = contrast_ratio(theme.color(ColorToken::TEXT_MUTED), surface);
        assert!(
            muted >= 3.0,
            "TEXT_MUTED on SURFACE contrasts {muted:.2} under {name}"
        );
    }
}
