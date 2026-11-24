//! A file browser to try the data views by hand: a tree of folders that
//! fills a table of files, a collapsible section of details, and a grid
//! that aligns the details' labels.
//!
//! Run with `cargo run --example data_views -p smudge`, then:
//!
//! - unfold folders in the tree and pick one to fill the table;
//! - press the Name, Size or Modified header to sort by it, and press
//!   again to reverse or clear the sort;
//! - drag a header cell's right edge to resize its column;
//! - select files with a press, Ctrl or Cmd, and Shift;
//! - move the focus with the arrow keys, Page Up, Page Down, Home and End,
//!   and toggle a row with Space;
//! - fold the Details section away, and watch it keep the picked folder's
//!   counts.
//!
//! The guide's data views page walks through the same widgets in shorter
//! steps.

use std::rc::Rc;

use smudge::core::semantics::SortDirection;
use smudge::core::style::Track;
use smudge::prelude::*;

/// The key of the folder the table shows before the user picks one.
const ROOT: u64 = 1;

/// One file of a folder, which the table shows as a row.
#[derive(Clone)]
struct File {
    /// The file's name, unique within its folder.
    name: String,
    /// The file's size in bytes.
    size: u64,
    /// The file's last change, kept as text for the example.
    modified: &'static str,
}

/// One folder of the tree, which may hold folders and files of its own.
#[derive(Clone)]
struct Node {
    /// The folder's key, unique across the tree.
    id: u64,
    /// The folder's name.
    name: String,
    /// The folders this one holds. The tree asks for them only while this
    /// folder is open, so a model that loads from disk would fetch here.
    folders: Vec<Node>,
    /// The files this folder holds directly.
    files: Rc<[File]>,
}

/// Builds the browser's model: a small project tree.
fn model() -> Rc<[Node]> {
    let file = |name: &str, size: u64, modified: &'static str| File {
        name: String::from(name),
        size,
        modified,
    };
    Rc::from(vec![Node {
        id: ROOT,
        name: String::from("project"),
        folders: vec![
            Node {
                id: 2,
                name: String::from("src"),
                folders: vec![Node {
                    id: 4,
                    name: String::from("views"),
                    folders: Vec::new(),
                    files: Rc::from(vec![
                        file("table.rs", 18_432, "2026-11-24"),
                        file("tree_view.rs", 12_288, "2026-11-23"),
                        file("collapsible.rs", 5_632, "2026-11-21"),
                    ]),
                }],
                files: Rc::from(vec![
                    file("main.rs", 2_048, "2026-11-24"),
                    file("lib.rs", 512, "2026-11-20"),
                ]),
            },
            Node {
                id: 3,
                name: String::from("docs"),
                folders: Vec::new(),
                files: Rc::from(vec![
                    file("guide.md", 24_576, "2026-11-24"),
                    file("readme.md", 3_072, "2026-11-18"),
                ]),
            },
        ],
        files: Rc::from(vec![
            file("Cargo.toml", 896, "2026-11-19"),
            file("license", 1_024, "2026-11-01"),
        ]),
    }])
}

/// Returns the folder whose key is `id`, walking `nodes` depth first.
fn node_of(nodes: &[Node], id: u64) -> Option<&Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = node_of(&node.folders, id) {
            return Some(found);
        }
    }
    None
}

/// Returns the files the folder `id` holds directly, or an empty list when
/// no folder of the tree carries that key.
fn files_of(model: &[Node], id: u64) -> Rc<[File]> {
    node_of(model, id)
        .map(|node| Rc::clone(&node.files))
        .unwrap_or_default()
}

/// Returns the folder's name, or the root's when `id` names nothing.
fn name_of(model: &[Node], id: u64) -> String {
    node_of(model, id)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| String::from("project"))
}

/// Builds the table over the picked folder's files, sorted the way its
/// header asks. The table never reorders the rows: its header writes the
/// sort to its own signal, an effect forwards it here, and the rows
/// closure sorts before it hands the rows over.
fn file_table(model: Rc<[Node]>, picked: RwSignal<Option<u64>>) -> TableView<String> {
    let sort_by: RwSignal<Option<(usize, SortDirection)>> = RwSignal::new(None);
    let rows_picked = picked.clone();
    let rows_sort = sort_by.clone();
    let view = table(
        [
            Column::new("Name", 220.0, 90.0, Align::Start, true),
            Column::new("Size", 90.0, 60.0, Align::End, true),
            Column::new("Modified", 110.0, 80.0, Align::Start, true),
        ],
        move || {
            let folder = rows_picked.get().unwrap_or(ROOT);
            let (column, descending) = rows_sort
                .get()
                .map(|(column, way)| (column, matches!(way, SortDirection::Descending)))
                .unwrap_or((0, false));
            let mut files = files_of(&model, folder).as_ref().to_vec();
            match column {
                1 => files.sort_by_key(|file| file.size),
                2 => files.sort_by_key(|file| file.modified),
                _ => files.sort_by(|a, b| a.name.cmp(&b.name)),
            }
            if descending {
                files.reverse();
            }
            Rc::from(files)
        },
        |file| file.name.clone(),
        |file, column_index| match column_index {
            1 => label(format!("{} kB", file.size / 1024)),
            2 => label(file.modified),
            _ => label(file.name.clone()).max_lines(Some(1)),
        },
    )
    .multiple(true);

    let from_view = view.sort().clone();
    RenderEffect::new(move || sort_by.set(from_view.get()));
    view
}

fn main() -> Result<(), PlatformError> {
    let model = model();
    let details_open = RwSignal::new(true);

    let tree_model = Rc::clone(&model);
    let table_model = Rc::clone(&model);
    let sidebar_model = Rc::clone(&model);
    let caption_model = Rc::clone(&model);

    run_with(
        WindowOptions {
            title: String::from("Smudge data views"),
            size: Size {
                width: 760.0,
                height: 480.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        move || {
            let tree = tree_view(
                move || Rc::clone(&tree_model),
                |node| Rc::from(node.folders.as_slice()),
                |node| node.id,
                24.0,
                |node, depth| label(format!("{}{}", "  ".repeat(depth), node.name)),
            )
            // The panes around it do not stretch their children, so the
            // tree needs the pane's width itself.
            .width(pct(100.0))
            .height(px(240.0));
            // The root starts open, so the tree shows its folders at once.
            tree.expanded().update(|open| {
                open.insert(ROOT);
            });

            let picked = tree.selected().clone();
            let view = file_table(table_model, picked.clone());
            let selection = view.selection().clone();

            let header_picked = picked.clone();
            let caption_selection = selection.clone();

            row()
                .padding(px(16.0))
                .gap(Size {
                    width: 16.0,
                    height: 0.0,
                })
                .child(
                    column()
                        .width(px(220.0))
                        .gap(Size {
                            width: 0.0,
                            height: 12.0,
                        })
                        .align_items(Align::Start)
                        .child(
                            panel()
                                .width(pct(100.0))
                                .border(1.0)
                                .border_color(ColorToken::BORDER)
                                .child(tree),
                        )
                        .child(collapsible(details_open, label("Details"), move || {
                            let name_model = Rc::clone(&sidebar_model);
                            let name_picked = picked.clone();
                            let folders_model = Rc::clone(&sidebar_model);
                            let folders_picked = picked.clone();
                            let files_model = Rc::clone(&sidebar_model);
                            let files_picked = picked.clone();
                            grid([Track::Px(72.0), Track::Fr(1.0)])
                                .width(pct(100.0))
                                .padding(px(8.0))
                                .gap(Size {
                                    width: 8.0,
                                    height: 6.0,
                                })
                                .align_items(Align::Start)
                                .border(1.0)
                                .border_color(ColorToken::BORDER)
                                .child(label("Folder"))
                                .child(label(move || {
                                    name_of(&name_model, name_picked.get().unwrap_or(ROOT))
                                }))
                                .child(label("Folders"))
                                .child(label(move || {
                                    let found = node_of(
                                        &folders_model,
                                        folders_picked.get().unwrap_or(ROOT),
                                    );
                                    found.map_or(0, |node| node.folders.len()).to_string()
                                }))
                                .child(label("Files"))
                                .child(label(move || {
                                    let found =
                                        node_of(&files_model, files_picked.get().unwrap_or(ROOT));
                                    found.map_or(0, |node| node.files.len()).to_string()
                                }))
                        })),
                )
                .child(
                    column()
                        .grow(1.0)
                        .gap(Size {
                            width: 0.0,
                            height: 8.0,
                        })
                        .align_items(Align::Start)
                        .child(label(move || {
                            format!(
                                "Files of {}",
                                name_of(&caption_model, header_picked.get().unwrap_or(ROOT))
                            )
                        }))
                        .child(
                            panel()
                                .grow(1.0)
                                .width(pct(100.0))
                                .border(1.0)
                                .border_color(ColorToken::BORDER)
                                .child(view.width(pct(100.0)).height(pct(100.0))),
                        )
                        .child(label(move || {
                            match caption_selection.with(|set| set.len()) {
                                0 => String::from("No files chosen"),
                                1 => String::from("1 file chosen"),
                                count => format!("{count} files chosen"),
                            }
                        })),
                )
        },
        |app| app.set_theme(Theme::dark()),
    )
}
