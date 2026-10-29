//! A window with no decorations that draws its own chrome, to try window
//! commands and cursors by hand.
//!
//! Run with `cargo run --example chrome -p smudge-platform`. Drag the header
//! to move the window, drag any edge or corner to resize it, and use the
//! header buttons to maximize, restore and close. A label in the header
//! shows the time a background thread sent a second ago, so a window that
//! keeps up proves a message from another thread reaches the next frame.

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_core::window::{self, Cursor, ResizeEdge};
use smudge_platform::{WindowOptions, run};
use smudge_text::label;
use smudge_widgets::{button, column, row, spacer};

/// The height of the header bar, in logical pixels.
const HEADER_HEIGHT: f32 = 36.0;

/// The thickness of one resize strip, in logical pixels.
const EDGE: f32 = 6.0;

/// The color the header and the resize strips paint.
const CHROME_COLOR: Color = Color::rgb(24, 26, 32);

fn main() -> ExitCode {
    let build = || {
        let status = RwSignal::new(String::from("synced -"));
        let handler_status = status.clone();
        let sender = smudge_core::channel::channel(move |time: String| handler_status.set(time));
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(1));
                if sender.send(clock()).is_err() {
                    break;
                }
            }
        });

        let status_text = status.clone();
        column()
            .background(Color::rgb(16, 17, 21))
            .child(top_strip())
            .child(middle(status_text))
            .child(bottom_strip())
    };

    let result = run(
        WindowOptions {
            title: String::from("Smudge chrome"),
            size: Size {
                width: 480.0,
                height: 320.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            decorations: false,
            app_id: Some(String::from("smudge-chrome")),
            ..WindowOptions::default()
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

/// The UTC wall clock the status thread sends each second.
fn clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    format!(
        "synced {:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

/// One resize handle: a strip that starts a resize from `edge` and shows the
/// cursor for it. It hides while the window is maximized, because a
/// maximized window does not resize.
fn handle(edge: ResizeEdge) -> smudge_core::element::Element<smudge_core::widget::Panel> {
    panel()
        .background(CHROME_COLOR)
        .cursor(Cursor::Resize(edge))
        .hidden(window::maximized)
        .on_pointer_down(move |_| window::start_resize(edge))
}

/// The strip along the top edge with its two corners.
fn top_strip() -> smudge_core::element::Element<smudge_core::widget::Panel> {
    row()
        .height(px(EDGE))
        .background(CHROME_COLOR)
        .child(handle(ResizeEdge::NorthWest).width(px(EDGE)))
        .child(handle(ResizeEdge::North).grow(1.0))
        .child(handle(ResizeEdge::NorthEast).width(px(EDGE)))
}

/// The strip along the bottom edge with its two corners.
fn bottom_strip() -> smudge_core::element::Element<smudge_core::widget::Panel> {
    row()
        .height(px(EDGE))
        .background(CHROME_COLOR)
        .child(handle(ResizeEdge::SouthWest).width(px(EDGE)))
        .child(handle(ResizeEdge::South).grow(1.0))
        .child(handle(ResizeEdge::SouthEast).width(px(EDGE)))
}

/// The strips along the side edges, beside both the header and the content
/// area, so the sides resize along their whole height.
fn middle(status: RwSignal<String>) -> smudge_core::element::Element<smudge_core::widget::Panel> {
    row()
        .grow(1.0)
        .child(handle(ResizeEdge::West).width(px(EDGE)))
        .child(column().grow(1.0).child(header(status)).child(spacer()))
        .child(handle(ResizeEdge::East).width(px(EDGE)))
}

/// The header bar: a drag area with the title and the status, and the
/// maximize and close buttons.
fn header(status: RwSignal<String>) -> smudge_core::element::Element<smudge_core::widget::Panel> {
    row()
        .height(px(HEADER_HEIGHT))
        .padding(px(8.0))
        .gap(Size {
            width: 8.0,
            height: 0.0,
        })
        .align_items(smudge_core::style::Align::Center)
        .background(CHROME_COLOR)
        .child(
            column()
                .grow(1.0)
                .on_pointer_down(|_| window::start_move())
                .child(label(move || format!("Smudge chrome, {}", status.get()))),
        )
        .child(
            button(move || {
                if window::maximized() {
                    String::from("Restore")
                } else {
                    String::from("Maximize")
                }
            })
            .cursor(Cursor::Pointer)
            .on_press(window::toggle_maximize),
        )
        .child(
            button(String::from("Close"))
                .cursor(Cursor::Pointer)
                .on_press(window::close),
        )
}
