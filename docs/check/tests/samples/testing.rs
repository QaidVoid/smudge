//! Mirrors docs/guide/testing.md: its Rust samples, held as functions.
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


use smudge::core::app::{App, AppConfig, RepaintMode};
use smudge::core::input::{InputEvent, PointerButton};
use smudge::prelude::*;

#[test]
fn a_press_repaints_the_button_and_the_label_only() {
    TextSystem::bundled_only().install();

    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();

    let mut app = App::new(
        AppConfig {
            viewport: Size { width: 280.0, height: 140.0 },
            scale: 1.0,
            clear_color: Color::rgb(16, 17, 21),
            repaint: RepaintMode::Damage,
        },
        move || {
            column()
                .padding(px(24.0))
                .child(button("Count up").on_press(move || press.update(|n| *n += 1)))
                .child(label(move || format!("Clicked {} times", shown.get())))
        },
    );
    app.set_theme(Theme::dark());
    app.frame();

    let at = Point { x: 40.0, y: 36.0 };
    app.handle_event(InputEvent::PointerDown { position: at, button: PointerButton::Primary });
    app.frame();
    // The pressed color repaints the button alone, with no layout.
    assert!(!app.stats().layout_ran);
    assert_eq!(app.stats().nodes_painted, 1);

    app.handle_event(InputEvent::PointerUp { position: at, button: PointerButton::Primary });
    app.frame();
    // The release counts, and the new text lays out and repaints the label.
    assert_eq!(count.get(), 1);
    assert_eq!(app.stats().nodes_painted, 2);
}

fn app_for_pixels() -> App {
    App::new(
        AppConfig {
            viewport: Size { width: 280.0, height: 140.0 },
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || panel(),
    )
}

#[test]
fn pixels() {
    let mut app = app_for_pixels();
    use smudge_cpu::CpuRenderer;
    
    let mut renderer = CpuRenderer::new(280, 140);
    let output = app.frame();
    renderer.render(&output);
    
    let pixels = renderer.pixels();      // premultiplied RGBA8, row-major
    let png = renderer.encode_png();     // for snapshot files
}
