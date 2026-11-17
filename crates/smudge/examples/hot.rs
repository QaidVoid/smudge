//! A window started through `run_view_with`, which `cargo smudge dev`
//! patches.
//!
//! Run with `cargo smudge dev --example hot -p smudge --features hot-reload`
//! and edit `view` or `heading`: the window shows the change within about a
//! second. The count lives in the state, so it survives every patch. The
//! notes and the list's scroll position live in signals the view creates,
//! which the remount after a patch carries over. The configure step makes
//! the app follow the system's light or dark preference, so the text stays
//! readable either way.

use smudge::core::element::AnyElement;
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
        .child(text_area(notes).width(px(320.0)).height(px(80.0)))
        .child(scroll(lines).width(px(320.0)).height(px(160.0)))
        .into_any()
}

fn main() -> Result<(), PlatformError> {
    run_view_with(
        WindowOptions {
            title: String::from("Hot"),
            size: Size {
                width: 380.0,
                height: 460.0,
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
