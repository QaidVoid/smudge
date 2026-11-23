use proptest::prelude::*;

use super::*;

/// The straight RGBA8 pixel of `image` at `at`, a byte offset.
fn pixel(image: &Image, at: u32) -> [u8; 4] {
    let at = at as usize;
    let pixels = image.pixels();
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// The colors on the axes of the picker, at known places.
#[test]
fn known_colors_have_known_axes() {
    let axes = |hue: f64, saturation: f64, value: f64| Hsv {
        hue,
        saturation,
        value,
        alpha: 1.0,
    };
    assert_eq!(axes(0.0, 1.0, 1.0).to_color(), Color::rgb(255, 0, 0));
    assert_eq!(axes(120.0, 1.0, 1.0).to_color(), Color::rgb(0, 255, 0));
    assert_eq!(axes(240.0, 1.0, 1.0).to_color(), Color::rgb(0, 0, 255));
    assert_eq!(axes(360.0, 1.0, 1.0).to_color(), Color::rgb(255, 0, 0));
    assert_eq!(axes(30.0, 1.0, 1.0).to_color(), Color::rgb(255, 128, 0));
    assert_eq!(axes(0.0, 0.0, 1.0).to_color(), Color::WHITE);
    assert_eq!(axes(0.0, 1.0, 0.0).to_color(), Color::BLACK);
    let orange = Hsv::from_color(Color::rgb(255, 128, 0));
    // A half is 127.5 in eight bits, which no channel can hold.
    assert!((orange.hue - 30.0).abs() < 0.2, "{}", orange.hue);
    assert_eq!(orange.saturation, 1.0);
    assert_eq!(orange.value, 1.0);
    assert_eq!(Hsv::from_color(Color::rgb(128, 64, 64)).saturation, 0.5);
    assert_eq!(Hsv::from_color(Color::WHITE).saturation, 0.0);
}

// Every color comes back from the picker's axes as itself, so a write
// from outside is never nudged by the picker reading it.
proptest! {
    #[test]
    fn every_color_survives_a_round_trip(
        r in any::<u8>(), g in any::<u8>(), b in any::<u8>(), a in any::<u8>()
    ) {
        let color = Color::rgba(r, g, b, a);
        prop_assert_eq!(Hsv::from_color(color).to_color(), color);
    }
}

/// The hex of a color, and the color of a hex.
#[test]
fn hex_round_trips() {
    assert_eq!(hex(Color::rgb(255, 128, 0)), "#ff8000");
    assert_eq!(hex(Color::rgba(1, 2, 3, 4)), "#01020304");
    assert_eq!(parse_hex("#ff8000"), Some(Color::rgb(255, 128, 0)));
    assert_eq!(parse_hex("#01020304"), Some(Color::rgba(1, 2, 3, 4)));
    assert_eq!(parse_hex(" FF8000 "), Some(Color::rgb(255, 128, 0)));
    assert_eq!(parse_hex("#ff80"), None);
    assert_eq!(parse_hex("#zz0000"), None);
    assert_eq!(parse_hex("#+12345"), None, "a sign is no hex digit");
    assert_eq!(parse_hex("#-12345"), None);
    assert_eq!(parse_hex(""), None);
    assert_eq!(parse_hex("red"), None);
    assert_eq!(parse_hex("#ff8000é"), None);
}

/// The square shows white, the hue and black at its corners.
#[test]
fn the_square_has_its_three_corners() {
    let square = square_image(120.0);
    let side = SQUARE as u32;
    let at = |x: u32, y: u32| pixel(&square, (y * side + x) * 4);
    assert_eq!(at(0, 0), [255, 255, 255, 255]);
    assert_eq!(at(side - 1, 0), [0, 255, 0, 255]);
    assert_eq!(at(0, side - 1), [0, 0, 0, 255]);
}

/// The hue track starts and ends red, with green and blue a third and two
/// thirds along.
#[test]
fn the_hue_track_runs_the_wheel() {
    let track = hue_track();
    let at = |x: u32| pixel(&track, x * 4);
    assert_eq!(at(0), [255, 2, 0, 255]);
    assert_eq!(at(120), [0, 255, 2, 255]);
    assert_eq!(at(240), [2, 0, 255, 255]);
    assert_eq!(at(359), [255, 0, 2, 255]);
}

/// The alpha track and the swatch lay the color over the checkerboard.
#[test]
fn the_alpha_track_fades_the_color_over_the_checkerboard() {
    let track = alpha_track(Color::rgb(255, 0, 0));
    let width = track.width();
    let at = |x: u32, y: u32| pixel(&track, (y * width + x) * 4);
    assert_eq!(at(0, 0), [255, 255, 255, 255], "nothing of it at the left");
    assert_eq!(at(1, 0), [255, 254, 254, 255]);
    assert_eq!(at(width - 1, 0), [255, 0, 0, 255], "all of it at the right");
    assert_eq!(
        at(0, 4),
        [204, 204, 204, 255],
        "the other square of the checkerboard"
    );
    let swatch = swatch_image(Color::rgba(255, 0, 0, 128));
    assert_eq!(pixel(&swatch, 0), [255, 127, 127, 255]);
    assert_eq!(pixel(&swatch, 4 * 4), [230, 102, 102, 255]);
}
