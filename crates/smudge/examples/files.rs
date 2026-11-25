//! A file picker parented to the window, a drop zone for files from the
//! file manager, and the list of the chosen and dropped paths: the pieces
//! an app combines to open a file.
//!
//! Run with `cargo run --example files -p smudge`. The picker is rfd's
//! synchronous dialog over the XDG desktop portal, so it runs on the
//! window's thread and the window pauses until the dialog closes. At run
//! time it needs a portal backend (GTK or KDE, as desktops ship one), and
//! falls back to zenity when the portal is unreachable, for example where
//! libdbus is missing.

use std::path::PathBuf;

use rfd::FileDialog;
use smudge::core::semantics::Role;
use smudge::core::theme::ColorValue;
use smudge::prelude::*;
use smudge::window::handle::with_window_handles;

/// The height of the drop zone, in logical pixels.
const ZONE_HEIGHT: f32 = 120.0;

/// The height of the list viewport, in logical pixels.
const LIST_HEIGHT: f32 = 260.0;

fn main() -> Result<(), PlatformError> {
    let paths = RwSignal::new(Vec::<PathBuf>::new());
    let (picked, clear, zone) = (paths.clone(), paths.clone(), paths.clone());

    run_with(
        WindowOptions {
            title: String::from("Smudge files"),
            size: Size {
                width: 460.0,
                height: 560.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        move || {
            column()
                .padding(px(16.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
                .width(pct(100.0))
                .child(
                    row()
                        .gap(Size {
                            width: 8.0,
                            height: 0.0,
                        })
                        .child(button("Open a file").on_press(move || {
                            if let Some(path) = pick_file() {
                                add_paths(&picked, std::slice::from_ref(&path));
                            }
                        }))
                        .child(button("Clear").on_press(move || clear.set(Vec::new()))),
                )
                .child(drop_zone(zone))
                .child(chosen(paths))
        },
        |app| app.set_theme(Theme::dark()),
    )
}

/// Opens the file picker parented to the window, and returns the path the
/// user chose.
///
/// The dialog blocks until it closes, so the press handler that calls it
/// holds the window's thread for the dialog's lifetime. An error from
/// `with_window_handles`, such as a run before the window opened, answers
/// `None` like a cancelled dialog.
fn pick_file() -> Option<PathBuf> {
    let dialog = FileDialog::new().set_title("Choose a file");
    with_window_handles(|handles| dialog.set_parent(handles).pick_file())
        .ok()
        .flatten()
}

/// Adds the paths to the list, skipping one the list already holds, so a
/// file that arrives twice keeps one row: `for_each` keys the row on the
/// path, and its keys must be unique.
fn add_paths(paths: &RwSignal<Vec<PathBuf>>, fresh: &[PathBuf]) {
    paths.update(|paths| {
        for path in fresh {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    });
}

/// The zone that highlights while files from the file manager hover over
/// it, and adds their paths to the list when they land.
fn drop_zone(paths: RwSignal<Vec<PathBuf>>) -> impl IntoElement {
    let hovering = RwSignal::new(false);
    let (accent, enter) = (hovering.clone(), hovering.clone());
    let (shown, land) = (hovering.clone(), paths.clone());
    panel()
        .width(pct(100.0))
        .height(px(ZONE_HEIGHT))
        .padding(px(16.0))
        .corner_radius(8.0)
        .background(ColorToken::SURFACE_HOVER)
        .border(2.0)
        .border_color(move || {
            if accent.get() {
                ColorValue::from(ColorToken::ACCENT)
            } else {
                ColorValue::from(ColorToken::BORDER)
            }
        })
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .role(Role::Group)
        .accessible_label("Drop files here")
        .on_file_hover(move |files| enter.set(files.is_some()))
        .on_file_drop(move |files, _at| add_paths(&land, files))
        .child(label(move || {
            if shown.get() {
                String::from("Release to add the files")
            } else {
                String::from("Drop files from the file manager here")
            }
        }))
}

/// The list of the paths the picker chose and the drop zone received.
fn chosen(paths: RwSignal<Vec<PathBuf>>) -> impl IntoElement {
    let (counted, empty, rows) = (paths.clone(), paths.clone(), paths.clone());
    column()
        .gap(Size {
            width: 0.0,
            height: 8.0,
        })
        .child(label(move || {
            counted.with(|paths| format!("{} file(s) in the list", paths.len()))
        }))
        .child(show(
            move || empty.get().is_empty(),
            || {
                label("Nothing chosen yet. Open a file or drop one here.")
                    .text_color(ColorToken::TEXT_MUTED)
            },
        ))
        .child(
            scroll(
                for_each(
                    move || rows.get(),
                    |path| path.clone(),
                    |path| {
                        panel()
                            .width(pct(100.0))
                            .padding(px(4.0))
                            .child(label(path.display().to_string()))
                    },
                )
                .column()
                .gap(Size {
                    width: 0.0,
                    height: 2.0,
                }),
            )
            .height(px(LIST_HEIGHT))
            .border(1.0)
            .border_color(ColorToken::BORDER),
        )
}
