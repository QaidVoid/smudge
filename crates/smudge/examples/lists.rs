//! A 100,000-row virtual list behind a filter input: typing keeps only the
//! matching rows, and the list mounts only the rows near the viewport.
//!
//! Run with `cargo run --example lists -p smudge`.

use smudge::prelude::*;

/// How many rows the list holds before filtering.
const ROW_COUNT: usize = 100_000;

/// The height of one row, in logical pixels.
const ROW_HEIGHT: f32 = 28.0;

/// The height of the list viewport, in logical pixels.
const VIEW_HEIGHT: f32 = 420.0;

/// The height of the filter field, in logical pixels.
const FIELD_HEIGHT: f32 = 32.0;

fn main() -> Result<(), PlatformError> {
    let filter = RwSignal::new(String::new());

    run(
        WindowOptions {
            title: String::from("Smudge lists"),
            size: Size {
                width: 420.0,
                height: 560.0,
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
            let needle = filter.clone();
            let matches = Memo::new(move || {
                let needle = needle.get();
                (0..ROW_COUNT)
                    .filter(|index| item_name(*index).contains(needle.as_str()))
                    .collect::<Vec<usize>>()
            });
            let counted = matches.clone();
            let listed = matches.clone();

            column()
                .padding(px(16.0))
                .gap(Size {
                    width: 0.0,
                    height: 8.0,
                })
                .width(pct(100.0))
                .child(
                    text_input(filter)
                        .width(pct(100.0))
                        .height(px(FIELD_HEIGHT))
                        .placeholder("Filter rows")
                        .border(1.0)
                        .border_color(Color::rgb(70, 74, 88)),
                )
                .child(label(move || {
                    format!("{} of {ROW_COUNT} rows", counted.with(|rows| rows.len()))
                }))
                .child(
                    virtual_list(
                        move || listed.with(|rows| rows.len()),
                        ROW_HEIGHT,
                        move |slot| {
                            let matches = matches.clone();
                            label(move || {
                                matches
                                    .with(|rows| rows.get(slot).copied())
                                    .map(item_name)
                                    .unwrap_or_default()
                            })
                            .height(px(ROW_HEIGHT))
                            .padding(px(4.0))
                        },
                    )
                    .width(pct(100.0))
                    .height(px(VIEW_HEIGHT)),
                )
        },
    )
}

/// Returns the text of the row with this index.
fn item_name(index: usize) -> String {
    format!("Item {index:05}")
}
