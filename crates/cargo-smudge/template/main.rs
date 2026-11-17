//! A counter: a button and a label over a count that lives in the window's
//! state, so `cargo smudge dev` keeps it across every hot patch.

use smudge::prelude::*;

/// What the window keeps while the app runs, across every hot patch.
struct State {
    /// How many times the button was pressed.
    count: RwSignal<u32>,
}

/// Builds the window's contents from the state. Edit it while
/// `cargo smudge dev` runs, and the window follows each save.
fn view(state: &State) -> AnyElement {
    let count = state.count.clone();
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
                .on_press(move || count.update(|count| *count += 1)),
        )
        .child(label(move || format!("Clicked {} times", shown.get())))
        .into_any()
}

fn main() -> Result<(), PlatformError> {
    run_view(
        WindowOptions {
            title: String::from("Counter"),
            size: Size {
                width: 320.0,
                height: 160.0,
            },
            ..WindowOptions::default()
        },
        State {
            count: RwSignal::new(0),
        },
        view,
    )
}
