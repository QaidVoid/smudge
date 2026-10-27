//! A grid of rounded boxes and a caption that animate in a browser, to show
//! the CPU renderer and damage tracking on the web target.
//!
//! The build and serve commands live in `examples/web/README.md`. The same
//! example also opens a desktop window with
//! `cargo run --example web_boxes -p smudge-platform`.
//!
//! One box changes color per frame while the repaint mode stays on damage,
//! so the caption shows what a frame actually had to copy.

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use smudge_core::app::RepaintMode;
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{self, RwSignal, prelude::*};
use smudge_core::style::{Align, Length, px};
use smudge_core::widget::panel;
use smudge_platform::{PlatformError, PresentStatsHandle, RendererChoice, WindowOptions, run_with};
use smudge_text::{LabelExt, label};

/// How many boxes the grid holds.
const BOXES: usize = 120;

/// The side of one box in logical pixels.
const BOX_SIZE: f32 = 36.0;

/// The space between adjacent boxes in logical pixels.
const GAP: f32 = 4.0;

/// The padding around the content in logical pixels.
const PADDING: f32 = 16.0;

/// How many presented frames lie between two caption updates.
const CAPTION_EVERY: u64 = 120;

/// Sets the color of one box.
type SetColor = Box<dyn Fn(Color)>;

/// The colors a box takes when the tick reaches it.
const PALETTE: [Color; 6] = [
    Color::rgb(232, 93, 76),
    Color::rgb(238, 160, 62),
    Color::rgb(236, 214, 96),
    Color::rgb(94, 191, 122),
    Color::rgb(72, 155, 226),
    Color::rgb(158, 112, 226),
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("smudge: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The web entry point the generated bindings call when the module loads.
///
/// Panics write to the browser console, which the default stderr sink never
/// reaches, so a hook routes them there first.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start_web() -> Result<(), wasm_bindgen::JsValue> {
    std::panic::set_hook(Box::new(|info| console_error(&info.to_string())));
    match run() {
        Ok(()) => Ok(()),
        Err(error) => {
            console_error(&error.to_string());
            Err(error.to_string().into())
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

// SAFETY: the block declares the browser's `console.error`, which takes a
// string and returns nothing, so every call with a valid string is sound.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn console_error(message: &str);
}

/// Opens the window, or on the web starts the event loop, and runs the app.
fn run() -> Result<(), PlatformError> {
    let setters: Rc<RefCell<Vec<SetColor>>> = Rc::new(RefCell::new(Vec::with_capacity(BOXES)));
    let grid_setters = Rc::clone(&setters);
    let stats = PresentStatsHandle::new();
    let caption = RwSignal::new(String::from("damage: waiting for the first present"));
    let caption_read = caption.clone();

    let build = move || {
        let mut grid = panel()
            .row()
            .wrap(true)
            .gap(Size {
                width: GAP,
                height: GAP,
            })
            .width(Length::pct(100.0));
        let mut setters = grid_setters.borrow_mut();
        for index in 0..BOXES {
            let (color, set_color) = reactive::signal_local(base_color(index));
            setters.push(Box::new(move |color| set_color.set(color)));
            grid = grid.child(
                panel()
                    .width(px(BOX_SIZE))
                    .height(px(BOX_SIZE))
                    .corner_radius(8.0)
                    .border(2.0)
                    .border_color(Color::rgb(70, 74, 88))
                    .background(move || color.get().into()),
            );
        }
        drop(setters);
        panel()
            .column()
            .align_items(Align::Start)
            .padding(px(PADDING))
            .gap(Size {
                width: 0.0,
                height: 12.0,
            })
            .background(Color::rgb(24, 26, 32))
            .child(
                label("Smudge boxes")
                    .text_size(20.0)
                    .text_color(Color::WHITE),
            )
            .child(
                label(move || caption_read.get())
                    .text_size(13.0)
                    .text_color(Color::rgb(160, 164, 178)),
            )
            .child(grid)
    };

    run_with(
        WindowOptions {
            title: String::from("Smudge boxes"),
            size: Size {
                width: 960.0,
                height: 640.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            repaint: RepaintMode::Damage,
            show_damage: false,
            renderer: RendererChoice::default(),
            stats: Some(stats.clone()),
            on_frame: None,
            on_event: None,
            frame_limit: None,
        },
        build,
        move |app| {
            let mut next = 0usize;
            let mut frames = 0_u64;
            app.set_tick(move |_| {
                let index = next % BOXES;
                let pass = next / BOXES;
                if let Some(set_color) = setters.borrow().get(index) {
                    set_color(PALETTE[(pass + index) % PALETTE.len()]);
                }
                next = next.wrapping_add(1);
                frames += 1;
                if frames.is_multiple_of(CAPTION_EVERY) {
                    let present = stats.get();
                    caption.set(format!(
                        "frame {frames}: {} present copied {} of {} kilobytes",
                        if present.with_damage {
                            "damage"
                        } else {
                            "full"
                        },
                        present.bytes_copied / 1024,
                        u64::from(present.surface_width) * u64::from(present.surface_height) * 4
                            / 1024,
                    ));
                }
            });
        },
    )
}

/// Returns the color a box starts with.
fn base_color(index: usize) -> Color {
    let shade = 40 + (index % 8) as u8 * 6;
    Color::rgb(shade, shade, shade + 20)
}
