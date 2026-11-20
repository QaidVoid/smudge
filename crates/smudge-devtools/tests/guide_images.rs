//! Renders the images of the devtools guide, `docs/guide/devtools.md`, with
//! the CPU renderer and the bundled fonts.
//!
//! The test is ignored, so the gates never rewrite the images. Run it on
//! purpose after a change to the panel's look, and review the new images:
//! `cargo test -p smudge-devtools --test guide_images -- --ignored`.

use std::time::Duration;

use smudge_core::app::{App, AppConfig, Remounted, RepaintMode};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::semantics::Role;
use smudge_core::style::px;
use smudge_core::widget::panel;
use smudge_devtools::{FrameRecord, HotEvent, Tools};
use smudge_test::Harness;
use smudge_text::TextSystem;

/// The size of each image, in logical pixels.
const SIZE: Size = Size {
    width: 432.0,
    height: 630.0,
};

/// Builds a harness that shows the panel of `tools`.
fn panel_of(tools: &Tools) -> Harness {
    let shown = tools.clone();
    let mut harness = Harness::new(SIZE.width, SIZE.height, 1.0, move || {
        smudge_devtools::overlay(&shown, || SIZE)
    });
    harness.app().set_theme(smudge_core::theme::Theme::dark());
    harness.frame();
    harness
}

/// Presses the overlay's text that reads `text`.
fn press(harness: &mut Harness, text: &str) {
    let bounds = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some(text))
        .unwrap_or_else(|| panic!("the panel shows {text:?}"))
        .bounds;
    harness.click(Point {
        x: (bounds.x0 + bounds.x1) / 2.0,
        y: (bounds.y0 + bounds.y1) / 2.0,
    });
    harness.frame();
    harness.frame();
}

/// Writes `harness`'s surface to `docs/public/<name>`.
fn save(harness: &Harness, name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/public")
        .join(name);
    std::fs::write(path, harness.png()).expect("the image is written");
}

#[test]
#[ignore = "rewrites the guide's images"]
fn render_the_guide_images() {
    TextSystem::bundled_only().install();
    let tools = Tools::new();
    for index in 0..240_u64 {
        tools.record(FrameRecord {
            layout: Duration::from_micros(400 + (index * 37) % 900),
            paint: Duration::from_micros(300 + (index * 53) % 700),
            raster: Duration::from_micros(1_200 + (index * 71) % 2_400),
            present: Duration::from_micros(500),
            presented: index % 17 != 0,
            nodes_painted: 3,
            ..FrameRecord::default()
        });
    }
    let mut harness = panel_of(&tools);
    let timeline = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some("Timeline"))
        .expect("the frames tab shows the timeline")
        .bounds;
    harness.move_to(Point {
        x: SIZE.width - 60.0,
        y: timeline.y1 + 60.0,
    });
    harness.frame();
    save(&harness, "devtools-frames.png");
    press(&mut harness, "Overlays");
    press(&mut harness, "Layout bounds");
    press(&mut harness, "Focus");
    save(&harness, "devtools-overlays.png");

    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 400.0,
                height: 300.0,
            },
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || {
            panel()
                .padding(px(12.0))
                .child(
                    panel()
                        .width(px(120.0))
                        .height(px(40.0))
                        .border(2.0)
                        .padding(px(6.0))
                        .role(Role::Button)
                        .accessible_label("Save")
                        .focusable()
                        .on_press(|| {}),
                )
                .child(
                    panel()
                        .width(px(40.0))
                        .height(px(40.0))
                        .role(Role::Button)
                        .focusable()
                        .on_press(|| {}),
                )
        },
    );
    app.frame();
    let mut entries = Vec::new();
    app.visit_tree(|entry| entries.push(entry));
    tools.show_tree(entries.iter().copied());
    tools.select(entries.get(1).map(|entry| entry.id));
    tools.show_details(entries.get(1).and_then(|entry| app.node_details(entry.id)));
    press(&mut harness, "Inspector");
    harness.frame();
    save(&harness, "devtools-inspector.png");

    tools.show_access(&app.semantics_nodes(), &app.accessibility_warnings());
    press(&mut harness, "Access");
    save(&harness, "devtools-access.png");

    tools.show_reactive(
        &[(Some(std::panic::Location::caller()), 12.0), (None, 4.0)],
        &[],
        &[],
    );
    press(&mut harness, "Reactive");
    save(&harness, "devtools-reactive.png");

    tools.record_hot(HotEvent::Building);
    tools.record_hot(HotEvent::BuildFailed(String::from(
        "error[E0308]: mismatched types\n  --> src/main.rs:12:9",
    )));
    tools.record_hot(HotEvent::Building);
    tools.record_hot(HotEvent::Applied {
        took: Duration::from_micros(8_400),
        remounted: Some(Remounted::default()),
    });
    press(&mut harness, "Live");
    save(&harness, "devtools-live.png");
}
