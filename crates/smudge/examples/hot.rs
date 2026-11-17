//! A short tour of hot reload: a window started through `run_view_with`,
//! which `cargo smudge dev` patches.
//!
//! Run it with `cargo smudge dev --example hot -p smudge`, which turns the
//! crate's `hot-reload` feature on, then edit while it runs:
//!
//! - change the text `heading` returns, a view helper;
//! - change what the button's handler adds to the count;
//! - change the timer's period or what it counts;
//! - type in the notes, scroll the list, and edit any of the above.
//!
//! Each save shows within about a second. The count lives in the state, so
//! it survives every patch. The notes, the seconds and the list's scroll
//! position live in signals the view creates, which the remount after a
//! patch carries over. The configure step makes the app follow the system's
//! light or dark preference, so the text stays readable either way.

use std::time::Duration;

use smudge::core::timer::every;
use smudge::prelude::*;

struct State {
    count: RwSignal<u32>,
}

/// The title above the counter, a helper an edit can change on its own.
fn heading() -> Element<Label> {
    label("Hot reload").text_size(20.0)
}

fn view(state: &State) -> AnyElement {
    let press = state.count.clone();
    let shown = state.count.clone();
    let notes = RwSignal::new(String::new());
    let seconds = RwSignal::new(0_u32);
    let tick = seconds.clone();
    // The timer belongs to the build, so each remount starts the patched
    // one, and the count it keeps in `seconds` carries over.
    every(Duration::from_secs(1), move || {
        tick.update(|seconds| *seconds += 1)
    })
    .detach();
    let lines = (1..=40).fold(column(), |lines, at| {
        lines.child(label(format!("Line {at} of the scrolled list")))
    });
    column()
        .padding(px(24.0))
        .gap(Size {
            width: 0.0,
            height: 12.0,
        })
        .align_items(Align::Center)
        .child(heading())
        .child(
            button("Count up")
                .primary()
                .on_press(move || press.update(|count| *count += 1)),
        )
        .child(label(move || format!("Clicked {} times", shown.get())))
        .child(label(move || format!("Open for {} seconds", seconds.get())))
        .child(
            text_area(notes)
                .width(px(320.0))
                .height(px(80.0))
                .padding(px(8.0))
                .border(1.0)
                .border_color(ColorToken::BORDER),
        )
        .child(
            scroll(lines.padding(px(8.0)))
                .width(px(320.0))
                .height(px(160.0))
                .border(1.0)
                .border_color(ColorToken::BORDER),
        )
        .into_any()
}

fn main() -> Result<(), PlatformError> {
    run_view_with(
        WindowOptions {
            title: String::from("Hot"),
            size: Size {
                width: 380.0,
                height: 480.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        State {
            count: RwSignal::new(0_u32),
        },
        view,
        |app| {
            app.follow_color_scheme(
                Theme::light(),
                Color::WHITE,
                Theme::dark(),
                Color::rgb(16, 17, 21),
            );
        },
    )
}
