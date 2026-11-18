use super::*;

/// The text of each block `split` finds in `text`.
fn blocks(text: &str) -> Vec<&str> {
    let starts = split(text);
    starts
        .iter()
        .enumerate()
        .map(|(at, &start)| {
            let end = starts.get(at + 1).copied().unwrap_or(text.len());
            &text[start..end]
        })
        .collect()
}

#[test]
fn a_fence_with_blank_lines_stays_one_block() {
    let text = "```\nfirst\n\nsecond\n```\n\nafter\n";
    assert_eq!(blocks(text), ["```\nfirst\n\nsecond\n```\n", "\nafter\n"]);
}

#[test]
fn a_loose_list_stays_one_block() {
    let text = "- one\n\n- two\n\n  more of two\n\nafter\n";
    assert_eq!(
        blocks(text),
        ["- one\n\n- two\n\n  more of two\n\n", "after\n"]
    );
}

#[test]
fn indented_code_with_blank_lines_stays_one_block() {
    let text = "    let a = 1;\n\n    let b = 2;\n\nafter\n";
    assert_eq!(
        blocks(text),
        ["    let a = 1;\n\n    let b = 2;\n", "\nafter\n"]
    );
}

#[test]
fn a_block_quote_ends_where_its_markers_do() {
    let text = "> quoted\n> more\n\n> another quote\n";
    assert_eq!(blocks(text), ["> quoted\n> more\n", "\n> another quote\n"]);
}

#[test]
fn the_last_block_stays_open_until_a_later_one_starts() {
    let mut splitter = Splitter::default();
    let mut starts = Vec::new();
    splitter.advance("para\n\n", &mut starts);
    assert!(starts.is_empty(), "nothing after the paragraph closes it");
    splitter.advance("para\n\n-", &mut starts);
    assert!(
        starts.is_empty(),
        "a line whose end has not arrived decides nothing"
    );
    splitter.advance("para\n\n- item\n", &mut starts);
    assert_eq!(starts, [5], "the list item's line closed the paragraph");
    assert_eq!(splitter.open(), 5);
}

#[test]
fn a_setext_underline_joins_the_open_paragraph() {
    assert_eq!(blocks("Title\n---\n\nbody\n"), ["Title\n---\n", "\nbody\n"]);
}

#[test]
fn definitions_join_the_block_after_them() {
    let text = "intro\n\n[a]: /x\n\nbody [a]\n";
    assert_eq!(blocks(text), ["intro\n", "\n[a]: /x\n\nbody [a]\n"]);
}

#[test]
fn a_definition_after_a_list_starts_the_block_it_belongs_to() {
    assert_eq!(split("- a\n\n[x]: /u\npara\n"), [0, 5]);
    assert_eq!(split("-\n\n[x]: /u\npara\n"), [0, 3]);
}
