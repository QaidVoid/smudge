//! Mirrors docs/guide/hot-reload.md: its Rust samples, held as functions.
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


use smudge::core::element::AnyElement;
use smudge::prelude::*;

struct State {
    count: RwSignal<u32>,
}

fn view(state: &State) -> AnyElement {
    let count = state.count.clone();
    let shown = state.count.clone();
    column()
        .child(button("Count up").on_press(move || count.update(|n| *n += 1)))
        .child(label(move || format!("Clicked {} times", shown.get())))
        .into_any()
}

fn main() -> Result<(), PlatformError> {
    run_view(
        WindowOptions::default(),
        State { count: RwSignal::new(0) },
        view,
    )
}

mod worker {
    use smudge::hot;

    fn work_once() {}

    fn spawn_worker() {
        std::thread::spawn(|| loop {
            hot::call(work_once);
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
    }
}

mod hook {
    use smudge::hot;

    fn restart_workers() {}

    fn watch_patches() {
        hot::on_hot_patch(restart_workers);
    }
}
