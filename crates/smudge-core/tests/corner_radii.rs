//! Whole-frame behavior of a panel rounded by a radius of its own at each
//! corner, drawn through the headless harness with the CPU renderer.

use std::time::Duration;

use insta::assert_binary_snapshot;
use smudge_core::animation::Spring;
use smudge_core::geometry::{Color, Corners};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Align, px};
use smudge_core::theme::{CornerRadii, RadiusToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;

/// The card's background color.
const CARD: Color = Color::rgb(40, 90, 170);

/// The surface pixel at `(x, y)`, without its alpha.
fn rgb(harness: &Harness, x: u32, y: u32) -> [u8; 3] {
    let [red, green, blue, _] = harness
        .pixel(x, y)
        .expect("the point is inside the surface");
    [red, green, blue]
}

/// The card's color without its alpha.
const CARD_RGB: [u8; 3] = [40, 90, 170];

/// White without its alpha.
const WHITE_RGB: [u8; 3] = [255, 255, 255];

/// Corners rounded by `radius` at the bottom and square at the top.
fn bottom(radius: f32) -> Corners {
    Corners {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: radius,
        bottom_left: radius,
    }
}

#[test]
fn a_card_rounds_only_the_corners_its_radii_give() {
    let mut harness = Harness::new(64.0, 48.0, 1.0, || {
        panel().align_items(Align::Start).child(
            panel()
                .width(px(56.0))
                .height(px(40.0))
                .background(CARD)
                .border(2.0)
                .border_color(Color::rgb(20, 30, 60))
                .corner_radii(bottom(12.0)),
        )
    });
    harness.frame();
    assert_eq!(
        rgb(&harness, 0, 0),
        [20, 30, 60],
        "the top left corner is square"
    );
    assert_eq!(
        rgb(&harness, 55, 0),
        [20, 30, 60],
        "the top right corner is square"
    );
    assert_eq!(
        rgb(&harness, 0, 39),
        WHITE_RGB,
        "the bottom left corner is round"
    );
    assert_eq!(
        rgb(&harness, 55, 39),
        WHITE_RGB,
        "the bottom right corner is round"
    );
    assert_eq!(rgb(&harness, 28, 20), CARD_RGB, "the card's middle");
    assert_binary_snapshot!(".png", harness.png());
}

#[test]
fn a_token_at_one_corner_follows_the_theme() {
    let mut harness = Harness::new(64.0, 48.0, 1.0, || {
        panel().align_items(Align::Start).child(
            panel()
                .width(px(40.0))
                .height(px(40.0))
                .background(CARD)
                .corner_radii(CornerRadii {
                    top_left: RadiusToken::MEDIUM.into(),
                    ..CornerRadii::default()
                }),
        )
    });
    let mut square = Theme::light();
    square.radii.insert(RadiusToken::MEDIUM.0, 0.0);
    harness.app().set_theme(square);
    harness.frame();
    assert_eq!(
        rgb(&harness, 1, 1),
        CARD_RGB,
        "a token of zero leaves the corner square"
    );

    let mut round = Theme::light();
    round.radii.insert(RadiusToken::MEDIUM.0, 16.0);
    harness.app().set_theme(round);
    harness.frame();
    assert_eq!(
        rgb(&harness, 1, 1),
        WHITE_RGB,
        "the corner rounds by the theme's radius"
    );
    assert_eq!(
        rgb(&harness, 38, 1),
        CARD_RGB,
        "the other corners stay square"
    );
    harness.assert_matches_full_repaint();
}

/// A card of 40 by 40 whose corner radii are `radii`, moving under a
/// transition when `transition` is set.
fn card(radii: impl Fn() -> CornerRadii + 'static, transition: bool) -> Harness {
    let mut harness = Harness::new(64.0, 48.0, 1.0, move || {
        let mut card = panel().width(px(40.0)).height(px(40.0)).background(CARD);
        if transition {
            card = card.transition(Spring::SMOOTH);
        }
        panel()
            .align_items(Align::Start)
            .child(card.corner_radii(radii))
    });
    harness.frame();
    harness
}

#[test]
fn a_transition_moves_each_corner_on_its_own() {
    let rounded = RwSignal::new(false);
    let read = rounded.clone();
    let only_bottom_left = |radius: f32| {
        CornerRadii::from(Corners {
            bottom_left: radius,
            ..Corners::ZERO
        })
    };
    let mut harness = card(
        move || only_bottom_left(if read.get() { 18.0 } else { 0.0 }),
        true,
    );
    rounded.set(true);
    harness.frame();
    let mut between = false;
    for _ in 0..90 {
        harness.advance(Duration::from_millis(16));
        for (x, y) in [(0, 0), (39, 0), (39, 39)] {
            assert_eq!(
                rgb(&harness, x, y),
                CARD_RGB,
                "the corner at ({x}, {y}) stays square"
            );
        }
        let corner = rgb(&harness, 2, 37);
        if corner != CARD_RGB && corner != WHITE_RGB {
            between = true;
        }
        harness.assert_matches_full_repaint();
    }
    assert!(
        between,
        "the bottom left corner passed through partly round frames"
    );
    let expected = card(move || only_bottom_left(18.0), false);
    harness.assert_matches(&expected);
}

#[test]
fn a_transition_lands_each_corner_on_its_own_radius() {
    let distinct = || {
        CornerRadii::from(Corners {
            top_left: 0.0,
            top_right: 6.0,
            bottom_right: 12.0,
            bottom_left: 18.0,
        })
    };
    let moved = RwSignal::new(false);
    let read = moved.clone();
    let mut harness = card(
        move || {
            if read.get() {
                distinct()
            } else {
                CornerRadii::default()
            }
        },
        true,
    );
    moved.set(true);
    harness.frame();
    for _ in 0..90 {
        harness.advance(Duration::from_millis(16));
    }
    harness.assert_matches(&card(distinct, false));
}
