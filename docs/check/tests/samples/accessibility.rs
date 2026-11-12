//! Mirrors docs/guide/accessibility.md: its Rust samples, held as functions.
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

use smudge::core::semantics::Role;
use smudge::prelude::*;

#[test]
fn close_button() {
    use smudge::prelude::*;
    use smudge::core::semantics::Role;
    
    let close = panel()
        .size(Size { width: 28.0, height: 28.0 })
        .focusable()
        .activate_on_keys()
        .role(Role::Button)
        .accessible_label("Close")
        .on_press(|| println!("closed"));
}

#[test]
fn named() {
    let muted = RwSignal::new(false);
    let named = muted.clone();
    
    let toggle = panel()
        .role(Role::Button)
        .accessible_label(move || if named.get() { "Unmute" } else { "Mute" }.to_owned());
}

#[test]
fn focus_ring() {
    let item = panel()
        .focusable()
        .activate_on_keys()
        .border(2.0)
        .border_color(Color::TRANSPARENT)
        .focused(|p| p.border_color(ColorToken::FOCUS_RING));
}
