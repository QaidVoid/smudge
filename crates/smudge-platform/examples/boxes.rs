//! A grid of rounded boxes with one box changing color on every frame.
//!
//! Run with `cargo run --example boxes -p smudge-platform`. Pass `--full` to
//! repaint the whole surface every frame instead of only the damage, `--stats`
//! to print a presentation summary every 120 frames, `--show-damage` to tint
//! the pixels each frame repainted, `--renderer cpu|gpu|auto` to choose the
//! renderer, and `--frames N` to close the window after N presented frames.

// The summaries are the output the example exists to produce.
#![allow(clippy::print_stdout)]

use std::cell::RefCell;
use std::env;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::app::RepaintMode;
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::{Align, Length, px};
use smudge_core::widget::panel;
use smudge_platform::{PresentStatsHandle, RendererChoice, WindowOptions, run_with};

/// How many boxes the grid holds.
const BOXES: usize = 600;

/// The side of one box in logical pixels.
const BOX_SIZE: f32 = 36.0;

/// The space between adjacent boxes in logical pixels.
const GAP: f32 = 4.0;

/// The padding around the grid in logical pixels.
const PADDING: f32 = 8.0;

/// How many frames lie between two presentation summaries.
const STATS_EVERY: u64 = 120;

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
    let mut repaint = RepaintMode::Damage;
    let mut report = false;
    let mut show_damage = false;
    let mut renderer = RendererChoice::default();
    let mut frame_limit = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--full" => repaint = RepaintMode::Full,
            "--stats" => report = true,
            "--show-damage" => show_damage = true,
            "--renderer" => match arguments.next().as_deref() {
                Some("cpu") => renderer = RendererChoice::Cpu,
                Some("gpu") => renderer = RendererChoice::Gpu,
                Some("auto") => renderer = RendererChoice::Auto,
                other => {
                    eprintln!("smudge: --renderer needs cpu, gpu or auto, got {other:?}");
                    return ExitCode::FAILURE;
                }
            },
            "--frames" => match arguments.next().and_then(|value| value.parse::<u64>().ok()) {
                Some(frames) => frame_limit = Some(frames),
                None => {
                    eprintln!("smudge: --frames needs a number of presented frames");
                    return ExitCode::FAILURE;
                }
            },
            other => {
                eprintln!("smudge: unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    let stats = report.then(PresentStatsHandle::new);

    let setters: Rc<RefCell<Vec<SetColor>>> = Rc::new(RefCell::new(Vec::with_capacity(BOXES)));
    let shared = Rc::clone(&setters);

    let build = move || {
        let mut grid = panel()
            .row()
            .wrap(true)
            .gap(Size {
                width: GAP,
                height: GAP,
            })
            .width(Length::pct(100.0));
        let mut setters = shared.borrow_mut();
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
        panel()
            .align_items(Align::Start)
            .padding(px(PADDING))
            .background(Color::rgb(24, 26, 32))
            .child(grid)
    };

    let result = run_with(
        WindowOptions {
            title: String::from("Smudge boxes"),
            size: Size {
                width: 1240.0,
                height: 840.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            repaint,
            show_damage,
            renderer,
            stats: stats.clone(),
            on_frame: None,
            frame_limit,
        },
        build,
        move |app| {
            let mut next = 0usize;
            let mut frames = 0_u64;
            let mut damage_presents = 0_u64;
            let mut copied = 0_u64;
            let mut age_total = 0_u64;
            let mut present_total = Duration::ZERO;
            app.set_tick(move |_| {
                let index = next % BOXES;
                let pass = next / BOXES;
                if let Some(set_color) = setters.borrow().get(index) {
                    set_color(PALETTE[(pass + index) % PALETTE.len()]);
                }
                next = next.wrapping_add(1);

                if let Some(stats) = &stats {
                    let present = stats.get();
                    frames += 1;
                    damage_presents += u64::from(present.with_damage);
                    copied += present.bytes_copied;
                    age_total += u64::from(present.buffer_age);
                    present_total += present.present;
                    if frames.is_multiple_of(STATS_EVERY) {
                        println!(
                            "frame {frames} [{}]: mean buffer age {:.2}, damage presents \
                             {damage_presents}/{frames} ({:.1}%), copied {:.1} MB, \
                             present {:.2} ms mean",
                            present.renderer,
                            age_total as f64 / frames as f64,
                            100.0 * damage_presents as f64 / frames as f64,
                            copied as f64 / 1_048_576.0,
                            present_total.as_secs_f64() * 1000.0 / frames as f64,
                        );
                    }
                }
            });
        },
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("smudge: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Returns the color a box starts with.
fn base_color(index: usize) -> Color {
    let shade = 40 + (index % 8) as u8 * 6;
    Color::rgb(shade, shade, shade + 20)
}
