//! Mirrors docs/guide/styling.md: its Rust samples, held as functions.
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

use smudge::prelude::*;


#[test]
fn card() {
    use smudge::prelude::*;
    
    let card = panel()
        .padding(px(16.0))
        .background(Color::rgb(28, 31, 37))
        .border(1.0)
        .border_color(Color::rgb(42, 46, 55))
        .corner_radius(10.0)
        .child(label("Synced 2 minutes ago").text_color(Color::rgb(236, 238, 243)));
}

#[test]
fn tokens() {
    let surface = panel()
        .background(ColorToken::SURFACE)
        .border(1.0)
        .border_color(ColorToken::BORDER)
        .corner_radius(RadiusToken::MEDIUM)
        .child(label("Theme-aware").text_color(ColorToken::TEXT));
}

fn theme_switch() -> Result<(), PlatformError> {
    let options = WindowOptions::default();
    let dark = RwSignal::new(true);
    let toggle = dark.clone();
    
    run_with(
        options,
        move || {
            column()
                .background(ColorToken::SURFACE)
                .child(button("Toggle theme").on_press(move || toggle.update(|d| *d = !*d)))
        },
        move |app| {
            app.bind_theme(move || if dark.get() { Theme::dark() } else { Theme::light() });
        },
    )?;
    Ok(())
}

#[test]
fn states() {
    let row_item = panel()
        .padding(px(8.0))
        .corner_radius(6.0)
        .background(Color::TRANSPARENT)
        .hover(|p| p.background(ColorToken::SURFACE_HOVER))
        .pressed(|p| p.background(ColorToken::SURFACE_PRESSED))
        .on_press(|| println!("opened"));
}

#[test]
fn tags() {
    use smudge::core::style::Edges;
    let tags = row()
        .gap(Size { width: 6.0, height: 0.0 })
        .style_children(|style| {
            style.layout.padding = Edges::all(px(4.0));
            style.paint.background = Some(ColorToken::SURFACE_HOVER.into());
        })
        .children(["rust", "gui", "cpu"].map(label));
}

#[test]
fn default_style() {
    provide_text_style(TextStyle { size: 15.0, ..current_text_style() });
}
