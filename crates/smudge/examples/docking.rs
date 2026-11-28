//! An editor workspace with movable tabs and a saved layout.
//!
//! Run on desktop with `cargo run -p smudge --example docking --features serde`.
//! The layout is stored in `smudge-docking-layout.json` in the current
//! directory, or in the path you pass after `--`. Close the window normally
//! and run it again to restore the layout. Document text is not saved.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use smudge::prelude::*;
use smudge::widgets::dock::{Branch, DockError, DockExt, DockLayout, DockPanel, Zone, dock};

const LAYOUT_VERSION: u32 = 1;
const PANELS: &[&str] = &["files", "notes", "readme", "terminal", "outline"];

#[derive(Deserialize, Serialize)]
struct SavedWorkspace {
    version: u32,
    layout: DockLayout,
}

fn default_layout() -> Result<DockLayout, DockError> {
    let mut layout = DockLayout::default();
    for key in PANELS {
        layout.add(key, &[])?;
    }
    layout.move_panel("files", &[], Zone::Left)?;
    layout.set_ratio(&[], 0.20)?;
    layout.move_panel("outline", &[Branch::Second], Zone::Right)?;
    layout.set_ratio(&[Branch::Second], 0.75)?;
    let center = &[Branch::Second, Branch::First];
    layout.move_panel("terminal", center, Zone::Bottom)?;
    layout.set_ratio(center, 0.72)?;
    layout.activate("notes")?;
    Ok(layout)
}

fn restore_layout(json: &[u8], default: DockLayout) -> DockLayout {
    let Ok(saved) = serde_json::from_slice::<SavedWorkspace>(json) else {
        return default;
    };
    if saved.version != LAYOUT_VERSION {
        return default;
    }
    let mut layout = saved.layout;
    let open: Vec<_> = PANELS
        .iter()
        .copied()
        .filter(|key| layout.contains(key))
        .collect();
    layout.reconcile(&open, &[]);
    layout
}

fn load_layout(path: &Path, default: DockLayout) -> DockLayout {
    match fs::read(path) {
        Ok(json) => restore_layout(&json, default),
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!("Could not read {}: {error}", path.display());
            }
            default
        }
    }
}

fn layout_json(layout: DockLayout) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec_pretty(&SavedWorkspace {
        version: LAYOUT_VERSION,
        layout,
    })
}

fn text_list(lines: &[&'static str]) -> Element<ScrollView> {
    let list = lines.iter().fold(
        column().padding(px(10.0)).gap(Size {
            width: 0.0,
            height: 6.0,
        }),
        |list, line| list.child(label(*line)),
    );
    scroll(list).grow(1.0).min_height(px(0.0))
}

fn editor(title: &'static str, contents: &'static str) -> DockPanel {
    DockPanel::new(title, move || {
        text_area(RwSignal::new(contents.to_owned()))
            .accessible_label(title)
            .padding(px(10.0))
            .grow(1.0)
            .min_height(px(0.0))
    })
}

fn build_panel(key: &str) -> DockPanel {
    match key {
        "files" => DockPanel::new("Files", || text_list(&["notes.txt", "README.txt"])),
        "notes" => editor(
            "notes.txt",
            "Draft notes\n\nEdit this text, then move its tab.\n",
        ),
        "readme" => editor(
            "README.txt",
            "Workspace\n\nGetting started\nMove tabs to arrange the workspace.\n",
        ),
        "terminal" => DockPanel::new("Terminal", || {
            column()
                .padding(px(10.0))
                .child(label("Demo session. No commands are run."))
                .child(
                    text_input(RwSignal::new(String::new()))
                        .accessible_label("Terminal scratch input")
                        .placeholder("Scratch input"),
                )
        }),
        "outline" => DockPanel::new("Outline", || {
            text_list(&["Workspace", "Getting started", "Draft notes"])
        }),
        _ => DockPanel::new("Unknown panel", || label("No content available")),
    }
}

fn workspace(layout: RwSignal<DockLayout>, default: DockLayout) -> Element<Panel> {
    let reset = layout.clone();
    column()
        .child(
            row()
                .padding(px(8.0))
                .gap(Size {
                    width: 12.0,
                    height: 0.0,
                })
                .align_items(Align::Center)
                .background(ColorToken::SURFACE_HOVER)
                .child(button("Reset layout").on_press(move || reset.set(default.clone())))
                .child(label("Drop tabs here to float them")),
        )
        .child(
            dock(layout, build_panel)
                .grow(1.0)
                .empty(|| label("Drop a floating tab here, or reset the layout").padding(px(12.0))),
        )
}

fn main() -> Result<(), Box<dyn Error>> {
    if cfg!(target_arch = "wasm32") {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "desktop example only").into());
    }
    let path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("smudge-docking-layout.json"),
        PathBuf::from,
    );
    let default = default_layout()?;
    let layout = RwSignal::new(load_layout(&path, default.clone()));
    let mounted = layout.clone();
    run_with(
        WindowOptions {
            title: String::from("Docking workspace"),
            size: Size {
                width: 1000.0,
                height: 680.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        move || workspace(mounted, default),
        |app| app.set_theme(Theme::dark()),
    )?;
    fs::write(&path, layout_json(layout.get())?).map_err(|error| {
        io::Error::other(format!("Could not save {}: {error}", path.display())).into()
    })
}

#[cfg(test)]
#[path = "docking/docking_tests.rs"]
mod tests;
