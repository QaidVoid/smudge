//! Mirrors docs/guide/dynamic-content.md: its Rust samples, held as functions.
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
fn sign_in() {
    use smudge::prelude::*;
    
    let signed_in = RwSignal::new(false);
    let when = signed_in.clone();
    let toggle = signed_in.clone();
    
    let page = column()
        .child(button("Sign in or out").on_press(move || toggle.update(|on| *on = !*on)))
        .child(show(move || when.get(), || label("Welcome back")));
}

#[test]
fn hidden() {
    let expanded = RwSignal::new(true);
    let details = column().hidden(move || !expanded.get());
}

#[test]
fn tasks() {
    #[derive(Clone)]
    struct Task { id: u32, title: String }

    let tasks = RwSignal::new(vec![
        Task { id: 1, title: String::from("Write the docs") },
        Task { id: 2, title: String::from("Ship v0.1") },
    ]);
    let listed = tasks.clone();

    let list = column().child(for_each(
        move || listed.get(),
        |task| task.id,
        |task| label(task.title).padding(px(6.0)),
    ));
}

#[test]
fn row_builder() {
    #[derive(Clone)]
    struct Row { id: u32, done: RwSignal<bool> }

    let row = |item: Row| {
        let done = item.done.clone();
        label(move || if done.get() { "done" } else { "open" }.to_owned())
    };
}
