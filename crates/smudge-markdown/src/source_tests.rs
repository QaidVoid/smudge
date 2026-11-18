use super::*;

#[test]
fn every_heading_paragraph_cell_and_item_is_a_leaf_in_reading_order() {
    let source = MarkdownSource::new();
    source.push("# Title\n\n- one\n- two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\nend\n");
    let texts: Vec<String> = source
        .leaves()
        .iter()
        .map(|leaf| source.leaf_text(leaf))
        .collect();
    assert_eq!(texts, ["Title", "one", "two", "a", "b", "1", "2", "end"]);
}

#[test]
fn an_html_block_shows_no_empty_line_after_its_last() {
    let source = MarkdownSource::new();
    source.push("<div>\nhi\n</div>\n");
    let texts: Vec<String> = source
        .leaves()
        .iter()
        .map(|leaf| source.leaf_text(leaf))
        .collect();
    assert_eq!(texts, ["<div>\nhi\n</div>"]);
}
