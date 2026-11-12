//! Mirrors docs/guide/signals.md: its Rust samples, held as functions.
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
#![allow(clippy::unnecessary_map_or)] // samples stay as the page shows them

use smudge::prelude::*;


#[test]
fn basics() {
    use smudge::prelude::*;
    
    let count = RwSignal::new(0_u32);
    
    count.set(5);                       // replace the value
    count.update(|n| *n += 1);          // change it in place
    let now = count.get();              // read a copy (needs Clone)
    let len = count.with(|n| n.to_string().len()); // read by reference
}

#[test]
fn binds() {
    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();
    
    let ui = column()
        .child(button("Add").on_press(move || press.update(|n| *n += 1)))
        .child(label(move || format!("{}", shown.get())));
}

#[test]
fn chip() {
    let selected = RwSignal::new(false);
    let shown = selected.clone();
    
    let chip = panel()
        .size(Size { width: 80.0, height: 28.0 })
        .corner_radius(8.0)
        .background(move || {
            if shown.get() { Color::rgb(212, 255, 63) } else { Color::rgb(42, 46, 55) }.into()
        })
        .on_press(move || selected.update(|on| *on = !*on));
}

#[test]
fn set_both() {
    let first = RwSignal::new(String::new());
    let last = RwSignal::new(String::new());
    first.set(String::from("Ada"));
    last.set(String::from("Lovelace"));
    // One frame later, a label that reads both shows "Ada Lovelace" once.
}

#[test]
fn memo() {
    let filter = RwSignal::new(String::new());
    let needle = filter.clone();
    let matches = Memo::new(move || {
        let needle = needle.get();
        (0..100_000)
            .filter(|index| format!("Item {index:05}").contains(needle.as_str()))
            .count()
    });
    let counted = matches.clone();
    let summary = label(move || format!("{} matches", counted.get()));
}

#[test]
fn effect() {
    let count = RwSignal::new(0_u32);
    let saved = count.clone();
    let _effect = RenderEffect::new(move || {
        println!("count is now {}", saved.get());
    });
}

#[test]
fn context() {
    use smudge::core::reactive::{provide_context, use_context};

    #[derive(Clone)]
    struct Settings { compact: bool }

    provide_context(Settings { compact: true });
    // Further down the tree:
    let compact = use_context::<Settings>().map_or(false, |s| s.compact);
}

fn load_lines() -> Vec<String> { Vec::new() }
fn log_view() -> Element<Panel> { panel() }
#[test]
fn messages() {
    use smudge::core::channel::channel;
    
    let lines = RwSignal::new(Vec::<String>::new());
    
    let build = move || {
        let apply = channel(move |incoming: Vec<String>| {
            lines.update(|all| all.extend(incoming));
        });
        std::thread::spawn(move || {
            let _ = apply.send(load_lines());
        });
        log_view()
    };
}
