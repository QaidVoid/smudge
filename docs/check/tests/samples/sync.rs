//! Checks that every Rust sample on a page appears in the module that
//! mirrors it, so a page edit that is not copied here fails the gates, and
//! that every page under `docs/` with a Rust sample, and the README, is
//! listed here.

/// Each page with Rust samples, by its path from the repository root, and
/// the module that holds them.
const PAGES: &[(&str, &str, &str)] = &[
    (
        "README.md",
        include_str!("../../../../README.md"),
        include_str!("index.rs"),
    ),
    (
        "docs/index.md",
        include_str!("../../../index.md"),
        include_str!("index.rs"),
    ),
    (
        "docs/guide/accessibility.md",
        include_str!("../../../guide/accessibility.md"),
        include_str!("accessibility.rs"),
    ),
    (
        "docs/guide/dynamic-content.md",
        include_str!("../../../guide/dynamic-content.md"),
        include_str!("dynamic_content.rs"),
    ),
    (
        "docs/guide/layout.md",
        include_str!("../../../guide/layout.md"),
        include_str!("layout.rs"),
    ),
    (
        "docs/guide/quick-start.md",
        include_str!("../../../guide/quick-start.md"),
        include_str!("quick_start.rs"),
    ),
    (
        "docs/guide/rendering.md",
        include_str!("../../../guide/rendering.md"),
        include_str!("rendering.rs"),
    ),
    (
        "docs/guide/signals.md",
        include_str!("../../../guide/signals.md"),
        include_str!("signals.rs"),
    ),
    (
        "docs/guide/styling.md",
        include_str!("../../../guide/styling.md"),
        include_str!("styling.rs"),
    ),
    (
        "docs/guide/testing.md",
        include_str!("../../../guide/testing.md"),
        include_str!("testing.rs"),
    ),
    (
        "docs/guide/text-and-input.md",
        include_str!("../../../guide/text-and-input.md"),
        include_str!("text_and_input.rs"),
    ),
    (
        "docs/guide/widgets.md",
        include_str!("../../../guide/widgets.md"),
        include_str!("widgets.rs"),
    ),
];

/// The non-blank lines of `text`, without their indentation.
fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// The bodies of the page's ```rust blocks.
fn rust_blocks(page: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in page.lines() {
        match current.as_mut() {
            None if line.trim_start().starts_with("```rust") => current = Some(String::new()),
            None => {}
            Some(_) if line.trim_start().starts_with("```") => {
                blocks.extend(current.take());
            }
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    blocks
}

#[test]
fn every_page_sample_is_compiled() {
    let mut missing = Vec::new();
    for (page, text, module) in PAGES {
        let module = lines(module);
        for (index, block) in rust_blocks(text).iter().enumerate() {
            let block = lines(block);
            if block.is_empty() {
                continue;
            }
            let found = module
                .windows(block.len())
                .any(|window| window == block.as_slice());
            if !found {
                missing.push(format!("{page}, sample {}", index + 1));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "samples not mirrored in docs/check: {missing:?}"
    );
}

/// The repository root, two levels above this crate.
fn repository_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Collects the Markdown files under `dir`, by their path from the
/// repository root, skipping installed packages and generated output.
fn markdown_pages(root: &std::path::Path, dir: &str, pages: &mut Vec<String>) {
    let entries = std::fs::read_dir(root.join(dir)).expect("the docs directory is readable");
    for entry in entries {
        let entry = entry.expect("the docs directory lists its entries");
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{dir}/{name}");
        if entry.path().is_dir() {
            if name != "node_modules" && !name.starts_with('.') {
                markdown_pages(root, &path, pages);
            }
        } else if name.ends_with(".md") {
            pages.push(path);
        }
    }
}

#[test]
fn every_page_with_rust_samples_is_listed() {
    let root = repository_root();
    let mut pages = vec![String::from("README.md")];
    markdown_pages(&root, "docs", &mut pages);
    let unlisted: Vec<String> = pages
        .into_iter()
        .filter(|page| {
            let text = std::fs::read_to_string(root.join(page)).expect("the page is readable");
            !rust_blocks(&text).is_empty() && !PAGES.iter().any(|(listed, ..)| listed == page)
        })
        .collect();
    assert!(
        unlisted.is_empty(),
        "pages with Rust samples missing from PAGES: {unlisted:?}"
    );
}
