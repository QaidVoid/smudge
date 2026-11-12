//! Checks that every Rust sample on a page appears in the module that
//! mirrors it, so a page edit that is not copied here fails the gates.

/// Each page with Rust samples, and the module that holds them.
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
        "accessibility.md",
        include_str!("../../../guide/accessibility.md"),
        include_str!("accessibility.rs"),
    ),
    (
        "dynamic-content.md",
        include_str!("../../../guide/dynamic-content.md"),
        include_str!("dynamic_content.rs"),
    ),
    (
        "layout.md",
        include_str!("../../../guide/layout.md"),
        include_str!("layout.rs"),
    ),
    (
        "quick-start.md",
        include_str!("../../../guide/quick-start.md"),
        include_str!("quick_start.rs"),
    ),
    (
        "rendering.md",
        include_str!("../../../guide/rendering.md"),
        include_str!("rendering.rs"),
    ),
    (
        "signals.md",
        include_str!("../../../guide/signals.md"),
        include_str!("signals.rs"),
    ),
    (
        "styling.md",
        include_str!("../../../guide/styling.md"),
        include_str!("styling.rs"),
    ),
    (
        "testing.md",
        include_str!("../../../guide/testing.md"),
        include_str!("testing.rs"),
    ),
    (
        "text-and-input.md",
        include_str!("../../../guide/text-and-input.md"),
        include_str!("text_and_input.rs"),
    ),
    (
        "widgets.md",
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
