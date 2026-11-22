//! Buttons, labels, a text input, sliders, a number field, a progress
//! bar, a spinner and a scroll view, to try accessibility by hand.
//!
//! Run with `cargo run --example controls -p smudge-platform`. A screen
//! reader announces the two buttons, the field, the sliders, the number
//! field, the progress bar and the list this window shows: press the
//! buttons with the keyboard, type into the field, step the sliders with
//! the arrows and drag or press their tracks, step or drag the number
//! field, while the reader runs to hear what changes.

use std::process::ExitCode;

use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::style::{Align, Length, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_platform::{WindowOptions, run_with};
use smudge_text::TextInputExt;
use smudge_text::label;
use smudge_text::text_input;
use smudge_widgets::ButtonExt;
use smudge_widgets::button;
use smudge_widgets::column;
use smudge_widgets::number_field;
use smudge_widgets::progress;
use smudge_widgets::range_slider;
use smudge_widgets::row;
use smudge_widgets::scroll;
use smudge_widgets::slider;
use smudge_widgets::spinner;

/// The height of one input field, in logical pixels.
const FIELD_HEIGHT: f32 = 32.0;

/// The height of the scroll view, in logical pixels.
const VIEW_HEIGHT: f32 = 180.0;

fn main() -> ExitCode {
    let build = || {
        let clicks = RwSignal::new(0u32);
        let typed = RwSignal::new(String::new());
        let opacity = RwSignal::new(0.5_f64);
        let (low, high) = (RwSignal::new(20.0), RwSignal::new(80.0));
        let size = RwSignal::new(12.0_f64);
        let done = RwSignal::new(Some(0.6_f32));
        let press_clicks = clicks.clone();
        let reset_clicks = clicks.clone();
        let read_clicks = clicks.clone();
        let read_typed = typed.clone();
        let read_opacity = opacity.clone();
        let read_low = low.clone();
        let read_high = high.clone();
        let read_size = size.clone();
        let bound_size = size.clone();
        let read_done = done.clone();
        let cycle_done = done.clone();
        panel()
            .column()
            .padding(px(24.0))
            .gap(Size {
                width: 0.0,
                height: 16.0,
            })
            .align_items(Align::Start)
            .background(Color::rgb(24, 26, 32))
            .child(
                button("Press me")
                    .primary()
                    .on_press(move || press_clicks.update(|count| *count += 1)),
            )
            .child(button("Reset").on_press(move || reset_clicks.set(0)))
            .child(label(move || format!("Clicks: {}", read_clicks.get())))
            .child(
                text_input(typed)
                    .width(Length::pct(100.0))
                    .height(px(FIELD_HEIGHT))
                    .placeholder("Type here")
                    .padding(px(8.0))
                    .border(1.0)
                    .border_color(Color::rgb(70, 74, 88)),
            )
            .child(label(move || {
                if read_typed.get().is_empty() {
                    String::from("The field is empty")
                } else {
                    format!("You typed: {}", read_typed.get())
                }
            }))
            .child(label(move || format!("Opacity: {:.2}", read_opacity.get())))
            .child(
                slider(opacity, 0.0..=1.0, 0.05)
                    .accessible_label("Opacity")
                    .width(Length::pct(100.0)),
            )
            .child(label(move || {
                format!("Range: {} to {}", read_low.get(), read_high.get())
            }))
            .child(
                column().width(Length::pct(100.0)).child(
                    range_slider(low, high, 0.0..=100.0)
                        .step(5.0)
                        .accessible_labels("Low end", "High end"),
                ),
            )
            .child(label(move || format!("Size: {}", read_size.get())))
            .child(
                column()
                    .width(Length::pct(100.0))
                    .child(number_field("Size", bound_size, 1.0..=100.0, 1.0).drag_speed(1.0)),
            )
            .child(label({
                let read = read_done.clone();
                move || match read.get() {
                    Some(done) => format!("Loading: {:.0}%", done * 100.0),
                    None => String::from("Loading: unknown length"),
                }
            }))
            .child(
                progress({
                    let read = read_done.clone();
                    move || read.get()
                })
                .width(Length::pct(100.0))
                .accessible_label("Loading"),
            )
            .child(
                row()
                    .gap(Size {
                        width: 8.0,
                        height: 0.0,
                    })
                    .child(button("Unknown length").on_press(move || {
                        cycle_done.update(|done| {
                            *done = match *done {
                                None => Some(0.25),
                                Some(done) if done < 1.0 => Some(done + 0.25),
                                Some(_) => None,
                            }
                        });
                    }))
                    .child(spinner().accessible_label("Working")),
            )
            .child(
                scroll(
                    column()
                        .gap(Size {
                            width: 0.0,
                            height: 4.0,
                        })
                        .children((1..=12).map(|index| {
                            label(move || format!("List item {index} of 12"))
                                .height(px(28.0))
                                .padding(px(4.0))
                        })),
                )
                .height(px(VIEW_HEIGHT))
                .width(Length::pct(100.0)),
            )
    };

    let result = run_with(
        WindowOptions {
            title: String::from("Smudge controls"),
            size: Size {
                width: 420.0,
                height: 480.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        build,
        |app| app.set_theme(Theme::dark()),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("smudge: {error}");
            ExitCode::FAILURE
        }
    }
}
