//! Two single-line text inputs to try an input method and the clipboard by
//! hand.
//!
//! Run with `cargo run --example text_inputs -p smudge-platform`. Click a
//! field to focus it and Tab to move to the other one. Compose text with the
//! input method of the operating system to see the underlined preedit and
//! the candidate window follow the caret, and copy, cut and paste with the
//! platform's command key. Each field keeps its text in a signal of its own,
//! so both remember what was typed while the other one was focused.

use std::process::ExitCode;

use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::RwSignal;
use smudge_core::style::{Align, Length, px};
use smudge_core::widget::panel;
use smudge_platform::{WindowOptions, run};
use smudge_text::TextInputExt;
use smudge_text::text_input;

/// The height of one input field, in logical pixels.
const FIELD_HEIGHT: f32 = 32.0;

fn main() -> ExitCode {
    let build = || {
        let first = RwSignal::new(String::new());
        let second = RwSignal::new(String::new());
        panel()
            .padding(px(24.0))
            .gap(Size {
                width: 0.0,
                height: 16.0,
            })
            .align_items(Align::Start)
            .background(Color::rgb(24, 26, 32))
            .child(
                text_input(first)
                    .width(Length::pct(100.0))
                    .height(px(FIELD_HEIGHT))
                    .placeholder("Focus me and compose with an input method")
                    .padding(px(8.0))
                    .border(1.0)
                    .border_color(Color::rgb(70, 74, 88)),
            )
            .child(
                text_input(second)
                    .width(Length::pct(100.0))
                    .height(px(FIELD_HEIGHT))
                    .placeholder("Then focus me and paste")
                    .padding(px(8.0))
                    .border(1.0)
                    .border_color(Color::rgb(70, 74, 88)),
            )
    };

    let result = run(
        WindowOptions {
            title: String::from("Smudge text inputs"),
            size: Size {
                width: 420.0,
                height: 160.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            repaint: Default::default(),
            show_damage: false,
            stats: None,
            on_frame: None,
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
