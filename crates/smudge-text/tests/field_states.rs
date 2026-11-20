//! The states a field shows besides its text: invalid, and masked.

use smudge_core::geometry::{Color, Point};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, text_input};

/// The color a pixel of `color` takes, as the harness reads it.
fn rgba(color: Color) -> Option<[u8; 4]> {
    Some([color.r, color.g, color.b, color.a])
}

/// A window of 200 by 60 with a bordered field at 10, 10, 160 wide, that
/// shows `invalid`, turns its border the accent color under the pointer,
/// and is disabled while `disabled` reads `true`.
fn field(invalid: RwSignal<bool>, disabled: RwSignal<bool>) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 60.0, 1.0, move || {
        let (invalid, disabled) = (invalid.clone(), disabled.clone());
        panel().padding(px(10.0)).child(
            text_input(RwSignal::new(String::from("12x")))
                .width(px(160.0))
                .border(2.0)
                .border_color(ColorToken::BORDER)
                .hover(|p| p.border_color(ColorToken::ACCENT))
                .disabled_style(|p| p.border_color(ColorToken::TEXT_MUTED))
                .invalid(move || invalid.get())
                .disabled(move || disabled.get()),
        )
    });
    harness.settle();
    harness
}

#[test]
fn an_invalid_field_turns_its_border_the_error_color_with_no_layout_pass() {
    let (invalid, disabled) = (RwSignal::new(false), RwSignal::new(false));
    let mut harness = field(invalid.clone(), disabled);
    let theme = Theme::light();
    assert_eq!(harness.pixel(11, 20), rgba(theme.color(ColorToken::BORDER)));

    invalid.set(true);
    let report = harness.frame();
    assert!(!report.layout_ran, "only the border's color changed");
    assert_eq!(harness.pixel(11, 20), rgba(theme.color(ColorToken::ERROR)));
    harness.assert_matches_full_repaint();

    invalid.set(false);
    harness.frame();
    assert_eq!(harness.pixel(11, 20), rgba(theme.color(ColorToken::BORDER)));
}

#[test]
fn a_hovered_invalid_field_keeps_its_error_look_and_a_disabled_one_looks_disabled() {
    let (invalid, disabled) = (RwSignal::new(true), RwSignal::new(false));
    let mut harness = field(invalid.clone(), disabled.clone());
    let theme = Theme::light();

    harness.move_to(Point { x: 60.0, y: 20.0 });
    harness.settle();
    assert_eq!(
        harness.pixel(11, 20),
        rgba(theme.color(ColorToken::ERROR)),
        "the pointer over it does not hide the error"
    );
    invalid.set(false);
    harness.settle();
    assert_eq!(
        harness.pixel(11, 20),
        rgba(theme.color(ColorToken::ACCENT)),
        "where a valid field shows its hover look"
    );
    invalid.set(true);
    harness.settle();

    disabled.set(true);
    harness.settle();
    assert_eq!(
        harness.pixel(11, 20),
        rgba(theme.color(ColorToken::TEXT_MUTED)),
        "a disabled field looks disabled, whatever it holds"
    );
}

/// Draws a valid and an invalid field in `theme`.
fn valid_and_invalid(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(220.0, 90.0, 1.0, || {
        panel()
            .column()
            .padding(px(10.0))
            .gap(smudge_core::geometry::Size {
                width: 0.0,
                height: 10.0,
            })
            .background(ColorToken::SURFACE)
            .child(
                text_input(RwSignal::new(String::from("24")))
                    .width(px(200.0))
                    .padding(px(4.0))
                    .border(1.0)
                    .border_color(ColorToken::BORDER),
            )
            .child(
                text_input(RwSignal::new(String::from("24px")))
                    .width(px(200.0))
                    .padding(px(4.0))
                    .border(1.0)
                    .border_color(ColorToken::BORDER)
                    .invalid(true),
            )
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness
}

#[test]
fn a_valid_and_an_invalid_field_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", valid_and_invalid(Theme::light()).png());
}

#[test]
fn a_valid_and_an_invalid_field_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", valid_and_invalid(Theme::dark()).png());
}
