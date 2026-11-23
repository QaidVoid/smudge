//! Whole-frame behavior of the color picker, driven through the headless
//! harness.
//!
//! Every test installs a bundled-only text system first, because the
//! picker's hex field measures its text, so the layout never depends on the
//! fonts installed on the machine running the tests.

use smudge_core::geometry::{Color, PixelRect, Point, Rect};
use smudge_core::input::{Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;
use smudge_widgets::color_picker;

/// The color every window starts from: an orange that lets nothing
/// through, a twelfth of the way around the wheel from red, with its marker
/// off every edge of the square.
const ORANGE: Color = Color::rgb(230, 150, 60);

/// The center of `rect`.
fn center(rect: Rect) -> Point {
    Point {
        x: (rect.x0 + rect.x1) / 2.0,
        y: (rect.y0 + rect.y1) / 2.0,
    }
}

/// The projected node named `label`, failing the test when there is not
/// exactly one.
fn named(harness: &mut Harness, label: &str) -> SemanticsNode {
    let mut found: Vec<_> = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter(|node| node.label.as_deref() == Some(label))
        .collect();
    assert_eq!(found.len(), 1, "exactly one node is named {label:?}");
    found.remove(0)
}

/// Whether the picker is open, which the hue slider being there says.
fn is_open(harness: &mut Harness) -> bool {
    harness
        .app()
        .semantics_nodes()
        .iter()
        .any(|node| node.label.as_deref() == Some("Hue"))
}

/// Presses the swatch to open the picker, and checks that it did.
fn open_picker(harness: &mut Harness) {
    let swatch = named(harness, "Brush color");
    harness.click(center(swatch.bounds));
    harness.settle();
    assert!(is_open(harness), "the press opens the picker");
}

/// A window of 260 by 400 whose root panel shows one picker bound to
/// `color` at its top left, in `theme`.
fn picker_window(color: RwSignal<Color>, theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(260.0, 400.0, 1.0, move || {
        panel()
            .padding(px(8.0))
            .align_items(Align::Start)
            .child(color_picker(color.clone()).accessible_label("Brush color"))
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness
}

/// A window in the light theme that starts from [`ORANGE`].
fn light_window() -> (Harness, RwSignal<Color>) {
    let color = RwSignal::new(ORANGE);
    let picker = color.clone();
    (picker_window(picker, Theme::light()), color)
}

/// Selects all of the hex field's text.
fn select_all(harness: &mut Harness) {
    harness.press_key_with(
        Key::Character(String::from("a")),
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
}

/// Replaces the hex field's text with `text`.
fn retype(harness: &mut Harness, text: &str) {
    select_all(harness);
    harness.type_text(text);
    harness.frame();
}

/// The color a pixel of `color` takes, as the harness reads it.
fn rgba(color: Color) -> Option<[u8; 4]> {
    Some([color.r, color.g, color.b, color.a])
}

/// Whether `rect` covers the whole square, whose image is 200 by 200.
fn covers_the_square(rect: &PixelRect) -> bool {
    rect.width >= 200 && rect.height >= 200
}

#[test]
fn the_swatch_is_a_color_well_that_names_its_color_in_hex() {
    let (mut harness, color) = light_window();
    let swatch = named(&mut harness, "Brush color");
    assert_eq!(swatch.role, Role::ColorWell);
    assert_eq!(swatch.value.as_deref(), Some("#e6963c"));
    assert_eq!(swatch.expanded, Some(false));
    assert!(swatch.focusable);

    color.set(Color::rgba(1, 2, 3, 4));
    harness.frame();
    assert_eq!(
        named(&mut harness, "Brush color").value.as_deref(),
        Some("#01020304"),
        "a color that lets something through names its alpha"
    );
}

#[test]
fn a_press_on_the_swatch_opens_the_picker_and_the_swatch_shows_it_is_open() {
    let (mut harness, _color) = light_window();
    assert!(!is_open(&mut harness));
    open_picker(&mut harness);
    let swatch = named(&mut harness, "Brush color");
    assert_eq!(swatch.expanded, Some(true));
    for (label, role) in [
        ("Saturation and brightness", Role::Slider),
        ("Hue", Role::Slider),
        ("Alpha", Role::Slider),
        ("Hex color", Role::TextInput),
    ] {
        let node = named(&mut harness, label);
        assert_eq!(node.role, role, "{label}");
    }
    assert_eq!(
        named(&mut harness, "Hex color").value.as_deref(),
        Some("#e6963c")
    );
}

#[test]
fn a_press_on_the_swatch_again_or_outside_closes_the_picker() {
    let (mut harness, _color) = light_window();
    open_picker(&mut harness);
    let swatch = named(&mut harness, "Brush color");
    harness.click(center(swatch.bounds));
    harness.settle();
    assert!(
        !is_open(&mut harness),
        "a second press on the swatch closes it"
    );
    assert_eq!(named(&mut harness, "Brush color").expanded, Some(false));

    open_picker(&mut harness);
    assert!(
        named(&mut harness, "Saturation and brightness").focused,
        "the square holds the focus while the picker is open"
    );
    harness.click(Point { x: 250.0, y: 390.0 });
    harness.settle();
    assert!(!is_open(&mut harness), "a press outside closes it");
    assert!(
        named(&mut harness, "Brush color").focused,
        "and the focus returns to the swatch"
    );
}

#[test]
fn enter_and_space_open_the_picker_and_escape_closes_it_with_the_focus_back() {
    let (mut harness, _color) = light_window();
    harness.press_key(Key::Tab);
    harness.frame();
    assert!(named(&mut harness, "Brush color").focused);

    harness.press_key(Key::Enter);
    harness.frame();
    assert!(is_open(&mut harness), "Enter opens the picker");
    assert!(
        named(&mut harness, "Saturation and brightness").focused,
        "the square takes the focus when the picker opens"
    );
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(!is_open(&mut harness), "Escape closes it");
    assert!(
        named(&mut harness, "Brush color").focused,
        "the focus returns to the swatch"
    );

    harness.press_key(Key::Space);
    harness.frame();
    assert!(is_open(&mut harness), "Space opens it again");
}

#[test]
fn keys_inside_the_picker_never_reach_the_swatch() {
    let (mut harness, _color) = light_window();
    open_picker(&mut harness);
    harness.press_key(Key::Space);
    harness.press_key(Key::Enter);
    harness.frame();
    assert!(
        is_open(&mut harness),
        "Space and Enter on the square stay in it"
    );

    let field = named(&mut harness, "Hex color");
    harness.click(center(field.bounds));
    harness.frame();
    harness.press_key(Key::Enter);
    harness.press_key(Key::Space);
    harness.frame();
    assert!(
        is_open(&mut harness),
        "Space and Enter in the hex field stay in it"
    );
}

#[test]
fn tab_walks_the_square_the_sliders_and_the_hex_field_and_wraps() {
    let (mut harness, _color) = light_window();
    open_picker(&mut harness);
    for label in [
        "Saturation and brightness",
        "Hue",
        "Alpha",
        "Hex color",
        "Saturation and brightness",
    ] {
        assert!(
            named(&mut harness, label).focused,
            "{label} holds the focus"
        );
        harness.press_key(Key::Tab);
        harness.frame();
    }
    assert!(
        named(&mut harness, "Hue").focused,
        "Tab went round inside the picker"
    );
}

#[test]
fn a_press_and_a_drag_on_the_square_write_the_color_at_that_place() {
    let (mut harness, color) = light_window();
    open_picker(&mut harness);
    let square = named(&mut harness, "Saturation and brightness");
    let at = |right: f32, down: f32| Point {
        x: square.bounds.x0 + right,
        y: square.bounds.y0 + down,
    };
    harness.click(at(2.0, 2.0));
    harness.frame();
    let corner = color.get();
    assert!(
        corner.r >= 250 && corner.g >= 250 && corner.b >= 250,
        "the top left is white: {corner:?}"
    );

    harness.button_down(at(100.0, 100.0));
    harness.move_to(at(400.0, -100.0));
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(255, 135, 0),
        "the top right is the pure hue, and a drag past the square stops at its edge"
    );
    harness.move_to(at(400.0, 400.0));
    harness.frame();
    assert_eq!(color.get(), Color::rgb(0, 0, 0), "the bottom is black");
    harness.move_to(at(-100.0, -100.0));
    harness.button_up(at(-100.0, -100.0));
    harness.frame();
    assert_eq!(color.get(), Color::WHITE, "the top left is white");
    assert!(
        is_open(&mut harness),
        "the drag stayed inside, so nothing closed"
    );
}

#[test]
fn the_arrows_move_the_marker_and_page_keys_move_it_further() {
    let (mut harness, color) = light_window();
    open_picker(&mut harness);
    assert_eq!(
        named(&mut harness, "Saturation and brightness")
            .value
            .as_deref(),
        Some("Saturation 74%, brightness 90%"),
        "the square names both of its shares"
    );
    harness.press_key(Key::ArrowUp);
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(232, 151, 61),
        "Up brightens by a hundredth"
    );
    assert_eq!(
        named(&mut harness, "Saturation and brightness")
            .value
            .as_deref(),
        Some("Saturation 74%, brightness 91%"),
        "and the value follows"
    );
    harness.press_key(Key::ArrowDown);
    harness.frame();
    assert_eq!(color.get(), ORANGE, "Down darkens by a hundredth");
    harness.press_key(Key::ArrowLeft);
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(230, 151, 62),
        "Left drains the saturation"
    );
    assert_eq!(
        named(&mut harness, "Saturation and brightness")
            .value
            .as_deref(),
        Some("Saturation 73%, brightness 90%"),
        "and the value names the other axis"
    );
    harness.press_key(Key::ArrowRight);
    harness.frame();
    assert_eq!(color.get(), ORANGE, "Right fills it");
    harness.press_key(Key::PageUp);
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(255, 166, 66),
        "Page Up brightens by a tenth"
    );
    harness.press_key(Key::PageUp);
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(255, 166, 66),
        "the marker stops at the top"
    );
    assert_eq!(
        named(&mut harness, "Hex color").value.as_deref(),
        Some("#ffa642"),
        "the field follows the marker"
    );

    harness.press_key(Key::PageDown);
    harness.frame();
    assert_eq!(color.get(), ORANGE, "Page Down darkens by a tenth");
    for _ in 0..10 {
        harness.press_key(Key::PageDown);
        harness.frame();
    }
    assert_eq!(
        color.get(),
        Color::rgb(0, 0, 0),
        "and the marker stops at the bottom"
    );
    assert_eq!(
        named(&mut harness, "Saturation and brightness")
            .value
            .as_deref(),
        Some("Saturation 74%, brightness 0%")
    );
}

#[test]
fn the_sliders_write_the_hue_and_the_alpha() {
    let (mut harness, color) = light_window();
    open_picker(&mut harness);
    let hue = named(&mut harness, "Hue");
    assert_eq!(
        hue.range.map(|range| range.value),
        Some(31.764_705_882_352_942)
    );
    // A slider's track starts one ring and one thumb radius in, and runs
    // 184 pixels here.
    let along = |node: &SemanticsNode, share: f64| Point {
        x: node.bounds.x0 + 10.0 + share as f32 * 184.0,
        y: center(node.bounds).y,
    };
    harness.click(along(&hue, 120.0 / 360.0));
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(60, 230, 60),
        "a press on the hue track turns the hue, and nothing else"
    );
    assert_eq!(
        named(&mut harness, "Hue").range.map(|range| range.value),
        Some(120.0)
    );

    harness.press_key(Key::ArrowRight);
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgb(60, 230, 63),
        "Right turns the hue one degree"
    );
    harness.press_key(Key::End);
    harness.frame();
    assert_eq!(
        named(&mut harness, "Hue").range.map(|range| range.value),
        Some(360.0),
        "the hue slider spans a whole turn"
    );
    assert_eq!(
        color.get(),
        Color::rgb(230, 60, 60),
        "a whole turn is red again, with the white the orange keeps"
    );

    let alpha = named(&mut harness, "Alpha");
    assert_eq!(alpha.range.map(|range| range.value), Some(1.0));
    harness.click(along(&alpha, 0.25));
    harness.frame();
    let quarter = color.get();
    assert_eq!(
        (quarter.r, quarter.g, quarter.b),
        (230, 60, 60),
        "the alpha slider leaves the color alone"
    );
    assert_eq!(quarter.a, 64, "a quarter of the way is a quarter opaque");
    assert_eq!(
        named(&mut harness, "Alpha").range.map(|range| range.value),
        Some(0.25)
    );
    assert_eq!(
        named(&mut harness, "Hex color").value.as_deref(),
        Some("#e63c3c40"),
        "the hex names the alpha once it lets something through"
    );
    harness.press_key(Key::Home);
    harness.frame();
    assert_eq!(
        color.get(),
        Color::rgba(230, 60, 60, 0),
        "Home is transparent"
    );
    harness.press_key(Key::End);
    harness.frame();
    assert_eq!(color.get(), Color::rgb(230, 60, 60), "End is opaque");
}

#[test]
fn the_hex_field_writes_the_color_it_reads() {
    let (mut harness, color) = light_window();
    open_picker(&mut harness);
    let field = named(&mut harness, "Hex color");
    harness.click(center(field.bounds));
    harness.frame();

    retype(&mut harness, "#00ff80");
    assert_eq!(color.get(), Color::rgb(0, 255, 128));
    assert_eq!(
        named(&mut harness, "Hue").range.map(|range| range.value),
        Some(150.117_647_058_823_54),
        "the hue slider follows"
    );

    retype(&mut harness, "#10203040");
    assert_eq!(color.get(), Color::rgba(16, 32, 48, 64));
    assert_eq!(
        named(&mut harness, "Alpha").range.map(|range| range.value),
        Some(64.0 / 255.0),
        "the alpha slider follows"
    );

    retype(&mut harness, "ff00ff");
    assert_eq!(color.get(), Color::rgb(255, 0, 255), "the # is optional");

    let hue = named(&mut harness, "Hue");
    harness.click(center(hue.bounds));
    harness.frame();
    assert!(
        named(&mut harness, "Hex color")
            .value
            .as_deref()
            .is_some_and(|text| text.starts_with('#') && text.len() == 7),
        "the slider wrote the color, and the field shows it"
    );
}

#[test]
fn a_hex_that_is_no_color_shows_the_invalid_state_and_leaves_the_color() {
    let (mut harness, color) = light_window();
    open_picker(&mut harness);
    let field = named(&mut harness, "Hex color");
    let border = (field.bounds.x0 as u32, center(field.bounds).y as u32);
    assert_eq!(
        harness.pixel(border.0, border.1),
        rgba(Theme::light().color(ColorToken::BORDER))
    );
    harness.click(center(field.bounds));
    harness.frame();

    retype(&mut harness, "#zz0000");
    assert_eq!(color.get(), ORANGE, "text that is no color writes nothing");
    assert!(named(&mut harness, "Hex color").invalid);
    assert_eq!(
        harness.pixel(border.0, border.1),
        rgba(Theme::light().color(ColorToken::ERROR)),
        "the border shows the invalid state"
    );

    retype(&mut harness, "#ff00");
    assert!(
        named(&mut harness, "Hex color").invalid,
        "a color needs six or eight digits"
    );
    assert_eq!(color.get(), ORANGE);

    retype(&mut harness, "#00ff00");
    assert!(!named(&mut harness, "Hex color").invalid, "a color ends it");
    assert_eq!(color.get(), Color::rgb(0, 255, 0));

    select_all(&mut harness);
    harness.press_key(Key::Backspace);
    harness.frame();
    assert_eq!(
        named(&mut harness, "Hex color").value.as_deref(),
        Some(""),
        "the field is empty"
    );
    assert!(
        named(&mut harness, "Hex color").invalid,
        "no text is no color either"
    );
    assert_eq!(color.get(), Color::rgb(0, 255, 0), "and writes nothing");
}

#[test]
fn a_write_from_outside_moves_every_part_of_the_picker() {
    let (mut harness, color) = light_window();
    open_picker(&mut harness);
    color.set(Color::rgb(0, 0, 255));
    harness.frame();
    assert_eq!(
        named(&mut harness, "Hue").range.map(|range| range.value),
        Some(240.0)
    );
    assert_eq!(
        named(&mut harness, "Alpha").range.map(|range| range.value),
        Some(1.0)
    );
    assert_eq!(
        named(&mut harness, "Hex color").value.as_deref(),
        Some("#0000ff")
    );
    let square = named(&mut harness, "Saturation and brightness");
    // Clear of the marker, which sits in the pure corner of this color.
    let tint = harness.pixel(square.bounds.x1 as u32 - 20, square.bounds.y0 as u32 + 2);
    assert!(
        tint.is_some_and(|[r, g, b, _]| r <= 40 && g <= 40 && b >= 250),
        "the square is blue now: {tint:?}"
    );
    assert_eq!(
        harness.pixel(24, 24),
        rgba(Color::rgb(0, 0, 255)),
        "and so is the swatch"
    );

    // Pure blue is already the square's saturated corner, so step down.
    harness.press_key(Key::ArrowDown);
    harness.frame();
    assert_ne!(
        color.get(),
        Color::rgb(0, 0, 255),
        "the picker still writes after following"
    );
}

#[test]
fn the_square_fills_again_only_when_the_hue_changes() {
    let (mut harness, _color) = light_window();
    open_picker(&mut harness);
    harness.press_key(Key::ArrowUp);
    harness.frame();

    harness.press_key(Key::ArrowUp);
    let report = harness.frame();
    assert!(
        !report.damage.iter().any(covers_the_square),
        "a marker move leaves the square alone: {:?}",
        report.damage
    );

    // Two tabs reach the alpha slider, which steps down from its top.
    harness.press_key(Key::Tab);
    harness.press_key(Key::Tab);
    harness.settle();
    harness.press_key(Key::ArrowLeft);
    let report = harness.frame();
    assert!(
        !report.damage.is_empty(),
        "the alpha move repainted something"
    );
    assert!(
        !report.damage.iter().any(covers_the_square),
        "an alpha move leaves the square alone: {:?}",
        report.damage
    );

    harness.press_key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    harness.settle();
    harness.press_key(Key::ArrowRight);
    let report = harness.frame();
    assert_eq!(
        report
            .damage
            .iter()
            .filter(|rect| covers_the_square(rect))
            .count(),
        1,
        "a hue move fills the square once: {:?}",
        report.damage
    );
}

#[test]
fn a_marker_move_repaints_within_the_picker_and_the_swatch() {
    let (mut harness, _color) = light_window();
    open_picker(&mut harness);
    harness.press_key(Key::ArrowUp);
    harness.frame();
    let budget = 234.0 * 330.0;

    harness.press_key(Key::ArrowUp);
    let report = harness.frame();
    let pixels = harness.app().stats().damage_pixels as f32;
    assert!(
        pixels <= budget,
        "{pixels} damage pixels stayed within the picker's {budget}"
    );
    // The picker sits at 10, 42 and the swatch at 8, 8.
    for rect in &report.damage {
        let (left, top) = (rect.x as f32, rect.y as f32);
        let (right, bottom) = (left + rect.width as f32, top + rect.height as f32);
        assert!(
            left >= 7.0 && right <= 241.0 && top >= 7.0 && bottom <= 371.0,
            "damage stayed on the picker and the swatch: {rect:?}"
        );
    }
    harness.assert_matches_full_repaint();
}

#[test]
fn the_open_picker_in_the_light_theme() {
    let color = RwSignal::new(Color::rgba(230, 150, 60, 128));
    let picker = color.clone();
    let mut harness = picker_window(picker, Theme::light());
    open_picker(&mut harness);
    insta::assert_binary_snapshot!(".png", harness.png());
}

#[test]
fn the_open_picker_in_the_dark_theme() {
    let color = RwSignal::new(Color::rgba(230, 150, 60, 128));
    let picker = color.clone();
    let mut harness = picker_window(picker, Theme::dark());
    open_picker(&mut harness);
    insta::assert_binary_snapshot!(".png", harness.png());
}
