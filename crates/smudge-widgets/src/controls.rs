//! Form controls bound to signals: a checkbox, a switch, a slider and a
//! number field.

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Size};
use smudge_core::input::{Drag, Handled, Key};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role, ValueRange};
use smudge_core::style::{Align, Edges, Justify, Length, pct, px};
use smudge_core::theme::{ColorToken, ColorValue, RadiusToken};
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_text::{TextInputExt, label, text_input};

use crate::containers::{column, row};
use crate::marks::{Shape, mark};

/// The width of the focus ring around a control, which every control keeps
/// room for so the ring appears without moving it.
const RING: f32 = 2.0;

/// Builds the row a labeled toggle lays out in: `indicator`, then the
/// label, with room for the focus ring, focusable and pressed by Enter and
/// Space.
fn toggle_row(
    text: String,
    role: Role,
    on: RwSignal<bool>,
    indicator: Element<Panel>,
) -> Element<Panel> {
    let (state, flip) = (on.clone(), on);
    row()
        .gap(Size {
            width: 8.0,
            height: 0.0,
        })
        .align_items(Align::Center)
        .padding(px(2.0))
        .border(RING)
        .border_color(Color::TRANSPARENT)
        .corner_radius(RadiusToken::SMALL)
        .focused(|p| p.border_color(ColorToken::FOCUS_RING))
        .disabled_style(|p| p.opacity(0.5))
        .role(role)
        .accessible_label(text.clone())
        .accessible_checked(move || Checked::from(state.get()))
        .focusable()
        .activate_on_keys()
        .on_press(move || flip.update(|on| *on = !*on))
        .child(indicator)
        .child(label(text))
}

/// Creates a checkbox labeled `text` that shows and toggles `checked`.
///
/// A press, Enter or Space toggles it, and a screen reader hears a checkbox
/// that is checked or not.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::checkbox;
///
/// let grid = RwSignal::new(true);
/// let toggle = checkbox("Show the grid", grid);
/// ```
#[track_caller]
pub fn checkbox(text: impl Into<String>, checked: RwSignal<bool>) -> Element<Panel> {
    let (fill, border, tick) = (checked.clone(), checked.clone(), checked.clone());
    let indicator = row()
        .width(px(16.0))
        .height(px(16.0))
        .shrink(0.0)
        .justify_content(Justify::Center)
        .align_items(Align::Center)
        .border(1.5)
        .corner_radius(RadiusToken::SMALL)
        .background(move || {
            ColorValue::from(if fill.get() {
                ColorToken::ACCENT
            } else {
                ColorToken::SURFACE
            })
        })
        .border_color(move || {
            ColorValue::from(if border.get() {
                ColorToken::ACCENT
            } else {
                ColorToken::BORDER
            })
        })
        .child(mark(Shape::Check, 12.0, move || tick.get()).text_color(ColorToken::ACCENT_TEXT));
    toggle_row(text.into(), Role::CheckBox, checked, indicator)
}

/// Creates a switch labeled `text` that shows and toggles `on`.
///
/// A press, Enter or Space toggles it, and a screen reader hears a switch
/// that is on or off.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::switch;
///
/// let snap = RwSignal::new(false);
/// let toggle = switch("Snap to grid", snap);
/// ```
#[track_caller]
pub fn switch(text: impl Into<String>, on: RwSignal<bool>) -> Element<Panel> {
    let (track, side) = (on.clone(), on.clone());
    let indicator = row()
        .width(px(32.0))
        .height(px(18.0))
        .shrink(0.0)
        .padding(px(2.0))
        .corner_radius(9.0)
        .align_items(Align::Center)
        .justify_content(move || {
            if side.get() {
                Justify::End
            } else {
                Justify::Start
            }
        })
        .background(move || {
            ColorValue::from(if track.get() {
                ColorToken::ACCENT
            } else {
                ColorToken::BORDER
            })
        })
        .child(
            panel()
                .width(px(14.0))
                .height(px(14.0))
                .corner_radius(7.0)
                .background(ColorToken::SURFACE),
        );
    toggle_row(text.into(), Role::Switch, on, indicator)
}

/// Returns `value` moved onto the nearest step of `step` that lies in
/// `range`, counting the steps from the range's start, or from zero when
/// the range has no finite start.
///
/// A NaN end of the range counts as no bound, an end below the start as
/// the start, and a NaN value as the start. A step that is not finite and
/// positive leaves the value where it is, inside the range.
fn snap(value: f64, range: &RangeInclusive<f64>, step: f64) -> f64 {
    let (min, max) = (*range.start(), *range.end());
    let min = if min.is_nan() { f64::NEG_INFINITY } else { min };
    let max = if max.is_nan() {
        f64::INFINITY
    } else {
        max.max(min)
    };
    let origin = if min.is_finite() { min } else { 0.0 };
    let value = if value.is_nan() { origin } else { value };
    if !(step.is_finite() && step > 0.0) {
        return value.clamp(min, max);
    }
    let on_step = |count: f64| {
        let stepped = origin + count * step;
        // Adding steps up leaves a trace of float error, which would show as
        // digits the step never had.
        if stepped.abs() < 1e6 {
            (stepped * 1e9).round() / 1e9
        } else {
            stepped
        }
    };
    let first = on_step(((min - origin) / step).ceil());
    let last = on_step(((max - origin) / step).floor());
    if first.is_nan() || last.is_nan() || first > last {
        return value.clamp(min, max);
    }
    on_step(((value - origin) / step).round()).clamp(first, last)
}

/// How far the wheel turns for one step of a number field, a line of a
/// mouse wheel, in logical pixels.
const WHEEL_LINE: f32 = 40.0;

/// How far the track of a slider stays from its sides, half the thumb, so
/// the thumb stays inside at both ends, in logical pixels.
const TRACK_INSET: f32 = 8.0;

/// The part of a slider that measures its own width, so a drag along it
/// knows where the pointer is as a share of the track.
pub struct SliderArea {
    /// The length of the track, as the last paint found it.
    track: Rc<Cell<f32>>,
}

impl Widget for SliderArea {
    /// Records the length of the track, and draws nothing of its own.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        self.track.set(cx.content.width() - 2.0 * TRACK_INSET);
        Panel.paint(cx, out);
    }
}

/// Creates a slider that picks a number of `range` in steps of `step`,
/// bound to `value`.
///
/// A press or a drag along the track sets the number under the pointer.
/// Left and Down step it down, Right and Up step it up, Page Up and Page
/// Down by ten steps, and Home and End go to the first and the last step.
/// Every number it sets lies on a step and inside the range, a negative
/// step counts as its size, and the steps count from the range's start. A
/// screen reader hears a slider with its number and range. Give it a name
/// with [`accessible_label`](Element::accessible_label).
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::slider;
///
/// let opacity = RwSignal::new(0.5);
/// let control = slider(opacity, 0.0..=1.0, 0.05).accessible_label("Opacity");
/// ```
#[track_caller]
pub fn slider(value: RwSignal<f64>, range: RangeInclusive<f64>, step: f64) -> Element<SliderArea> {
    let step = step.abs();
    let width = Rc::new(Cell::new(0.0_f32));
    // The pointer's offset from the start of the track, from its offset
    // from the node's corner.
    let along = |x: f32| x - RING - TRACK_INSET;
    let share = {
        let (value, range) = (value.clone(), range.clone());
        move || {
            let (min, max) = (*range.start(), *range.end());
            let share = (value.get() - min) / (max - min);
            if max > min && share.is_finite() {
                share.clamp(0.0, 1.0) as f32 * 100.0
            } else {
                0.0
            }
        }
    };
    let set_at = {
        let (value, range, width) = (value.clone(), range.clone(), Rc::clone(&width));
        move |x: f32| {
            let across = width.get();
            if across <= 0.0 {
                return;
            }
            let fraction = f64::from((x / across).clamp(0.0, 1.0));
            let (min, max) = (*range.start(), *range.end());
            let next = snap(min + fraction * (max - min), &range, step);
            if next.is_finite() && value.get() != next {
                value.set(next);
            }
        }
    };
    let (press_at, drag_at) = (set_at.clone(), set_at);
    let keys = {
        let (value, range) = (value.clone(), range.clone());
        move |key: &Key| -> Option<f64> {
            let current = snap(value.get(), &range, step);
            match key {
                Key::ArrowLeft | Key::ArrowDown => Some(current - step),
                Key::ArrowRight | Key::ArrowUp => Some(current + step),
                Key::PageDown => Some(current - 10.0 * step),
                Key::PageUp => Some(current + 10.0 * step),
                Key::Home => Some(*range.start()),
                Key::End => Some(*range.end()),
                _ => None,
            }
        }
    };
    let (fill_share, thumb_share) = (share.clone(), share);
    let (announced, key_value, key_range) = (value.clone(), value.clone(), range.clone());
    let reported = range.clone();
    Element::new(SliderArea {
        track: Rc::clone(&width),
    })
    .height(px(20.0))
    .min_width(px(80.0))
    .border(RING)
    .border_color(Color::TRANSPARENT)
    .corner_radius(RadiusToken::SMALL)
    .focused(|p| p.border_color(ColorToken::FOCUS_RING))
    .disabled_style(|p| p.opacity(0.5))
    .role(Role::Slider)
    .accessible_range(move || ValueRange {
        value: announced.get(),
        min: *reported.start(),
        max: *reported.end(),
    })
    .focusable()
    .on_pointer_down(move |at| press_at(along(at.x)))
    .on_drag(move |step| match step {
        Drag::Moved(at) => {
            drag_at(along(at.x));
            Handled::Yes
        }
        _ => Handled::No,
    })
    .on_key(move |event| {
        if !event.pressed {
            return Handled::No;
        }
        let Some(next) = keys(&event.key) else {
            return Handled::No;
        };
        let next = snap(next, &key_range, step);
        if next.is_finite() && key_value.get() != next {
            key_value.set(next);
        }
        Handled::Yes
    })
    .child(
        panel()
            .absolute()
            .inset(Edges {
                left: Length::px(TRACK_INSET),
                right: Length::px(TRACK_INSET),
                top: Length::px(6.0),
                bottom: Length::Auto,
            })
            .height(px(4.0))
            .corner_radius(2.0)
            .background(ColorToken::BORDER)
            .child(
                panel()
                    .height(pct(100.0))
                    .width(move || pct(fill_share()))
                    .corner_radius(2.0)
                    .background(ColorToken::ACCENT),
            ),
    )
    // The thumb moves along a lane as long as the track, so its middle sits
    // over the track's point for the value.
    .child(
        panel()
            .absolute()
            .inset(Edges {
                left: Length::px(0.0),
                right: Length::px(2.0 * TRACK_INSET),
                top: Length::px(0.0),
                bottom: Length::px(0.0),
            })
            .child(
                panel()
                    .absolute()
                    .inset(move || Edges {
                        left: Length::Percent(thumb_share()),
                        top: Length::px(0.0),
                        right: Length::Auto,
                        bottom: Length::Auto,
                    })
                    .width(px(2.0 * TRACK_INSET))
                    .height(px(16.0))
                    .corner_radius(TRACK_INSET)
                    .background(ColorToken::SURFACE)
                    .border(1.5)
                    .border_color(ColorToken::ACCENT),
            ),
    )
}

/// The key of the binding that shows an outside write to a number field's
/// number in its text.
struct Follow;

/// Writes `value` with as many decimals as `step` has.
fn format_number(value: f64, step: f64) -> String {
    let step = format!("{step}");
    let decimals = step
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    format!("{value:.decimals$}")
}

/// Creates a field named `name` that holds a number of `range`, in steps
/// of `step`, bound to `value`.
///
/// The text can be typed. It is read when Enter is pressed and when the
/// focus leaves the field after an edit: a number is moved onto a step of
/// the range, as a [`slider`] does, and text that is no number puts the
/// field back to `value`. Up and Down step the number, Page Up and Page
/// Down by ten steps, and so do the arrows beside it and the wheel while
/// the field holds the focus, a step for each line it turns. A screen
/// reader hears a spin button called `name` with its number and range.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::number_field;
///
/// let width = RwSignal::new(12.0);
/// let field = number_field("Width", width, 1.0..=512.0, 1.0);
/// ```
#[track_caller]
pub fn number_field(
    name: impl Into<String>,
    value: RwSignal<f64>,
    range: RangeInclusive<f64>,
    step: f64,
) -> Element<Panel> {
    let step = step.abs();
    let text = crate::signal(format_number(value.get(), step));
    let focused = crate::signal(false);
    let (written, shown_text) = (value.clone(), text.clone());
    let commit = {
        let (value, text, range) = (value.clone(), text.clone(), range.clone());
        Rc::new(move || {
            // Text nobody edited reads the number as it is, even one off the
            // steps written from outside.
            if text.with(|text| *text == format_number(value.get(), step)) {
                return;
            }
            let typed = text.with(|text| text.trim().parse::<f64>().ok());
            match typed.filter(|typed| typed.is_finite()) {
                Some(typed) => {
                    let next = snap(typed, &range, step);
                    if value.get() != next {
                        value.set(next);
                    }
                    text.set(format_number(next, step));
                }
                None => text.set(format_number(value.get(), step)),
            }
        })
    };
    let nudge = {
        let (value, text, range) = (value.clone(), text.clone(), range.clone());
        let commit = Rc::clone(&commit);
        Rc::new(move |steps: f64| {
            commit();
            let next = snap(snap(value.get(), &range, step) + steps * step, &range, step);
            if value.get() != next {
                value.set(next);
            }
            // The text shows the step at once, so a second step in the same
            // frame reads it rather than the number before.
            text.set(format_number(next, step));
        })
    };
    let (announced, reported) = (value.clone(), range.clone());
    let input = text_input(text)
        .role(Role::SpinButton)
        .accessible_label(name.into())
        .accessible_range(move || ValueRange {
            value: announced.get(),
            min: *reported.start(),
            max: *reported.end(),
        })
        .grow(1.0)
        .min_width(px(48.0));
    let stepper = |shape: Shape, steps: f64| {
        let nudge = Rc::clone(&nudge);
        row()
            .grow(1.0)
            .padding_xy(Size {
                width: 4.0,
                height: 0.0,
            })
            .align_items(Align::Center)
            .hover(|p| p.background(ColorToken::SURFACE_HOVER))
            .pressed(|p| p.background(ColorToken::SURFACE_PRESSED))
            .keeps_focus()
            .on_press(move || nudge(steps))
            .child(mark(shape, 10.0, true).text_color(ColorToken::TEXT_MUTED))
    };
    let (key_nudge, wheel_nudge, blur) = (Rc::clone(&nudge), Rc::clone(&nudge), Rc::clone(&commit));
    let (track_focus, wheel_focus) = (focused.clone(), focused);
    // The wheel's distance not yet turned into a step.
    let turned = Rc::new(Cell::new(0.0_f32));
    row()
        .align_items(Align::Stretch)
        .background(ColorToken::SURFACE)
        .border(1.0)
        .border_color(ColorToken::BORDER)
        .corner_radius(RadiusToken::MEDIUM)
        // An outside write to the number shows in the text, unless the
        // text already reads that number.
        .bind(
            Follow,
            move || {
                let number = written.get();
                // Untracked, so typing, which writes the text, never wakes
                // this and puts the number back.
                let shown = shown_text.read(|text| text.trim().parse::<f64>().ok());
                if shown != Some(number) {
                    shown_text.set(format_number(number, step));
                }
            },
            |_, ()| smudge_core::tree::Dirty::Clean,
        )
        .on_focus_within(move |inside| {
            track_focus.set(inside);
            if !inside {
                blur();
            }
        })
        .on_key(move |event| {
            if !event.pressed {
                return Handled::No;
            }
            let steps = match event.key {
                Key::ArrowUp => 1.0,
                Key::ArrowDown => -1.0,
                Key::PageUp => 10.0,
                Key::PageDown => -10.0,
                Key::Enter => {
                    key_nudge(0.0);
                    return Handled::Yes;
                }
                _ => return Handled::No,
            };
            key_nudge(steps);
            Handled::Yes
        })
        .on_wheel(move |delta| {
            if !wheel_focus.get() || delta.y == 0.0 || !delta.y.is_finite() {
                return Handled::No;
            }
            let mut sum = turned.get();
            if sum * delta.y < 0.0 {
                sum = 0.0;
            }
            sum += delta.y;
            let lines = (sum / WHEEL_LINE).trunc();
            turned.set(sum - lines * WHEEL_LINE);
            if lines != 0.0 {
                wheel_nudge(-f64::from(lines));
            }
            Handled::Yes
        })
        .child(
            row()
                .grow(1.0)
                .padding_xy(Size {
                    width: 8.0,
                    height: 4.0,
                })
                .child(input.placeholder("0")),
        )
        .child(
            column()
                .width(px(18.0))
                .border_edges(Edges {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 1.0,
                })
                .border_color(ColorToken::BORDER)
                .child(stepper(Shape::Up, 1.0))
                .child(stepper(Shape::Down, -1.0)),
        )
}
