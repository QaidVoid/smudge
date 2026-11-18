use proptest::prelude::*;

use super::*;
use crate::model::Inline;
use crate::split::split;

/// Pushes `text` into a new document in pieces that end at each of `cuts`.
fn streamed(text: &str, cuts: &[usize]) -> Document {
    let mut document = Document::default();
    let mut from = 0;
    for &cut in cuts {
        let cut = cut.min(text.len());
        if cut <= from || !text.is_char_boundary(cut) {
            continue;
        }
        document.push(&text[from..cut]);
        from = cut;
    }
    document.push(&text[from..]);
    document
}

/// Every node of every block of `document`, in order.
fn all_nodes(document: &Document) -> Vec<Node> {
    document
        .blocks()
        .iter()
        .flat_map(|block| block.nodes.iter().cloned())
        .collect()
}

/// Asserts that streaming `text` in the pieces `cuts` gives the blocks of
/// splitting it at once, and the structure of parsing it whole.
fn assert_streams_like_the_whole(text: &str, cuts: &[usize]) {
    let document = streamed(text, cuts);
    let whole = normalize_line_endings(text);
    assert_eq!(document.text(), whole, "the text of {text:?}");
    let starts: Vec<usize> = document.blocks().iter().map(|block| block.start).collect();
    assert_eq!(starts, split(&whole), "the blocks of {text:?}");
    assert_eq!(
        all_nodes(&document),
        model::document(&whole),
        "the structure of {text:?}"
    );
}

#[test]
fn a_streamed_transcript_renders_as_its_whole_parse() {
    let text = "# Title\n\nSome *text* with `code` and a [link][ref].\n\n\
                - one\n- two\n\n  continued\n\n```rust\nfn main() {}\n\n```\n\n\
                > quoted\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[ref]: /there\n";
    let cuts: Vec<usize> = (1..text.len()).step_by(3).collect();
    assert_streams_like_the_whole(text, &cuts);
}

#[test]
fn a_reference_updates_when_its_definition_arrives_three_blocks_later() {
    let mut document = Document::default();
    document.push("see [x]\n\nb\n\nc\n\nd\n\n");
    assert_eq!(
        document.blocks()[0].nodes[0],
        Node::Paragraph {
            at: 0,
            text: vec![Inline {
                text: String::from("see [x]"),
                ..Inline::default()
            }],
        },
        "no definition yet, so the source text shows"
    );
    let changes = document.push("[x]: /url\n\ne\n");
    assert!(
        changes.changed.contains(&0),
        "the waiting block parsed again: {changes:?}"
    );
    assert!(
        !changes.changed.contains(&1) && !changes.changed.contains(&2),
        "blocks that cite nothing kept their nodes: {changes:?}"
    );
    let Node::Paragraph { text: runs, .. } = &document.blocks()[0].nodes[0] else {
        panic!("the block is a paragraph");
    };
    assert_eq!(runs[1].link.as_deref(), Some("/url"));
}

#[test]
fn an_append_inside_the_open_block_changes_only_that_block() {
    let mut document = Document::default();
    document.push("first\n\nsecond\n\nthird\n");
    let changes = document.push("grows");
    assert_eq!(
        changes,
        Changes {
            opened: 0,
            changed: vec![2]
        }
    );
}

/// One line a generated document can hold.
fn line() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "",
        "",
        "plain words here",
        "more *emphasis* and **strong**",
        "a `code span` and [a link](/x)",
        "see [ref] and [other][ref2]",
        "[ref]: /target",
        "[ref2]: /two \"title\"",
        "[REF]: /shadowed",
        "- item",
        "* star item",
        "1. first",
        "2) second",
        "  indented two",
        "    indented four",
        "\tindented tab",
        "```",
        "```rust",
        "~~~",
        "> quoted",
        ">",
        "# heading",
        "## heading two",
        "===",
        "---",
        "***",
        "| a | b |",
        "|---|---|",
        "<div>",
        "</div>",
        "<!-- comment",
        "-->",
        "<pre>",
        "</pre>",
        "line with trailing spaces  ",
        "back\\slash",
        "carriage\rreturn",
        "crlf\r\nline",
    ])
}

/// A generated document: lines joined by newlines, with or without a
/// last one.
fn markdown() -> impl Strategy<Value = String> {
    (prop::collection::vec(line(), 0..24), any::<bool>()).prop_map(|(lines, end)| {
        let mut text = lines.join("\n");
        if end {
            text.push('\n');
        }
        text
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn streaming_in_any_pieces_renders_as_the_whole_parse(
        text in markdown(),
        cuts in prop::collection::vec(0usize..400, 0..40),
    ) {
        let mut cuts = cuts;
        cuts.sort_unstable();
        assert_streams_like_the_whole(&text, &cuts);
    }
}

#[test]
fn an_empty_list_item_does_not_take_the_paragraph_after_it() {
    let text = "-\n \u{fffd}-\r88\n `>.--";
    for cut in 1..text.len() {
        if text.is_char_boundary(cut) {
            assert_streams_like_the_whole(text, &[cut]);
        }
    }
}

#[test]
fn a_line_a_carriage_return_left_open_closes_with_the_next_text() {
    let text = "first\r\r=\r\r=!";
    for cut in 1..text.len() {
        assert_streams_like_the_whole(text, &[cut]);
    }
}

#[test]
fn a_fence_after_carriage_returns_parses_as_its_lines_do() {
    let text = "```````x\r/\r```\n code";
    for cut in 1..text.len() {
        assert_streams_like_the_whole(text, &[cut]);
    }
}

#[test]
fn a_line_feed_after_a_carriage_return_pairs_with_it_once() {
    let mut document = Document::default();
    for piece in ["a\r", "\n", "\n", "b\r", "\r\n", "c"] {
        document.push(piece);
    }
    assert_eq!(document.text(), "a\n\nb\n\nc");
}

#[test]
fn a_paragraph_that_becomes_a_definition_leaves_the_blocks_of_the_whole() {
    let text = "first\n\n[a\nlabel]: /x\n\nafter\n";
    for cut in 1..text.len() {
        assert_streams_like_the_whole(text, &[cut]);
    }
}

#[test]
fn a_bracketed_block_waits_while_every_block_after_it_could_vanish() {
    let text = "first\n\n[b\n\n[a\nlabel]: /x\n";
    for cut in 1..text.len() {
        assert_streams_like_the_whole(text, &[cut]);
    }
}

#[test]
fn a_label_closed_lines_later_turns_the_blocks_it_spans_into_one_definition() {
    let text = "[a|\n|-\n|\nb]:v";
    for cut in 1..text.len() {
        assert_streams_like_the_whole(text, &[cut]);
    }
}

#[test]
fn an_empty_push_between_a_carriage_return_and_its_line_feed_keeps_one_ending() {
    let mut document = Document::default();
    for piece in ["a\r", "", "\nb"] {
        document.push(piece);
    }
    assert_eq!(document.text(), "a\nb");
}

#[test]
fn a_streaming_definition_changes_only_the_blocks_that_cite_it() {
    let mut document = Document::default();
    document.push("See [1].\n\nPlain text.\n\nAlso plain.\n\n[1]: https://exa");
    let changes = document.push("mple.com/page");
    assert_eq!(changes.changed, [0], "only the citing block changed");
    assert_streams_like_the_whole(document.text(), &[]);
}

#[test]
fn a_line_of_unicode_whitespace_does_not_settle_an_open_label() {
    let text = "[a|\n|-\n|\n\u{a0}\nb]:v\n";
    for cut in 1..text.len() {
        if text.is_char_boundary(cut) {
            assert_streams_like_the_whole(text, &[cut]);
        }
    }
}

#[test]
fn a_definition_whose_destination_decodes_to_a_line_break_resolves_later_blocks() {
    let text = "see [x]\n\nplain\n\n[x]: <a&#10;b> \"t&#13;u\"\n";
    for cut in 1..text.len() {
        assert_streams_like_the_whole(text, &[cut]);
    }
}

#[test]
fn quotes_and_lists_nested_past_the_depth_limit_parse_and_drop() {
    for marker in ["> ", "- ", "1. "] {
        let mut document = Document::default();
        document.push(&format!("{}deep\n", marker.repeat(100_000)));
        assert_streams_like_the_whole(&format!("{}deep\n", marker.repeat(200)), &[]);
        drop(document);
    }
}

#[test]
fn an_empty_item_past_the_depth_limit_gives_its_start_to_no_later_leaf() {
    let text = format!("{}-\n\n---\n\nlater\n", "> ".repeat(32));
    assert_streams_like_the_whole(&text, &[]);
}
