//! Buttons, labels, a text input and a scroll view, to try accessibility by
//! hand.
//!
//! Run with `cargo run --example controls -p smudge-platform`. A screen
//! reader announces the two buttons, the field and the list this window
//! shows: press the buttons with the keyboard, type into the field, and
//! scroll the list while the reader runs to hear what changes.

use std::process::ExitCode;

use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::style::{Align, Length, px};
use smudge_core::widget::panel;
use smudge_platform::{RendererChoice, WindowOptions, run};
use smudge_text::TextInputExt;
use smudge_text::label;
use smudge_text::text_input;
use smudge_widgets::ButtonExt;
use smudge_widgets::button;
use smudge_widgets::column;
use smudge_widgets::scroll;

/// The height of one input field, in logical pixels.
const FIELD_HEIGHT: f32 = 32.0;

/// The height of the scroll view, in logical pixels.
const VIEW_HEIGHT: f32 = 180.0;

fn main() -> ExitCode {
    let build = || {
        let clicks = RwSignal::new(0u32);
        let typed = RwSignal::new(String::new());
        let press_clicks = clicks.clone();
        let reset_clicks = clicks.clone();
        let read_clicks = clicks.clone();
        let read_typed = typed.clone();
        panel()
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

    let result = run(
        WindowOptions {
            title: String::from("Smudge controls"),
            size: Size {
                width: 420.0,
                height: 480.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            repaint: Default::default(),
            show_damage: false,
            renderer: RendererChoice::default(),
            stats: None,
            on_frame: None,
            on_event: None,
            frame_limit: None,
        },
        build,
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("smudge: {error}");
            ExitCode::FAILURE
        }
    }
}
