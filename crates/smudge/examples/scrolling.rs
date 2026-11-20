//! Scroll views on both axes, nested in a page.
//!
//! Run with `cargo run --example scrolling -p smudge`, then:
//!
//! - Swipe sideways on a touchpad over the strip of cards, or hold Shift
//!   and turn the mouse wheel, and drag its thumb along the bottom.
//! - Turn the wheel over the strip without Shift. The page scrolls on.
//! - Scroll the grid both ways, with the wheel and both thumbs, and Tab
//!   through its cells to see each one brought into view.
//! - Scroll the long line sideways.

use smudge::prelude::*;
use smudge::text::{FontFamily, GenericFamily};
use smudge::widgets::{ScrollAxis, ScrollExt};

/// A card of the strip, `index` deciding its color.
fn card(index: usize) -> Element<Panel> {
    column()
        .width(px(140.0))
        .height(px(90.0))
        .shrink(0.0)
        .padding(px(10.0))
        .corner_radius(RadiusToken::MEDIUM)
        .background(Color::rgb(40 + (index * 23 % 120) as u8, 70, 120))
        .child(label(format!("Card {index}")))
}

/// A grid of focusable cells, 20 by 20.
fn grid() -> Element<Panel> {
    (0..20).fold(column(), |grid, row_index| {
        grid.child((0..20).fold(row(), |cells, column_index| {
            cells.child(
                column()
                    .width(px(70.0))
                    .height(px(40.0))
                    .shrink(0.0)
                    .padding(px(6.0))
                    .background(Color::rgb(
                        (column_index * 9) as u8,
                        (row_index * 9) as u8,
                        110,
                    ))
                    .focused(|p| p.border(2.0).border_color(ColorToken::FOCUS_RING))
                    .focusable()
                    .accessible_label(format!("Cell {row_index}, {column_index}"))
                    .child(label(format!("{row_index}, {column_index}"))),
            )
        }))
    })
}

/// A heading over the section that follows it.
fn heading(text: &str) -> Element<Label> {
    label(text.to_owned()).text_size(18.0)
}

fn main() -> Result<(), PlatformError> {
    run_with(
        WindowOptions {
            title: String::from("Scrolling"),
            size: Size {
                width: 720.0,
                height: 560.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        || {
            let line =
                "let total = values.iter().map(|value| value * factor).sum::<f64>(); ".repeat(8);
            let page = column()
                .padding(px(20.0))
                .gap(Size {
                    width: 0.0,
                    height: 16.0,
                })
                .child(heading("A strip of cards"))
                .child(
                    scroll((0..30).fold(
                        row().gap(Size {
                            width: 10.0,
                            height: 0.0,
                        }),
                        |strip, index| strip.child(card(index)),
                    ))
                    .axis(ScrollAxis::Horizontal)
                    .height(px(110.0)),
                )
                .child(heading("A grid that scrolls both ways"))
                .child(
                    scroll(grid())
                        .axis(ScrollAxis::Both)
                        .height(px(240.0))
                        .border(1.0)
                        .border_color(ColorToken::BORDER),
                )
                .child(heading("A long line"))
                .child(
                    scroll(label(line).family(FontFamily::from(GenericFamily::Monospace)))
                        .axis(ScrollAxis::Horizontal)
                        .padding(px(8.0))
                        .background(ColorToken::SURFACE_HOVER),
                )
                .children((0..12).map(|index| {
                    label(format!(
                        "Paragraph {index}: the page keeps going, so the wheel over the strip \
                         and the grid has somewhere to pass when they cannot use it."
                    ))
                }));
            scroll(page)
        },
        |app| app.set_theme(Theme::dark()),
    )
}
