//! Mirrors docs/guide/devtools.md: its Rust samples, held as functions.
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

mod theme_file {
    use smudge::prelude::*;
    use smudge::core::app::App;

    fn configure(app: &mut App) {
        let theme = RwSignal::new(Theme::dark());
        let shown = theme.clone();
        app.bind_theme(move || shown.get());
        std::mem::forget(smudge::devtools::watch_theme("theme.txt", theme));
    }
}
