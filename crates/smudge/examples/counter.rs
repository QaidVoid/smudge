//! A label and a button bound to one signal: the smallest Smudge application.
//!
//! Run with `cargo run --example counter -p smudge`. The quick start in the
//! crate docs is this file.

use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();

    run(
        WindowOptions {
            title: String::from("Counter"),
            size: Size {
                width: 280.0,
                height: 140.0,
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
        move || {
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
        },
    )
}
