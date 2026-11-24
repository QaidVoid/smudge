//! Mirrors docs/guide/data-views.md: its Rust samples, held as functions.
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
fn form() {
    use smudge::core::style::Track;
    use smudge::prelude::*;
    
    let name = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let name_field = name.clone();
    let email_field = email.clone();
    
    let form = grid([Track::Px(110.0), Track::Fr(1.0)])
        .gap(Size { width: 12.0, height: 8.0 })
        .child(label("Name"))
        .child(text_input(name_field))
        .child(label("Email"))
        .child(text_input(email_field));
}

#[test]
fn content() {
    use smudge::core::style::{Track, TrackSize};
    
    let content = grid([
        Track::MinMax(TrackSize::Px(160.0), TrackSize::Fr(1.0)),
        Track::Fr(3.0),
    ]);
}

#[test]
fn spanned() {
    use smudge::core::style::Track;
    use smudge::prelude::*;
    
    let name = RwSignal::new(String::new());
    
    let form = grid([Track::Px(110.0), Track::Fr(1.0)])
        .child(label("Account").column_span(2))
        .child(label("Name"))
        .child(text_input(name));
}

#[test]
fn section() {
    use smudge::prelude::*;
    
    let advanced = RwSignal::new(false);
    let section = collapsible(
        advanced,
        label("Advanced"),
        || column()
            .child(label("Expert settings live here"))
            .child(label("Each section folds on its own")),
    );
}

/// The tree of `tree()`, which `tree_state()` stubs its view with.
fn file_tree() -> smudge::widgets::TreeView<u64> {
    use std::rc::Rc;
    use smudge::prelude::*;

    #[derive(Clone)]
    struct Node {
        id: u64,
        name: String,
        children: Vec<Node>,
    }

    let tree: RwSignal<Rc<[Node]>> = RwSignal::new(Rc::from(vec![Node {
        id: 1,
        name: String::from("src"),
        children: vec![Node {
            id: 2,
            name: String::from("main.rs"),
            children: Vec::new(),
        }],
    }]));

    let roots = tree.clone();
    tree_view(
        move || roots.get().clone(),
        |node| Rc::from(node.children.as_slice()),
        |node| node.id,
        24.0,
        |node, depth| label(format!("{}{}", "  ".repeat(depth), node.name)),
    )
    .height(px(320.0))
}

#[test]
fn tree() {
    use std::rc::Rc;
    use smudge::prelude::*;

    #[derive(Clone)]
    struct Node {
        id: u64,
        name: String,
        children: Vec<Node>,
    }

    let tree: RwSignal<Rc<[Node]>> = RwSignal::new(Rc::from(vec![Node {
        id: 1,
        name: String::from("src"),
        children: vec![Node {
            id: 2,
            name: String::from("main.rs"),
            children: Vec::new(),
        }],
    }]));

    let roots = tree.clone();
    let view = tree_view(
        move || roots.get().clone(),
        |node| Rc::from(node.children.as_slice()),
        |node| node.id,
        24.0,
        |node, depth| label(format!("{}{}", "  ".repeat(depth), node.name)),
    )
    .height(px(320.0));
}

#[test]
fn tree_state() {
    let view = file_tree();
    
    let opened = view.expanded();
    let picked = view.selected();
}

/// The table of `files_table()`, which `table_state()` stubs its view with.
fn file_table() -> smudge::widgets::TableView<String> {
    use std::rc::Rc;
    use smudge::prelude::*;

    #[derive(Clone)]
    struct File {
        name: String,
        size: u64,
    }

    let files: RwSignal<Rc<[File]>> = RwSignal::new(Rc::from(vec![
        File { name: String::from("main.rs"), size: 4096 },
        File { name: String::from("guide.md"), size: 512 },
    ]));

    let listed = files.clone();
    table(
        [
            Column::new("Name", 220.0, 80.0, Align::Start, true),
            Column::new("Size", 90.0, 60.0, Align::End, false),
        ],
        move || listed.get().clone(),
        |file| file.name.clone(),
        |file, column_index| {
            if column_index == 0 {
                label(file.name.clone()).max_lines(Some(1))
            } else {
                label(format!("{} kB", file.size / 1024))
            }
        },
    )
    .height(px(400.0))
}

#[test]
fn files_table() {
    use std::rc::Rc;
    use smudge::prelude::*;

    #[derive(Clone)]
    struct File {
        name: String,
        size: u64,
    }

    let files: RwSignal<Rc<[File]>> = RwSignal::new(Rc::from(vec![
        File { name: String::from("main.rs"), size: 4096 },
        File { name: String::from("guide.md"), size: 512 },
    ]));

    let listed = files.clone();
    let view = table(
        [
            Column::new("Name", 220.0, 80.0, Align::Start, true),
            Column::new("Size", 90.0, 60.0, Align::End, false),
        ],
        move || listed.get().clone(),
        |file| file.name.clone(),
        |file, column_index| {
            if column_index == 0 {
                label(file.name.clone()).max_lines(Some(1))
            } else {
                label(format!("{} kB", file.size / 1024))
            }
        },
    )
    .height(px(400.0));
}

#[test]
fn table_state() {
    let view = file_table();
    
    let by = view.sort();
    let picked = view.selection();
    let widths = view.widths();
}
