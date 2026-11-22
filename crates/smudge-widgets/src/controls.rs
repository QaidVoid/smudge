//! Form controls bound to signals: a checkbox, a switch, a radio group, a
//! slider, a range slider and a number field.

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use smudge_core::element::{AnyElement, Element, IntoElement, Value};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::input::{ButtonEvent, Drag, Handled, Key, PointerButton};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role, ValueRange};
use smudge_core::style::{Align, Edges, Justify, Length, pct, px};
use smudge_core::theme::{ColorToken, ColorValue, RadiusToken};
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_core::window::Cursor;
use smudge_text::{PressHold, TextInputExt, label, text_input};

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
    // A division can leave the count a hair under its whole number: 0.3
    // in steps of 0.03 is ten steps, not nine. That hair rounds up.
    let whole_count = |count: f64| {
        let whole = count.round();
        // The hair stays under half a step, so a real half is never
        // rounding error.
        let hair = (count.abs() * f64::EPSILON * 8.0).min(0.25);
        if count != whole && (count - whole).abs() <= hair {
            whole
        } else {
            count
        }
    };
    let first = on_step(whole_count((min - origin) / step).ceil());
    let last = on_step(whole_count((max - origin) / step).floor());
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
/// Down by ten steps, and Home and End go to the ends of the range
/// whatever the step divides.
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
        move |key: &Key| -> Option<(f64, bool)> {
            let current = snap(value.get(), &range, step);
            match key {
                Key::ArrowLeft | Key::ArrowDown => Some((current - step, false)),
                Key::ArrowRight | Key::ArrowUp => Some((current + step, false)),
                Key::PageDown => Some((current - 10.0 * step, false)),
                Key::PageUp => Some((current + 10.0 * step, false)),
                Key::Home => Some((*range.start(), true)),
                Key::End => Some((*range.end(), true)),
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
        let Some((next, exact)) = keys(&event.key) else {
            return Handled::No;
        };
        let next = if exact {
            next
        } else {
            snap(next, &key_range, step)
        };
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

/// The pointer's offset from the start of a range slider's track, from its
/// offset from the node's corner. The surface keeps a transparent ring
/// reserve like every control, so its content starts one ring in.
fn along_track(x: f32) -> f32 {
    x - RING - TRACK_INSET
}

/// Builds one thumb of a [`RangeSlider`] at `share` percent of the way
/// along the lane both thumbs share.
fn range_thumb(
    share: impl Fn() -> f32 + 'static,
    label: Value<String>,
    announced: impl Fn() -> ValueRange + 'static,
    mut steps_with: impl FnMut(&Key) -> bool + 'static,
    mut grabs: impl FnMut(f32) + 'static,
    disabled: Value<bool>,
) -> Element<Panel> {
    panel()
        .absolute()
        .inset(move || Edges {
            left: Length::Percent(share()),
            top: px(0.0),
            right: Length::Auto,
            bottom: Length::Auto,
        })
        .width(px(2.0 * TRACK_INSET))
        .height(px(16.0))
        .corner_radius(TRACK_INSET)
        .background(ColorToken::SURFACE)
        .border(1.5)
        .border_color(ColorToken::ACCENT)
        .focused(|p| p.border_color(ColorToken::FOCUS_RING))
        .disabled_style(|p| p.opacity(0.5))
        .role(Role::Slider)
        .accessible_label(label)
        .accessible_range(announced)
        .disabled(disabled)
        .focusable()
        .on_pointer_down(move |at| grabs(at.x))
        .on_key(move |event| {
            if event.pressed && steps_with(&event.key) {
                Handled::Yes
            } else {
                Handled::No
            }
        })
}

/// The key of the binding that swaps a pair an outside write left crossed,
/// so no thumb sits past the other while the control is mounted.
struct Uncross;

/// The builder of a range slider, which [`range_slider`] creates and
/// [`RangeSlider::step`] and the naming and disabling methods fill.
pub struct RangeSlider {
    /// The signal that holds the low thumb's value.
    low: RwSignal<f64>,
    /// The signal that holds the high thumb's value.
    high: RwSignal<f64>,
    /// The range both thumbs move in.
    range: RangeInclusive<f64>,
    /// The size of one arrow step, or `None` for a hundredth of the range.
    step: Option<f64>,
    /// The names the thumbs announce, the low one first.
    names: Option<(Value<String>, Value<String>)>,
    /// The name the group announces, when the app gave one.
    name: Option<Value<String>>,
    /// Whether the control is disabled: the thumbs take neither the focus
    /// nor a press, and the pointer moves no thumb.
    disabled: Value<bool>,
}

impl RangeSlider {
    /// Returns the slider whose arrows and page keys land on steps of
    /// `step`, counted from the range's start, and whose presses and drags
    /// land on the nearest step. Home and End go to the ends of the range
    /// whatever the step divides. A negative step counts as its size. A
    /// step that is not finite and positive, and no step at all, is a
    /// hundredth of the range.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Names the thumbs, which a screen reader announces, instead of the
    /// defaults "minimum" and "maximum". The low thumb's name comes first.
    pub fn accessible_labels(
        mut self,
        low: impl Into<Value<String>>,
        high: impl Into<Value<String>>,
    ) -> Self {
        self.names = Some((low.into(), high.into()));
        self
    }

    /// Names the group that holds the thumbs, which a screen reader
    /// announces before them.
    pub fn accessible_label(mut self, name: impl Into<Value<String>>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Returns the slider with both thumbs disabled while `disabled` reads
    /// `true`: they take neither the focus nor a press, and the pointer
    /// moves no thumb.
    pub fn disabled(mut self, disabled: impl Into<Value<bool>>) -> Self {
        self.disabled = disabled.into();
        self
    }
}

impl IntoElement for RangeSlider {
    fn into_any(self) -> AnyElement {
        let RangeSlider {
            low,
            high,
            range,
            step,
            names,
            name,
            disabled,
        } = self;
        let step = step
            .map(|step| step.abs())
            .filter(|step| step.is_finite() && *step > 0.0)
            .unwrap_or_else(|| (*range.end() - *range.start()).abs() / 100.0);
        let (low_label, high_label) = names.unwrap_or_else(|| {
            (
                Value::Static(String::from("minimum")),
                Value::Static(String::from("maximum")),
            )
        });
        let width = Rc::new(Cell::new(0.0_f32));
        let grabbed: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let grab_offset: Rc<Cell<f32>> = Rc::new(Cell::new(0.0));
        let stopped = |high_side: bool, next: f64, stop_at: f64| {
            if !next.is_finite() {
                return next;
            }
            if high_side {
                next.max(stop_at)
            } else {
                next.min(stop_at)
            }
        };
        let share = {
            let (low, high, range) = (low.clone(), high.clone(), range.clone());
            move |high_side: bool| -> f32 {
                let value = if high_side { high.get() } else { low.get() };
                let (min, max) = (*range.start(), *range.end());
                let share = (value - min) / (max - min);
                if max > min && share.is_finite() {
                    (share.clamp(0.0, 1.0) * 100.0) as f32
                } else {
                    0.0
                }
            }
        };
        let low_share = {
            let share = share.clone();
            move || share(false)
        };
        let high_share = move || share(true);
        let (low_lane, high_lane) = (low_share.clone(), high_share.clone());
        let place = {
            let (low, high, range, width) =
                (low.clone(), high.clone(), range.clone(), Rc::clone(&width));
            move |high_side: bool, along: f32| {
                let across = width.get();
                if across <= 0.0 {
                    return;
                }
                let fraction = f64::from((along / across).clamp(0.0, 1.0));
                let (min, max) = (*range.start(), *range.end());
                let next = snap(min + fraction * (max - min), &range, step);
                let (mine, stop_at) = if high_side {
                    (high.clone(), low.get())
                } else {
                    (low.clone(), high.get())
                };
                let next = stopped(high_side, next, stop_at);
                if next.is_finite() && mine.get() != next {
                    mine.set(next);
                }
            }
        };
        let (press_place, drag_place) = (place.clone(), place);
        let nearer_high = {
            let (low, high, range, width) =
                (low.clone(), high.clone(), range.clone(), Rc::clone(&width));
            move |along: f32| -> bool {
                let across = width.get();
                if across <= 0.0 {
                    return false;
                }
                let fraction = f64::from((along / across).clamp(0.0, 1.0));
                let (min, max) = (*range.start(), *range.end());
                let at = min + fraction * (max - min);
                at - low.get() > high.get() - at
            }
        };
        let (press_near, drag_near) = (nearer_high.clone(), nearer_high);
        // Reports whether the key belongs to the thumb, so a thumb at its
        // stop still keeps the key from its ancestors.
        let step_at = {
            let (low, high, range) = (low.clone(), high.clone(), range.clone());
            move |high_side: bool, key: &Key| -> bool {
                let (mine, stop_at) = if high_side {
                    (high.clone(), low.get())
                } else {
                    (low.clone(), high.get())
                };
                let raw = mine.get();
                // An endpoint Home or End wrote can sit off the steps:
                // outward keys keep it, a step back lands on the nearest
                // step inside.
                let at_end = raw == *range.end();
                let current = snap(raw, &range, step);
                let off_grid = current != raw;
                let (headed, exact) = match key {
                    Key::ArrowRight | Key::ArrowUp | Key::PageUp if at_end => (raw, true),
                    Key::ArrowLeft | Key::ArrowDown if at_end && off_grid => (current, true),
                    Key::ArrowLeft | Key::ArrowDown => (current - step, false),
                    Key::ArrowRight | Key::ArrowUp => (current + step, false),
                    Key::PageDown => (current - 10.0 * step, false),
                    Key::PageUp => (current + 10.0 * step, false),
                    Key::Home => (*range.start(), true),
                    Key::End => (*range.end(), true),
                    _ => return false,
                };
                let next = if exact {
                    headed
                } else {
                    snap(headed, &range, step)
                };
                // A landing that a bound pulled back can lie behind the
                // value, so a page never carries the thumb the other way.
                let next = match key {
                    Key::PageDown if next > raw => raw,
                    Key::PageUp if next < raw => raw,
                    _ => next,
                };
                let next = stopped(high_side, next, stop_at);
                if next.is_finite() && mine.get() != next {
                    mine.set(next);
                }
                true
            }
        };
        let (low_steps, high_steps) = {
            let (step_at, step_at_high) = (step_at.clone(), step_at);
            (
                move |key: &Key| step_at(false, key),
                move |key: &Key| step_at_high(true, key),
            )
        };
        let (low_report, high_report) = (low.clone(), high.clone());
        let (low_bound, high_bound) = (low.clone(), high.clone());
        let (low_span, high_span) = (range.clone(), range);
        let (press_grab, drag_grab, low_grab_src, high_grab_src) = (
            grabbed.clone(),
            grabbed.clone(),
            grabbed.clone(),
            grabbed.clone(),
        );
        let (press_offset, drag_offset, low_offset_src, high_offset_src) = (
            grab_offset.clone(),
            grab_offset.clone(),
            grab_offset.clone(),
            grab_offset,
        );
        // A thumb hears its press in its own frame, where its center
        // sits half a thumb in from the left.
        let (low_grab, high_grab) = (
            move |pointer_x: f32| {
                low_grab_src.set(Some(false));
                low_offset_src.set(pointer_x - TRACK_INSET);
            },
            move |pointer_x: f32| {
                high_grab_src.set(Some(true));
                high_offset_src.set(pointer_x - TRACK_INSET);
            },
        );
        // The ring shows around the focused thumb, so the reserve stays
        // transparent.
        let mut area = Element::new(SliderArea {
            track: Rc::clone(&width),
        })
        .height(px(20.0))
        .min_width(px(80.0))
        .border(RING)
        .border_color(Color::TRANSPARENT)
        .disabled(disabled.clone())
        .disabled_style(|p| p.opacity(0.5))
        .role(Role::Group);
        if let Some(name) = name {
            area = area.accessible_label(name);
        }
        area.bind(
            Uncross,
            {
                let (low, high) = (low.clone(), high.clone());
                move || {
                    let (low_value, high_value) = (low.get(), high.get());
                    if low_value > high_value {
                        low.set(high_value);
                        high.set(low_value);
                    }
                }
            },
            |_, ()| smudge_core::tree::Dirty::Clean,
        )
        .keeps_focus()
        .on_pointer_down(move |at| {
            let along = along_track(at.x);
            let which = press_near(along);
            press_grab.set(Some(which));
            press_offset.set(0.0);
            press_place(which, along);
        })
        .on_drag(move |step| match step {
            Drag::Moved(at) => {
                let along = along_track(at.x) - drag_offset.get();
                let which = drag_grab.get().unwrap_or_else(|| {
                    let which = drag_near(along);
                    drag_grab.set(Some(which));
                    which
                });
                drag_place(which, along);
                Handled::Yes
            }
            // The drag is over, or another observer took it.
            _ => {
                drag_grab.set(None);
                drag_offset.set(0.0);
                Handled::No
            }
        })
        .child(
            panel()
                .absolute()
                .inset(Edges {
                    left: px(TRACK_INSET),
                    right: px(TRACK_INSET),
                    top: px(6.0),
                    bottom: Length::Auto,
                })
                .height(px(4.0))
                .corner_radius(2.0)
                .background(ColorToken::BORDER)
                .child(
                    panel()
                        .absolute()
                        .inset(move || Edges {
                            left: Length::Percent(low_share()),
                            right: Length::Percent(100.0 - high_share()),
                            top: px(0.0),
                            bottom: px(0.0),
                        })
                        .corner_radius(2.0)
                        .background(ColorToken::ACCENT),
                ),
        )
        .child(
            panel()
                .absolute()
                .inset(Edges {
                    left: px(0.0),
                    right: px(2.0 * TRACK_INSET),
                    top: px(0.0),
                    bottom: px(0.0),
                })
                // One lane for both circles, so a press lands on the circle under the pointer.
                .child(range_thumb(
                    low_lane,
                    low_label,
                    move || ValueRange {
                        value: low_report.get(),
                        min: *low_span.start(),
                        max: high_bound.get(),
                    },
                    low_steps,
                    low_grab,
                    disabled.clone(),
                ))
                .child(range_thumb(
                    high_lane,
                    high_label,
                    move || ValueRange {
                        value: high_report.get(),
                        min: low_bound.get(),
                        max: *high_span.end(),
                    },
                    high_steps,
                    high_grab,
                    disabled,
                )),
        )
        .into_any()
    }
}

/// Creates a range slider that picks the low and the high of a pair, bound
/// to `low` and `high`, with both thumbs moving in `range`.
///
/// The two thumbs are one track with the stretch between them filled: a
/// press on the track moves the thumb it lies nearer to, and a drag moves
/// the thumb it grabbed. No thumb crosses the other: a drag or a step that
/// would stops at the thumb it meets, and a write from outside that
/// crosses the pair swaps the two back. A press on the track leaves the
/// keyboard focus where it is, so a thumb that holds it keeps it.
///
/// Each thumb is its own Tab stop, and the arrows step the thumb the
/// keyboard is on, Page Up and Page Down by ten steps, and Home and End to
/// the ends of the range. The arrows and the page keys land on steps of
/// [`RangeSlider::step`], counted from the range's start, and a press or a
/// drag lands on the nearest step, or on a hundredth of the range when the
/// app set no step. A thumb that meets the other stops at whatever value
/// the other holds, which an app can seed off the steps. The range's ends
/// should be finite numbers. A screen reader hears a group holding the two
/// thumbs, each a slider with its value and the stretch it can still move
/// in, bounded by the other thumb, named "minimum" and "maximum" unless
/// the app names them with [`RangeSlider::accessible_labels`]. Name the
/// group itself with [`RangeSlider::accessible_label`]. The builder sets
/// no style of its own, so the parent that holds it gives it its size.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::range_slider;
///
/// let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
/// let filter = range_slider(low, high, 10.0..=100.0).step(5.0);
/// ```
#[track_caller]
pub fn range_slider(
    low: RwSignal<f64>,
    high: RwSignal<f64>,
    range: RangeInclusive<f64>,
) -> RangeSlider {
    RangeSlider {
        low,
        high,
        range,
        step: None,
        names: None,
        name: None,
        disabled: Value::Static(false),
    }
}

#[cfg(test)]
#[path = "controls_tests.rs"]
mod tests;

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

/// How far a press must travel before it becomes a drag of a number
/// field's number, in logical pixels.
const DRAG_START: f32 = 4.0;

/// How much a held Shift slows a number field's drag, tenfold.
const SLOW_DRAG: f64 = 0.1;

/// The horizontal drag a number field with a [`NumberField::drag_speed`]
/// turns into value changes, and the hold that keeps its text out of the
/// press until the press has decided.
struct FieldDrag {
    /// The hold the input keeps the press under while the field decides.
    hold: PressHold,
    /// Where the press began, relative to the field's text.
    press: Cell<Point>,
    /// Whether the press became a drag.
    dragging: Cell<bool>,
    /// The x and the number the drag moves from, retaken when Shift joins
    /// or leaves, so slowing a drag never moves the number back.
    anchor: Cell<(f32, f64)>,
    /// Whether the anchor was taken with Shift held.
    slow: Cell<bool>,
    /// The x of the last move, which a change of speed anchors from.
    last: Cell<f32>,
    /// The number change of one logical pixel of drag.
    speed: f64,
    /// The signal that holds the number.
    value: RwSignal<f64>,
    /// The range the number moves in.
    range: RangeInclusive<f64>,
    /// The size of one step.
    step: f64,
}

impl FieldDrag {
    /// Watches a drag that changes `value` of `range` in steps of `step`,
    /// at `speed` per logical pixel.
    fn new(speed: f32, value: RwSignal<f64>, range: RangeInclusive<f64>, step: f64) -> FieldDrag {
        FieldDrag {
            hold: PressHold::new(),
            press: Cell::new(Point { x: 0.0, y: 0.0 }),
            dragging: Cell::new(false),
            anchor: Cell::new((0.0, 0.0)),
            slow: Cell::new(false),
            last: Cell::new(0.0),
            speed: f64::from(speed),
            value,
            range,
            step,
        }
    }

    /// Takes the press over, so the input holds its caret back.
    fn press(&self, event: &ButtonEvent) -> Handled {
        if event.button != PointerButton::Primary {
            return Handled::No;
        }
        self.press.set(event.position);
        self.dragging.set(false);
        self.hold.claim();
        Handled::Yes
    }

    /// Moves the number once the press has travelled far enough, by the
    /// speed times the distance from the anchor, stepped and clamped.
    ///
    /// A change of speed anchors at the last position, so the move that
    /// brought the change travels on at the speed it names.
    fn drag(&self, at: Point, slow_now: bool) {
        if !self.hold.is_claimed() {
            return;
        }
        if !self.dragging.get() {
            let from = self.press.get();
            if (at.x - from.x).hypot(at.y - from.y) <= DRAG_START {
                return;
            }
            self.dragging.set(true);
            self.last.set(from.x);
            self.anchor.set((from.x, self.value.get()));
            self.slow.set(slow_now);
        }
        if slow_now != self.slow.get() {
            self.anchor.set((self.last.get(), self.value.get()));
            self.slow.set(slow_now);
        }
        let (from_x, base) = self.anchor.get();
        let rate = self.speed * if slow_now { SLOW_DRAG } else { 1.0 };
        let next = snap(
            base + f64::from(at.x - from_x) * rate,
            &self.range,
            self.step,
        );
        if next.is_finite() && self.value.get() != next {
            self.value.set(next);
        }
        self.last.set(at.x);
    }

    /// Ends the hold: a drag puts it down, a still press places the caret.
    fn release(&self) {
        if !self.hold.is_claimed() {
            return;
        }
        if self.dragging.get() {
            self.hold.release();
        } else {
            self.hold.click();
        }
    }

    /// Puts a drag down that ended without reaching the field, as when the
    /// window lost the focus while the button was held. A hold a press has
    /// just taken stays, because the end belongs to the drag it replaces.
    fn dropped(&self) {
        if self.dragging.get() {
            self.dragging.set(false);
            self.hold.release();
        }
    }
}

/// The builder of a number field, which [`number_field`] creates and
/// [`NumberField::drag_speed`] fills.
pub struct NumberField {
    /// The name the field announces.
    name: String,
    /// The signal that holds the number.
    value: RwSignal<f64>,
    /// The range the number moves in.
    range: RangeInclusive<f64>,
    /// The size of one step.
    step: f64,
    /// The number change of one logical pixel of horizontal drag, or
    /// `None` for a field that does not drag.
    drag_speed: Option<f32>,
}

impl NumberField {
    /// Returns the field whose number a horizontal drag across its text
    /// changes by `speed` per logical pixel.
    ///
    /// A press that moves more than four logical pixels before release
    /// becomes a drag: the number follows the pointer from the number the
    /// press began with, clamped and stepped, and the field does not take
    /// the text caret. A press that stays still edits the text as before.
    /// Shift slows the drag tenfold, for fine changes, and the field shows
    /// the column resize cursor over its text while it can be dragged. A
    /// negative speed counts as its size, and a speed of no use, zero or
    /// one that is not finite, leaves the field without a drag.
    pub fn drag_speed(mut self, speed: f32) -> Self {
        self.drag_speed = (speed.is_finite() && speed != 0.0).then(|| speed.abs());
        self
    }
}

impl IntoElement for NumberField {
    fn into_any(self) -> AnyElement {
        let NumberField {
            name,
            value,
            range,
            step,
            drag_speed,
        } = self;
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
        let mut input = text_input(text)
            .role(Role::SpinButton)
            .accessible_label(name)
            .accessible_range(move || ValueRange {
                value: announced.get(),
                min: *reported.start(),
                max: *reported.end(),
            })
            .grow(1.0)
            .min_width(px(48.0));
        let drag =
            drag_speed.map(|speed| Rc::new(FieldDrag::new(speed, value.clone(), range, step)));
        if let Some(drag) = &drag {
            input = input
                .cursor(Cursor::ColumnResize)
                .press_hold(drag.hold.clone());
        }
        // The wheel leaves a held drag alone: the drag owns the number, and
        // a step it took would be lost to the next move from the anchor.
        let wheel_held = drag.as_ref().map(|drag| drag.hold.clone());
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
        let (key_nudge, wheel_nudge, blur) =
            (Rc::clone(&nudge), Rc::clone(&nudge), Rc::clone(&commit));
        let (track_focus, wheel_focus) = (focused.clone(), focused);
        // The wheel's distance not yet turned into a step.
        let turned = Rc::new(Cell::new(0.0_f32));
        let mut surface = row().grow(1.0).padding_xy(Size {
            width: 8.0,
            height: 4.0,
        });
        if let Some(drag) = drag {
            let (press_drag, move_drag, up_drag, ended_drag) = (
                Rc::clone(&drag),
                Rc::clone(&drag),
                Rc::clone(&drag),
                Rc::clone(&drag),
            );
            let step_drag = Rc::clone(&drag);
            surface = surface
                .on_button_down(move |event| press_drag.press(&event))
                .on_button_move(move |event| move_drag.drag(event.position, event.modifiers.shift))
                .on_button_up(move |_| up_drag.release())
                .on_drag(move |step| match step {
                    Drag::Moved(_) if step_drag.dragging.get() => Handled::Yes,
                    Drag::Ended => {
                        ended_drag.dropped();
                        Handled::No
                    }
                    _ => Handled::No,
                });
        }
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
                if wheel_held.as_ref().is_some_and(|hold| hold.is_claimed()) {
                    return Handled::No;
                }
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
            .child(surface.child(input.placeholder("0")))
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
            .into_any()
    }
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
/// The field does not drag until [`NumberField::drag_speed`] says how
/// fast.
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
) -> NumberField {
    NumberField {
        name: name.into(),
        value,
        range,
        step,
        drag_speed: None,
    }
}

/// One choice of a [`RadioGroup`].
struct Choice<T> {
    /// The value the group's signal holds while this choice is the chosen
    /// one.
    value: T,
    /// The choice's text.
    label: String,
    /// Whether the choice is disabled, so it can be neither chosen nor
    /// focused, and the arrows pass it over.
    disabled: Value<bool>,
}

/// The builder of a radio group, which [`radio_group`] creates and
/// [`RadioGroup::option`] fills.
pub struct RadioGroup<T: PartialEq + Clone + 'static> {
    /// The signal that holds the chosen value.
    selected: RwSignal<T>,
    /// The choices added so far, in order.
    choices: Vec<Choice<T>>,
    /// Whether the choices lay out in a row instead of a column.
    horizontal: bool,
    /// The name the app gives the group, or `None` for a group a screen
    /// reader announces by its choices alone.
    name: Option<Value<String>>,
}

impl<T: PartialEq + Clone + 'static> RadioGroup<T> {
    /// Returns the group with a choice of `value`, shown as `label`, added
    /// at the end.
    ///
    /// The choices keep the order they were added in, which the arrows walk
    /// and the projection reports each choice's place by.
    pub fn option(mut self, value: T, label: impl Into<String>) -> Self {
        self.choices.push(Choice {
            value,
            label: label.into(),
            disabled: Value::Static(false),
        });
        self
    }

    /// Returns the group with a choice of `value`, shown as `label`, added
    /// at the end, that is disabled while `disabled` reads `true`: it can
    /// be neither chosen nor focused, and the arrows pass it over.
    pub fn option_disabled(
        mut self,
        value: T,
        label: impl Into<String>,
        disabled: impl Into<Value<bool>>,
    ) -> Self {
        self.choices.push(Choice {
            value,
            label: label.into(),
            disabled: disabled.into(),
        });
        self
    }

    /// Returns the group with its choices laid out in a row instead of a
    /// column.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Names the group, which a screen reader announces before its choices.
    pub fn accessible_label(mut self, name: impl Into<Value<String>>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl<T: PartialEq + Clone + 'static> IntoElement for RadioGroup<T> {
    fn into_any(self) -> AnyElement {
        let RadioGroup {
            selected,
            choices,
            horizontal,
            name,
        } = self;
        let count = choices.len();
        let mut group = if horizontal {
            row().gap(Size {
                width: 16.0,
                height: 0.0,
            })
        } else {
            column().gap(Size {
                width: 0.0,
                height: 4.0,
            })
        };
        group = group
            .role(Role::Group)
            .accessible_set_size(count)
            .navigate_with_all_arrows();
        if let Some(name) = name {
            group = group.accessible_label(name);
        }
        for (index, choice) in choices.into_iter().enumerate() {
            group = group.child(radio_option(choice, index, selected.clone()));
        }
        group.into_any()
    }
}

/// Builds one option of a radio group: a circle with a dot while it is the
/// chosen one, and a label a press chooses by.
fn radio_option<T: PartialEq + Clone + 'static>(
    choice: Choice<T>,
    index: usize,
    selected: RwSignal<T>,
) -> Element<Panel> {
    let Choice {
        value,
        label: text,
        disabled,
    } = choice;
    let (fill, stroke, dot, checked, picked, press) = (
        selected.clone(),
        selected.clone(),
        selected.clone(),
        selected.clone(),
        selected.clone(),
        selected.clone(),
    );
    let (fill_value, stroke_value, dot_value, checked_value, picked_value) = (
        value.clone(),
        value.clone(),
        value.clone(),
        value.clone(),
        value.clone(),
    );
    let (press_value, chosen) = (value.clone(), value);
    let indicator = row()
        .width(px(16.0))
        .height(px(16.0))
        .shrink(0.0)
        .justify_content(Justify::Center)
        .align_items(Align::Center)
        .corner_radius(8.0)
        .border(1.5)
        .background(move || {
            ColorValue::from(if fill.get() == fill_value {
                ColorToken::ACCENT
            } else {
                ColorToken::SURFACE
            })
        })
        .border_color(move || {
            ColorValue::from(if stroke.get() == stroke_value {
                ColorToken::ACCENT
            } else {
                ColorToken::BORDER
            })
        })
        .child(
            panel()
                .width(px(8.0))
                .height(px(8.0))
                .shrink(0.0)
                .corner_radius(4.0)
                .background(move || {
                    if dot.get() == dot_value {
                        ColorValue::from(ColorToken::ACCENT_TEXT)
                    } else {
                        ColorValue::Literal(Color::TRANSPARENT)
                    }
                }),
        );
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
        .hover(|p| p.background(ColorToken::SURFACE_HOVER))
        .pressed(|p| p.background(ColorToken::SURFACE_PRESSED))
        .focused(|p| p.border_color(ColorToken::FOCUS_RING))
        .disabled_style(|p| p.opacity(0.5))
        .role(Role::RadioButton)
        .accessible_label(text.clone())
        .accessible_checked(move || Checked::from(checked.get() == checked_value))
        .accessible_selected(move || picked.get() == picked_value)
        .accessible_position(index)
        .disabled(disabled)
        .focusable()
        .activate_on_keys()
        .on_press(move || press.set(press_value.clone()))
        .on_arrow_focus(move || selected.set(chosen.clone()))
        .child(indicator)
        .child(label(text))
}

/// Creates a radio group whose chosen value `selected` holds, which
/// [`RadioGroup::option`] fills with choices.
///
/// The group is one Tab stop: Tab enters it at the chosen choice, or at the
/// first when no choice matches `selected`, and the next Tab leaves it. The
/// arrow keys move the choice and the focus together, Down and Right to the
/// next choice and Up and Left to the one before it, wrapping at the ends,
/// and a press on a choice, or Enter or Space on the focused one, chooses
/// it. An arrow whose choice already holds the focus, because it is the
/// only one enabled or the walk has come back around to it, still chooses
/// it. A disabled choice can be neither chosen nor focused, and the arrows
/// pass it over, as does an arrow that reaches a group whose focused choice
/// was just disabled. A screen reader hears a group, named by
/// [`RadioGroup::accessible_label`], of radio buttons, the chosen one
/// checked, each with its place in the set of choices.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::radio_group;
///
/// # #[derive(Clone, PartialEq)]
/// # enum Size { Small, Large }
/// let size = RwSignal::new(Size::Small);
/// let group = radio_group(size)
///     .option(Size::Small, "Small")
///     .option(Size::Large, "Large")
///     .accessible_label("Brush size");
/// ```
#[track_caller]
pub fn radio_group<T: PartialEq + Clone + 'static>(selected: RwSignal<T>) -> RadioGroup<T> {
    RadioGroup {
        selected,
        choices: Vec::new(),
        horizontal: false,
        name: None,
    }
}
