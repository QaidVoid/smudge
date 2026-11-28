//! Mirrors docs/guide/rendering.md: its Rust samples, held as functions.
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
#![allow(clippy::redundant_closure)] // samples stay as the page shows them


use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    run_with(
        WindowOptions {
            title: String::from("Notes"),
            size: Size { width: 640.0, height: 480.0 },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        || label("Hello"),
        |app| app.set_theme(Theme::dark()),
    )
}

fn bind_theme() -> Result<(), PlatformError> {
    let options = WindowOptions::default();
    let build = || label("Hello");
    run_with(options, build, |app| {
        app.bind_theme(|| Theme::dark());
    })?;
    Ok(())
}

fn start_move() {
    use smudge::commands::start_move;
    
    let header = panel()
        .height(px(40.0))
        .on_pointer_down(|_| start_move());
}

fn ask_for_the_gpu() {
    let options = WindowOptions {
        renderer: RendererChoice::Auto,
        ..WindowOptions::default()
    };
}

fn gpu_view_sample() {
    use smudge::gpu::{GpuViewExt, gpu_view};

    let camera = RwSignal::new(0.0_f32);
    let shown = camera.clone();
    let view = gpu_view(move |frame| {
        let mut encoder = frame.device.create_command_encoder(&Default::default());
        // Record the scene's passes into `frame.target` here.
        frame.queue.submit([encoder.finish()]);
    })
    .redraw_when(move || shown.get())
    .width(px(640.0))
    .height(px(480.0));
}

fn open_docs() {}
#[test]
fn pointer_cursor() {
    let link = label("Open the docs")
        .cursor(Cursor::Pointer)
        .on_press(open_docs);
}

#[test]
fn present_stats() {
    let stats = PresentStatsHandle::new();
    let reader = stats.clone();
    // Pass `stats: Some(stats)` in WindowOptions, then from anywhere on the main thread:
    let latest = reader.get();
    println!("copied {} bytes, damage only: {}", latest.bytes_copied, latest.with_damage);
}

mod dock_layout_state {
    use serde::{Deserialize, Serialize};
    use smudge::widgets::dock::{Branch, DockLayout};

    const STATE_VERSION: u32 = 1;

    #[derive(Serialize, Deserialize)]
    struct AppState {
        version: u32,
        dock: DockLayout,
    }

    fn save_layout(dock: DockLayout) -> serde_json::Result<String> {
        serde_json::to_string(&AppState { version: STATE_VERSION, dock })
    }

    fn restore_layout(
        saved: Option<&str>,
        default: DockLayout,
        open_keys: &[&str],
        fallback: &[Branch],
    ) -> DockLayout {
        let mut dock = saved
            .and_then(|text| serde_json::from_str::<AppState>(text).ok())
            .filter(|state| state.version == STATE_VERSION)
            .map_or(default, |state| state.dock);
        dock.reconcile(open_keys, fallback);
        dock
    }

    #[test]
    fn saved_dock_survives_changes_to_the_panel_catalog() -> Result<(), Box<dyn std::error::Error>> {
        use smudge::widgets::dock::DockNode;

        let mut saved = DockLayout::default();
        saved.add("editor", &[])?;
        saved.add("retired", &[])?;
        saved.activate("editor")?;
        let json = save_layout(saved)?;

        let mut default = DockLayout::default();
        default.add("outline", &[])?;
        default.add("editor", &[])?;
        let restored = restore_layout(Some(&json), default, &["editor", "outline"], &[]);
        let expected = DockLayout::new(DockNode::Group {
            panels: vec!["editor".into(), "outline".into()],
            active: "editor".into(),
        })?;
        assert_eq!(restored, expected);
        Ok(())
    }

    #[test]
    fn unsupported_or_unreadable_state_restores_the_default() -> Result<(), Box<dyn std::error::Error>> {
        let mut default = DockLayout::default();
        default.add("outline", &[])?;
        default.add("editor", &[])?;
        let incompatible = serde_json::to_string(&AppState {
            version: STATE_VERSION + 1,
            dock: DockLayout::default(),
        })?;
        for saved in [None, Some("not JSON"), Some(incompatible.as_str())] {
            assert_eq!(restore_layout(saved, default.clone(), &["editor", "outline"], &[]), default);
        }
        Ok(())
    }
}
