//! Every pointer button, the context menu and the pointer lock.
//!
//! Run with `cargo run --example pointer -p smudge`, then:
//!
//! - press any button over the pad: the log names the button and the
//!   modifiers held;
//! - drag with the middle button: the pad's offset follows, even past the
//!   window's edge;
//! - hold the right button to fly: the pointer hides and stays put, and the
//!   heading and pitch follow the raw motion until the release, Escape or
//!   a switch to another window frees it;
//! - watch the title bar, which says whether the pad is flying, and the
//!   window's icon, a small gradient the example draws;
//! - Shift+right-click, which the pad leaves to the context menu, or press
//!   the menu key or Shift+F10 after a click on the pad: the log shows where
//!   a context menu would open.

use smudge::commands;
use smudge::core::image::Image;
use smudge::core::input::{ButtonEvent, Handled, PointerButton};
use smudge::core::reactive::RenderEffect;
use smudge::prelude::*;

/// Draws the window's icon: a square that fades from blue to orange.
fn icon() -> Option<Image> {
    let size = 32_u32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let t = (x + y) * 255 / (2 * (size - 1));
            let t = u8::try_from(t).unwrap_or(u8::MAX);
            pixels.extend_from_slice(&[t, 120, 255 - t, 255]);
        }
    }
    Image::from_straight_rgba(size, size, pixels).ok()
}

/// Describes `event` in a line of the log.
fn describe(event: &ButtonEvent) -> String {
    let held: Vec<&str> = [
        (event.modifiers.shift, "Shift"),
        (event.modifiers.control, "Ctrl"),
        (event.modifiers.alt, "Alt"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    format!(
        "{:?} at {:.0}, {:.0}{}",
        event.button,
        event.position.x,
        event.position.y,
        if held.is_empty() {
            String::new()
        } else {
            format!(" with {}", held.join("+"))
        }
    )
}

fn main() -> Result<(), PlatformError> {
    let log = RwSignal::new(String::from("Press any button over the pad."));
    let offset = RwSignal::new(Point::default());
    let grab = RwSignal::new(Point::default());
    let flying = RwSignal::new(false);
    let angles = RwSignal::new((0.0_f32, 0.0_f32));

    run_with(
        WindowOptions {
            title: String::from("Pointer"),
            size: Size {
                width: 520.0,
                height: 360.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            icon: icon(),
            ..WindowOptions::default()
        },
        move || {
            let (written, menu, shown) = (log.clone(), log.clone(), log.clone());
            let (start, pan, held) = (grab.clone(), offset.clone(), grab.clone());
            let (fly, land, look) = (flying.clone(), flying.clone(), angles.clone());
            let (where_, how) = (offset.clone(), angles.clone());
            let lock = flying.clone();
            let state = flying.clone();
            let titled = flying.clone();
            let _title = RenderEffect::new(move || {
                commands::set_title(if titled.get() {
                    "Pointer, flying"
                } else {
                    "Pointer"
                });
            });
            column()
                .padding(px(24.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
                .child(
                    panel()
                        .height(px(180.0))
                        .background(Color::rgb(36, 38, 46))
                        .corner_radius(8.0)
                        .focusable()
                        .lock_pointer(lock)
                        .on_button_down(move |event| {
                            written.set(describe(&event));
                            match event.button {
                                PointerButton::Middle => {
                                    start.set(event.position);
                                    Handled::Yes
                                }
                                PointerButton::Secondary if event.modifiers.shift => Handled::No,
                                PointerButton::Secondary => {
                                    fly.set(true);
                                    Handled::Yes
                                }
                                _ => Handled::No,
                            }
                        })
                        .on_button_move(move |event| {
                            if event.button == PointerButton::Middle {
                                let from = held.get();
                                pan.update(|offset| {
                                    offset.x += event.position.x - from.x;
                                    offset.y += event.position.y - from.y;
                                });
                                held.set(event.position);
                            }
                        })
                        .on_button_up(move |event| {
                            if event.button == PointerButton::Secondary {
                                land.set(false);
                            }
                        })
                        .on_pointer_motion(move |delta| {
                            look.update(|(heading, pitch)| {
                                *heading += delta.x * 0.2;
                                *pitch = (*pitch - delta.y * 0.2).clamp(-89.0, 89.0);
                            });
                        })
                        .on_context_menu(move |at| {
                            menu.set(format!("Context menu at {:.0}, {:.0}", at.x, at.y));
                        }),
                )
                .child(label(move || shown.get()))
                .child(label(move || {
                    let offset = where_.get();
                    format!("Middle drag offset: {:.0}, {:.0}", offset.x, offset.y)
                }))
                .child(label(move || {
                    let (heading, pitch) = how.get();
                    format!(
                        "Heading {heading:.1}, pitch {pitch:.1}{}",
                        if state.get() { ", flying" } else { "" }
                    )
                }))
                .child(label(
                    "Shift+right-click skips flying, so the context menu opens.",
                ))
        },
        |app| app.set_theme(Theme::dark()),
    )
}
