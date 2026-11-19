use smudge_core::geometry::Color;
use smudge_core::theme::{ColorToken, Theme};

use super::{apply_theme_file, theme_file};

#[test]
fn good_lines_set_their_tokens_and_the_others_are_reported() {
    let mut theme = Theme::dark();
    let before = theme.clone();
    let skipped = apply_theme_file(
        "# a comment\n\naccent = #e07a3a\nsurface = #10203040\nglow = #ffffff\nborder = #zz0000\nnonsense\n",
        &mut theme,
    );
    assert_eq!(
        theme.color(ColorToken::ACCENT),
        Color::rgb(0xe0, 0x7a, 0x3a)
    );
    assert_eq!(
        theme.color(ColorToken::SURFACE),
        Color::rgba(0x10, 0x20, 0x30, 0x40)
    );
    assert_eq!(
        theme.color(ColorToken::BORDER),
        before.color(ColorToken::BORDER),
        "a bad value leaves the token alone"
    );
    assert_eq!(
        skipped,
        [
            "line 5: the theme has no token `glow`",
            "line 6: `#zz0000` is not a color like #rrggbb or #rrggbbaa",
            "line 7: `nonsense` is not `token = value`",
        ]
    );
}

#[test]
fn a_written_theme_reads_back_as_the_same_theme() {
    let mut theme = Theme::light();
    if let Some(color) = theme.colors.values_mut().next() {
        *color = Color::rgba(1, 2, 3, 4);
    }
    if let Some(radius) = theme.radii.values_mut().next() {
        *radius = 7.5;
    }
    let mut read = Theme::dark();
    let skipped = apply_theme_file(&theme_file(&theme), &mut read);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(read.colors, theme.colors);
    assert_eq!(read.radii, theme.radii);
}
