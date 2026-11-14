//! A window started through `run_view_with`, which `cargo smudge dev`
//! patches.
//!
//! Run with `cargo smudge dev --example hot -p smudge --features hot-reload`
//! and edit `view`: the window shows the change within about a second and
//! keeps the count. The configure step makes the app follow the system's
//! light or dark preference, so the text stays readable either way.

use smudge::core::element::AnyElement;
use smudge::prelude::*;

struct State {
    count: RwSignal<u32>,
}

fn view(state: &State) -> AnyElement {
    let press = state.count.clone();
    let shown = state.count.clone();
    column()
        .padding(px(24.0))
        .gap(Size {
            width: 0.0,
            height: 12.0,
        })
        .align_items(Align::Center)
        .child(
            button("Count up")
                .primary()
                .on_press(move || press.update(|count| *count += 1)),
        )
        .child(label(move || format!("Clicked {} times", shown.get())))
        .into_any()
}

fn main() -> Result<(), PlatformError> {
    run_view_with(
        WindowOptions {
            title: String::from("Hot"),
            size: Size {
                width: 280.0,
                height: 140.0,
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
