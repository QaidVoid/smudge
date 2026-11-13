//! Mirrors docs/guide/layout.md: its Rust samples, held as functions.
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
fn toolbar() {
    use smudge::prelude::*;
    
    let toolbar = row()
        .padding(px(8.0))
        .gap(Size { width: 8.0, height: 0.0 })
        .align_items(Align::Center)
        .child(button("Open"))
        .child(button("Save").primary())
        .child(spacer())
        .child(label("Ready"));
}

#[test]
fn children() {
    let list = column().children(["Red", "Green", "Blue"].map(label));
}

#[test]
fn sidebar() {
    let sidebar = column()
        .width(px(240.0))
        .height(pct(100.0))
        .min_width(px(180.0))
        .padding(px(12.0));
}

#[test]
fn header() {
    let header = row()
        .justify_content(Justify::SpaceBetween)
        .align_items(Align::Center)
        .padding_xy(Size { width: 16.0, height: 8.0 })
        .child(label("Inbox"))
        .child(button("Compose").primary());
}

#[test]
fn badge() {
    use smudge::core::style::Edges;
    
    let badge = panel()
        .absolute()
        .inset(Edges { top: px(-4.0), right: px(-4.0), bottom: Length::Auto, left: Length::Auto })
        .size(Size { width: 12.0, height: 12.0 })
        .corner_radius(6.0)
        .background(Color::rgb(255, 64, 64));
}

#[test]
fn pinned() {
    let anchor = RwSignal::new(Some((1, 100.0)));
    let pinned = anchor.clone();
    
    let rows = column()
        .pin_child(move || pinned.get())
        .child(label("Grows as it wraps"))
        .child(label("Stays 100 pixels down"));
}
