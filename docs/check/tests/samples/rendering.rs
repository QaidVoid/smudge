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
