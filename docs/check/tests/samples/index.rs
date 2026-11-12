//! Mirrors docs/index.md: its Rust samples, held as functions.
//!
//! Every ```rust block of the page appears here exactly as the page
//! shows it, reindented one level inside the wrapper each block needs,
//! and names the page leaves undefined are stubbed next to their
//! block. The page is the source of truth: change it here and there
//! together. The allows hold what page text may carry without a
//! compiler's blessing, so page edits trip this check only for real
//! API drift.
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_imports)]


use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();

    run_with(
        WindowOptions {
            title: String::from("Counter"),
            size: Size { width: 280.0, height: 140.0 },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        move || {
            column()
                .padding(px(24.0))
                .gap(Size { width: 0.0, height: 12.0 })
                .align_items(Align::Center)
                .child(
                    button("Count up")
                        .primary()
                        .on_press(move || press.update(|count| *count += 1)),
                )
                .child(label(move || format!("Clicked {} times", shown.get())))
        },
        |app| app.set_theme(Theme::dark()),
    )
}
