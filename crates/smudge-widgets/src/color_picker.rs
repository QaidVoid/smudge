//! The color picker: a swatch that shows a color and opens a picker for it.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{Element, Placement};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::image::Image;
use smudge_core::input::{Drag, Handled, Key, KeyEvent};
use smudge_core::reactive::{Memo, RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Edges, Length, pct, px};
use smudge_core::theme::{ColorToken, RadiusToken};
use smudge_core::tree::Dirty;
use smudge_core::widget::{Panel, panel};
use smudge_core::window::Cursor;
use smudge_text::{TextInputExt, text_input};

use crate::containers::column;
use crate::controls::{RING, TRACK_INSET, slider_with_track};
use crate::image::{ImageView, image};

/// The side of the swatch, its ring included.
const SWATCH: f32 = 32.0;

/// The side of the saturation and value square, its ring excluded.
const SQUARE: f32 = 200.0;

/// The side of the square's marker, its outline included.
const MARKER: f32 = 14.0;

/// The height of a slider's track, as the slider paints it.
const TRACK: f32 = 4.0;

/// The two colors of the checkerboard that shows how opaque a color is.
const CHECKER: [Color; 2] = [Color::rgb(255, 255, 255), Color::rgb(204, 204, 204)];

/// The key of the binding that writes the color the parts of the picker
/// have chosen.
struct Write;

/// The key of the binding that moves the parts of the picker to a color
/// written from outside.
struct Follow;

/// The key of the binding that shows the color in the hex field.
struct Shown;

/// The key of the binding that reads the hex field's text.
struct Typed;

/// A color on the axes the picker moves it on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsv {
    /// The hue, in degrees around the color wheel.
    hue: f64,
    /// The saturation, from 0 for gray to 1 for the purest color.
    saturation: f64,
    /// The value, from 0 for black to 1 for the brightest color.
    value: f64,
    /// The alpha, from 0 for transparent to 1 for opaque.
    alpha: f64,
}

impl Hsv {
    /// The color as the picker writes it to the signal it is bound to.
    fn to_color(self) -> Color {
        let (s, v) = (self.saturation.clamp(0.0, 1.0), self.value.clamp(0.0, 1.0));
        let wheel = self.hue.rem_euclid(360.0) / 60.0;
        let sector = wheel.floor();
        let turn = wheel - sector;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * turn);
        let t = v * (1.0 - s * (1.0 - turn));
        let (r, g, b) = match sector as u32 % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        let channel = |share: f64| (share * 255.0).round() as u8;
        Color::rgba(
            channel(r),
            channel(g),
            channel(b),
            channel(self.alpha.clamp(0.0, 1.0)),
        )
    }

    /// The color on the picker's axes. A gray has no hue and reads as red.
    fn from_color(color: Color) -> Self {
        let (r, g, b) = (
            f64::from(color.r) / 255.0,
            f64::from(color.g) / 255.0,
            f64::from(color.b) / 255.0,
        );
        let (most, least) = (r.max(g).max(b), r.min(g).min(b));
        let spread = most - least;
        let hue = if spread == 0.0 {
            0.0
        } else if most == r {
            60.0 * ((g - b) / spread).rem_euclid(6.0)
        } else if most == g {
            60.0 * ((b - r) / spread + 2.0)
        } else {
            60.0 * ((r - g) / spread + 4.0)
        };
        Self {
            hue,
            saturation: if most == 0.0 { 0.0 } else { spread / most },
            value: most,
            alpha: f64::from(color.a) / 255.0,
        }
    }
}

/// The color written as `#rrggbb`, or as `#rrggbbaa` when it lets
/// something through.
fn hex(color: Color) -> String {
    if color.is_opaque() {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        )
    }
}

/// The color `text` names as `#rrggbb` or `#rrggbbaa`, with or without the
/// `#` and with whitespace at either end ignored, or `None` for any other
/// text.
fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    // `from_str_radix` would read a leading `+` as a sign.
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    match digits.len() {
        6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// The saturation and value square of `hue`: white at its top left, the
/// pure hue at its top right and black along its bottom.
fn square_image(hue: f64) -> Image {
    let side = SQUARE as u32;
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        let value = 1.0 - f64::from(y) / f64::from(side - 1);
        for x in 0..side {
            let saturation = f64::from(x) / f64::from(side - 1);
            pixels.extend_from_slice(&bytes(
                Hsv {
                    hue,
                    saturation,
                    value,
                    alpha: 1.0,
                }
                .to_color(),
            ));
        }
    }
    Image::from_straight_rgba(side, side, pixels).expect("the square has pixels")
}

/// The track of the hue slider: the pure hues from red around the wheel
/// back to red, one pixel per degree.
fn hue_track() -> Image {
    let degrees = 360_u32;
    let rows = TRACK as u32;
    let mut pixels = Vec::with_capacity((degrees * rows * 4) as usize);
    for _ in 0..rows {
        for degree in 0..degrees {
            pixels.extend_from_slice(&bytes(
                Hsv {
                    hue: (f64::from(degree) + 0.5) * 360.0 / f64::from(degrees),
                    saturation: 1.0,
                    value: 1.0,
                    alpha: 1.0,
                }
                .to_color(),
            ));
        }
    }
    Image::from_straight_rgba(degrees, rows, pixels).expect("the track has pixels")
}

/// The track of the alpha slider: `color` over the checkerboard, with
/// nothing of it at the left and all of it at the right. Its pixels are
/// twice the track's size, so the checkerboard keeps squares of two logical
/// pixels at the scale of one and of two.
fn alpha_track(color: Color) -> Image {
    let width = 2.0 * (SQUARE - 2.0 * TRACK_INSET);
    let height = 2.0 * TRACK;
    let (width, height) = (width as u32, height as u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&over(
                color,
                (f64::from(x) + 0.5) / f64::from(width),
                checker(x / 4, y / 4),
            ));
        }
    }
    Image::from_straight_rgba(width, height, pixels).expect("the track has pixels")
}

/// The swatch of `color` over the checkerboard, in pixels twice the
/// swatch's size.
fn swatch_image(color: Color) -> Image {
    let side = (2.0 * (SWATCH - 2.0 * RING - 2.0)) as u32;
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            pixels.extend_from_slice(&over(
                color,
                f64::from(color.a) / 255.0,
                checker(x / 4, y / 4),
            ));
        }
    }
    Image::from_straight_rgba(side, side, pixels).expect("the swatch has pixels")
}

/// The checkerboard color of the square at the cell `x`, `y`.
fn checker(x: u32, y: u32) -> Color {
    CHECKER[((x + y) % 2) as usize]
}

/// `color` with the share `alpha` of it laid over `under`, as opaque
/// straight RGBA8.
fn over(color: Color, alpha: f64, under: Color) -> [u8; 4] {
    let mix = |over: u8, under: u8| {
        (alpha * f64::from(over) + (1.0 - alpha) * f64::from(under)).round() as u8
    };
    [
        mix(color.r, under.r),
        mix(color.g, under.g),
        mix(color.b, under.b),
        255,
    ]
}

/// `color` as straight RGBA8.
fn bytes(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

/// The signals the parts of the picker share.
#[derive(Clone)]
struct Parts {
    /// The color the picker is bound to.
    color: RwSignal<Color>,
    /// The hue the hue slider holds, in degrees.
    hue: RwSignal<f64>,
    /// The saturation the marker's horizontal place holds.
    saturation: RwSignal<f64>,
    /// The value the marker's vertical place holds.
    value: RwSignal<f64>,
    /// The alpha the alpha slider holds.
    alpha: RwSignal<f64>,
    /// The text of the hex field.
    text: RwSignal<String>,
    /// Whether the hex field shows the invalid state.
    invalid: RwSignal<bool>,
}

/// Creates a swatch bound to `color` that opens a picker under it.
///
/// A press opens the picker below the swatch and closes it again, and so do
/// Enter and Space while the swatch holds the focus. The picker holds a
/// saturation and value square, a hue slider and an alpha slider, each of
/// which writes `color` as it moves, and a hex field that writes `color`
/// once its text is `#rrggbb` or `#rrggbbaa`, with or without the `#`, and
/// shows the invalid state while its text is anything else, an empty field
/// included. The square takes the focus when the picker opens. Left and
/// Right move its marker's saturation and Up and Down its value, by a
/// hundredth, and Page Up and Page Down the value by a tenth. Tab walks the
/// square, the two sliders and the hex field. Escape closes the picker and
/// the focus returns to the swatch, as it does after a press outside. A
/// write to `color` from outside moves every part of the picker. A screen
/// reader hears a color well whose value is the color in hex, and the square
/// as a slider whose value names its saturation and brightness in percent.
/// Give the swatch a name with [`accessible_label`](Element::accessible_label).
///
/// ```
/// use smudge_core::geometry::Color;
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::color_picker;
///
/// let brush = RwSignal::new(Color::rgb(230, 150, 60));
/// let picker = color_picker(brush).accessible_label("Brush color");
/// ```
#[track_caller]
pub fn color_picker(color: RwSignal<Color>) -> Element<Panel> {
    let open = crate::signal(false);
    let Hsv {
        hue,
        saturation,
        value,
        alpha,
    } = Hsv::from_color(color.get());
    let parts = Parts {
        color: color.clone(),
        hue: crate::signal(hue),
        saturation: crate::signal(saturation),
        value: crate::signal(value),
        alpha: crate::signal(alpha),
        text: crate::signal(hex(color.get())),
        invalid: crate::signal(false),
    };
    // The last color the picker wrote, which tells a write of its own from
    // one from outside.
    let written = Rc::new(Cell::new(color.get()));
    let named = color.clone();
    let swatch = Memo::new(move || swatch_image(color.get()));
    let picker = {
        let parts = parts.clone();
        move || popup(parts.clone())
    };
    let (toggle, key, expanded) = (open.clone(), open.clone(), open.clone());
    let (writing, following, showing, typing) =
        (parts.clone(), parts.clone(), parts.clone(), parts);
    let followed = Rc::clone(&written);
    panel()
        .width(px(SWATCH))
        .height(px(SWATCH))
        .border(RING)
        .border_color(Color::TRANSPARENT)
        .corner_radius(RadiusToken::SMALL)
        .focused(|p| p.border_color(ColorToken::FOCUS_RING))
        .disabled_style(|p| p.opacity(0.5))
        .role(Role::ColorWell)
        .accessible_value(move || hex(named.get()))
        .accessible_expanded(move || expanded.get())
        .focusable()
        .cursor(Cursor::Pointer)
        .on_press(move || toggle.update(|open| *open = !*open))
        .on_key(move |event| {
            // Keys from inside the open picker bubble here too, and belong
            // to the picker.
            let opens = matches!(event.key, Key::Enter | Key::Space);
            if event.pressed && !event.repeat && opens && !key.get() {
                key.set(true);
                return Handled::Yes;
            }
            Handled::No
        })
        .bind(
            Write,
            move || {
                let next = Hsv {
                    hue: writing.hue.get(),
                    saturation: writing.saturation.get(),
                    value: writing.value.get(),
                    alpha: writing.alpha.get(),
                }
                .to_color();
                // Untracked, so a write from outside never wakes this.
                if writing.color.read(|color| *color) != next {
                    written.set(next);
                    writing.color.set(next);
                }
            },
            |_, ()| Dirty::Clean,
        )
        .bind(
            Follow,
            move || {
                let now = following.color.get();
                if followed.get() != now {
                    followed.set(now);
                    let Hsv {
                        hue,
                        saturation,
                        value,
                        alpha,
                    } = Hsv::from_color(now);
                    set_if_changed(&following.hue, hue);
                    set_if_changed(&following.saturation, saturation);
                    set_if_changed(&following.value, value);
                    set_if_changed(&following.alpha, alpha);
                }
            },
            |_, ()| Dirty::Clean,
        )
        .bind(
            Shown,
            move || {
                let shown = hex(showing.color.get());
                // Untracked, so typing, which writes the text, never wakes
                // this and puts the color back.
                if showing.text.read(|text| !text.eq_ignore_ascii_case(&shown)) {
                    showing.text.set(shown);
                }
            },
            |_, ()| Dirty::Clean,
        )
        .bind(
            Typed,
            move || {
                let typed = typing.text.with(|text| text.trim().to_owned());
                match parse_hex(&typed) {
                    Some(next) => {
                        if typing.color.read(|color| *color) != next {
                            typing.color.set(next);
                        }
                        set_if_changed(&typing.invalid, false);
                    }
                    None => set_if_changed(&typing.invalid, true),
                }
            },
            |_, ()| Dirty::Clean,
        )
        .child(
            image(move || Some(swatch.get()))
                .corner_radius(RadiusToken::SMALL)
                .border(1.0)
                .border_color(ColorToken::BORDER),
        )
        .overlay(open, Placement::Below, picker)
}

/// Writes `value` to `signal` unless it already holds it, so a part that
/// lands where it was wakes nothing.
fn set_if_changed<T: Clone + PartialEq + 'static>(signal: &RwSignal<T>, value: T) {
    if signal.get() != value {
        signal.set(value);
    }
}

/// The open picker, with every part bound to the signals in `parts`.
fn popup(parts: Parts) -> Element<Panel> {
    let Parts {
        color: _,
        hue,
        saturation,
        value,
        alpha,
        text,
        invalid,
    } = parts;
    // The square depends on the hue alone, so moving the marker or the
    // alpha slider never fills it again.
    let square = {
        let hue = hue.clone();
        Memo::new(move || square_image(hue.get()))
    };
    let hue_image = hue_track();
    let alpha_image = {
        let (hue, saturation, value) = (hue.clone(), saturation.clone(), value.clone());
        Memo::new(move || {
            alpha_track(
                Hsv {
                    hue: hue.get(),
                    saturation: saturation.get(),
                    value: value.get(),
                    alpha: 1.0,
                }
                .to_color(),
            )
        })
    };
    let pick = {
        let (saturation, value) = (saturation.clone(), value.clone());
        Rc::new(move |at: Point| {
            let share = |pixels: f32| f64::from(pixels / SQUARE).clamp(0.0, 1.0);
            set_if_changed(&saturation, share(at.x - RING));
            set_if_changed(&value, share(SQUARE - (at.y - RING)));
        })
    };
    let keys = {
        let (saturation, value) = (saturation.clone(), value.clone());
        move |event: &KeyEvent| {
            if !event.pressed {
                return Handled::No;
            }
            // A step is a hundredth, so the marker lands where a drag can
            // reach too.
            let stepped = |now: f64, steps: f64| ((now * 100.0).round() + steps) / 100.0;
            let (sideways, upward) = match event.key {
                Key::ArrowLeft => (-1.0, 0.0),
                Key::ArrowRight => (1.0, 0.0),
                Key::ArrowUp => (0.0, 1.0),
                Key::ArrowDown => (0.0, -1.0),
                Key::PageUp => (0.0, 10.0),
                Key::PageDown => (0.0, -10.0),
                _ => return Handled::No,
            };
            if sideways != 0.0 {
                set_if_changed(
                    &saturation,
                    stepped(saturation.get(), sideways).clamp(0.0, 1.0),
                );
            }
            if upward != 0.0 {
                set_if_changed(&value, stepped(value.get(), upward).clamp(0.0, 1.0));
            }
            Handled::Yes
        }
    };
    let (press, drag) = (Rc::clone(&pick), pick);
    let shares = {
        let (saturation, value) = (saturation.clone(), value.clone());
        move || {
            let percent = |share: f64| (share * 100.0).round() as u32;
            format!(
                "Saturation {}%, brightness {}%",
                percent(saturation.get()),
                percent(value.get())
            )
        }
    };
    let (across, down) = (saturation.clone(), value.clone());
    let marker = panel()
        .absolute()
        .inset(move || Edges {
            left: Length::px((across.get() as f32) * SQUARE - MARKER / 2.0),
            top: Length::px((1.0 - down.get() as f32) * SQUARE - MARKER / 2.0),
            right: Length::Auto,
            bottom: Length::Auto,
        })
        .width(px(MARKER))
        .height(px(MARKER))
        .corner_radius(MARKER / 2.0)
        .border(1.0)
        .border_color(Color::rgb(0, 0, 0))
        .child(
            panel()
                .width(pct(100.0))
                .height(pct(100.0))
                .corner_radius(MARKER / 2.0)
                .border(2.0)
                .border_color(Color::WHITE),
        );
    column()
        .gap(Size {
            width: 0.0,
            height: 10.0,
        })
        .padding(px(12.0))
        .margin(px(2.0))
        .background(ColorToken::SURFACE)
        .border(1.0)
        .border_color(ColorToken::BORDER)
        .corner_radius(RadiusToken::MEDIUM)
        .trap_focus()
        .child(
            panel()
                .width(px(SQUARE + 2.0 * RING))
                .height(px(SQUARE + 2.0 * RING))
                .border(RING)
                .border_color(Color::TRANSPARENT)
                .corner_radius(RadiusToken::SMALL)
                .focused(|p| p.border_color(ColorToken::FOCUS_RING))
                // No one number says where the marker sits in two axes, so
                // the value names both shares instead of reporting a range.
                .role(Role::Slider)
                .accessible_label("Saturation and brightness")
                .accessible_value(shares)
                .focusable()
                .autofocus()
                .on_pointer_down(move |at| press(at))
                .on_drag(move |step| match step {
                    Drag::Moved(at) => {
                        drag(at);
                        Handled::Yes
                    }
                    _ => Handled::No,
                })
                .on_key(keys)
                .child(image(move || Some(square.get())).corner_radius(RadiusToken::SMALL))
                .child(marker),
        )
        .child(
            slider_with_track(
                hue,
                0.0..=360.0,
                1.0,
                track(image(move || Some(hue_image.clone()))),
            )
            .accessible_label("Hue"),
        )
        .child(
            slider_with_track(
                alpha,
                0.0..=1.0,
                0.01,
                track(image(move || Some(alpha_image.get()))),
            )
            .accessible_label("Alpha"),
        )
        .child(
            text_input(text)
                .height(px(28.0))
                .padding_xy(Size {
                    width: 8.0,
                    height: 4.0,
                })
                .background(ColorToken::SURFACE)
                .border(1.0)
                .border_color(ColorToken::BORDER)
                .corner_radius(RadiusToken::SMALL)
                .focused(|p| p.border_color(ColorToken::FOCUS_RING))
                .invalid(move || invalid.get())
                .role(Role::TextInput)
                .accessible_label("Hex color")
                .accessible_description("A color as #rrggbb or #rrggbbaa")
                .placeholder("#rrggbb"),
        )
}

/// A slider's track, which fills the rectangle the slider paints its own
/// track in.
fn track(paint: Element<ImageView>) -> Element<ImageView> {
    paint
        .absolute()
        .inset(Edges {
            left: Length::px(TRACK_INSET),
            right: Length::px(TRACK_INSET),
            top: Length::px(6.0),
            bottom: Length::Auto,
        })
        .height(px(TRACK))
        .corner_radius(TRACK / 2.0)
}

#[cfg(test)]
#[path = "color_picker_tests.rs"]
mod tests;
