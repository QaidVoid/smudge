//! Unit tests for the pure resolution of style variants.
//!
//! The integration tests observe the resolved style through the frame
//! pipeline; these tests call `resolve` directly and check the order the
//! patches apply in and the fields each one overrides.

use super::{PatchBuilder, StylePatch, Variants, resolve};
use crate::geometry::Color;
use crate::style::{Length, Style};
use crate::tree::interaction::Interaction;

/// A base style whose background tells the patches apart.
fn base() -> Style {
    let mut style = Style::default();
    style.paint.background = Some(Color::BLACK);
    style
}

/// A patch that sets only the background.
fn background_patch(color: Color) -> StylePatch {
    PatchBuilder::default().background(color).patch()
}

/// An interaction state with exactly the given bits set.
fn interaction(bits: &[u8]) -> Interaction {
    let mut interaction = Interaction::default();
    for bit in bits {
        interaction.set(*bit, true);
    }
    interaction
}

/// One patch per state, each with its own background.
fn every_patch() -> Variants {
    Variants {
        hovered: Some(background_patch(Color::rgb(10, 0, 0))),
        focused: Some(background_patch(Color::rgb(20, 0, 0))),
        pressed: Some(background_patch(Color::rgb(30, 0, 0))),
        disabled: Some(background_patch(Color::rgb(40, 0, 0))),
    }
}

#[test]
fn patches_apply_in_a_fixed_order_and_pressed_overrides_hovered() {
    let variants = every_patch();
    let hovered = interaction(&[Interaction::HOVERED]);
    let focused = interaction(&[Interaction::HOVERED, Interaction::KEYBOARD_FOCUS]);
    let pressed = interaction(&[
        Interaction::HOVERED,
        Interaction::KEYBOARD_FOCUS,
        Interaction::PRESSED,
    ]);
    let disabled = interaction(&[
        Interaction::HOVERED,
        Interaction::KEYBOARD_FOCUS,
        Interaction::PRESSED,
        Interaction::DISABLED,
    ]);

    assert_eq!(
        resolve(&base(), &variants, disabled).paint.background,
        Some(Color::rgb(40, 0, 0)),
        "the disabled patch wins over every other state"
    );
    assert_eq!(
        resolve(&base(), &variants, pressed).paint.background,
        Some(Color::rgb(30, 0, 0)),
        "pressed overrides hovered and focused for the same field"
    );
    assert_eq!(
        resolve(&base(), &variants, focused).paint.background,
        Some(Color::rgb(20, 0, 0)),
        "focused overrides hovered for the same field"
    );
    assert_eq!(
        resolve(&base(), &variants, hovered).paint.background,
        Some(Color::rgb(10, 0, 0))
    );
    assert_eq!(
        resolve(&base(), &variants, interaction(&[]))
            .paint
            .background,
        Some(Color::BLACK),
        "without any state the base style stands"
    );
}

#[test]
fn a_focused_patch_applies_only_while_keyboard_navigation_is_on() {
    let variants = Variants {
        focused: Some(background_patch(Color::rgb(20, 0, 0))),
        ..Variants::default()
    };

    let mouse = interaction(&[Interaction::FOCUSED]);
    assert_eq!(
        resolve(&base(), &variants, mouse).paint.background,
        Some(Color::BLACK),
        "a focus that a press moved draws no focus style"
    );

    let keyboard = interaction(&[Interaction::FOCUSED, Interaction::KEYBOARD_FOCUS]);
    assert_eq!(
        resolve(&base(), &variants, keyboard).paint.background,
        Some(Color::rgb(20, 0, 0)),
        "a focus that Tab moved applies the focused patch"
    );
}

#[test]
fn a_patch_overrides_only_the_fields_its_setters_wrote() {
    let mut base_style = base();
    base_style.layout.width = Length::px(10.0);
    let variants = Variants {
        hovered: Some(background_patch(Color::rgb(10, 0, 0))),
        ..Variants::default()
    };

    let resolved = resolve(&base_style, &variants, interaction(&[Interaction::HOVERED]));
    assert_eq!(resolved.paint.background, Some(Color::rgb(10, 0, 0)));
    assert_eq!(
        resolved.layout.width,
        Length::px(10.0),
        "a paint patch leaves the layout fields alone"
    );
}

#[test]
fn a_state_without_a_patch_keeps_the_base_style() {
    let variants = Variants {
        pressed: Some(background_patch(Color::rgb(30, 0, 0))),
        ..Variants::default()
    };

    let resolved = resolve(
        &base(),
        &variants,
        interaction(&[Interaction::HOVERED, Interaction::DISABLED]),
    );
    assert_eq!(
        resolved.paint.background,
        Some(Color::BLACK),
        "hovered and disabled have no patch, so the base background stands"
    );
}
