use super::*;

/// Parses `block` as one that starts the source.
fn parse(block: &str, before: &[Definition], after: &[Definition]) -> Parsed {
    super::parse(block, 0, before, after)
}

/// Parses `usage` as the block after `source`, a blank line apart, with
/// the definitions of `source` written back as `defined`.
fn rewritten(source: &str, usage: &str, defined: &[Definition]) -> Parsed {
    super::parse(usage, source.len() + 1, defined, &[])
}

/// The destination of the first link in `nodes`.
fn first_link(nodes: &[Node]) -> Option<String> {
    nodes.iter().find_map(|node| match node {
        Node::Paragraph { text, .. } => text.iter().find_map(|run| run.link.clone()),
        _ => None,
    })
}

/// The definitions `text` holds.
fn definitions(text: &str) -> Vec<Definition> {
    parse(text, &[], &[]).definitions
}

#[test]
fn a_reference_resolves_through_a_definition_after_the_block() {
    let before = parse("see [docs]\n", &[], &[]);
    assert_eq!(first_link(&before.nodes), None, "no definition yet");
    let after = parse(
        "see [docs]\n",
        &[],
        &definitions("[docs]: <https://example.com>\n"),
    );
    assert_eq!(
        first_link(&after.nodes).as_deref(),
        Some("https://example.com")
    );
    assert!(after.definitions.is_empty(), "the block defines nothing");
}

#[test]
fn the_first_definition_of_a_label_wins() {
    let parsed = parse(
        "[a]: /second\n\nsee [a]\n",
        &definitions("[a]: </first>\n"),
        &[],
    );
    assert_eq!(first_link(&parsed.nodes).as_deref(), Some("/first"));
    assert!(
        parsed.definitions.is_empty(),
        "a definition an earlier block holds is not the block's own"
    );
}

#[test]
fn a_definition_written_back_parses_to_the_same_link() {
    let source = "[Odd Label]: <a b&amp;\\<c> \"say \\\"hi\\\" & go\"\n";
    let defined = definitions(source);
    assert_eq!(defined.len(), 1);
    let original = parse(&format!("{source}\n[odd label]\n"), &[], &[]);
    let rewritten = rewritten(source, "[odd label]\n", &defined);
    assert_eq!(original.nodes, rewritten.nodes);
    assert!(rewritten.definitions.is_empty());
    assert!(first_link(&rewritten.nodes).is_some());
}

#[test]
fn a_tight_list_item_keeps_its_text_outside_a_paragraph() {
    assert_eq!(
        document("- a\n- b\n"),
        [Node::List {
            at: 0,
            start: None,
            items: vec![
                vec![Node::Text {
                    at: 0,
                    text: vec![Inline {
                        text: String::from("a"),
                        ..Inline::default()
                    }],
                }],
                vec![Node::Text {
                    at: 4,
                    text: vec![Inline {
                        text: String::from("b"),
                        ..Inline::default()
                    }],
                }],
            ],
        }]
    );
}

#[test]
fn a_later_definition_leaves_a_block_that_ends_open_as_it_was() {
    let later = definitions("[x]: /y\n");
    for block in ["- ```\n", "> ```\n", "    code\n", "<!-- open\n"] {
        assert_eq!(
            parse(block, &[], &later).nodes,
            document(block),
            "{block:?}"
        );
    }
}

#[test]
fn a_later_definition_does_not_override_the_block_s_own() {
    let parsed = parse("[a]: /own\n\nsee [a]\n", &[], &definitions("[a]: /later\n"));
    assert_eq!(first_link(&parsed.nodes).as_deref(), Some("/own"));
    assert_eq!(parsed.definitions.len(), 1);
}

#[test]
fn a_label_that_ends_in_a_backslash_is_written_back_whole() {
    let source = "[odd\\\n]: /x\n";
    let defined = definitions(source);
    assert_eq!(defined.len(), 1);
    let original = parse(&format!("{source}\n[odd\\\n]\n"), &[], &[]);
    let rewritten = rewritten(source, "[odd\\\n]\n", &defined);
    assert_eq!(original.nodes, rewritten.nodes);
    assert!(first_link(&rewritten.nodes).is_some());
}

#[test]
fn a_definition_with_decoded_line_breaks_is_written_back_whole() {
    let source = "[x]: <a&#10;b> \"t&#13;u\"\n";
    let defined = definitions(source);
    assert_eq!(defined.len(), 1);
    let original = parse(&format!("{source}\n[x]\n"), &[], &[]);
    let rewritten = rewritten(source, "[x]\n", &defined);
    assert_eq!(original.nodes, rewritten.nodes);
    assert_eq!(first_link(&rewritten.nodes).as_deref(), Some("a\nb"));
}

#[test]
fn a_label_of_the_longest_length_ending_in_escaped_backslashes_is_written_back_whole() {
    let label = format!("{} \\\\", "\\!".repeat(498));
    let source = format!("[{label}]: /ok\n");
    let usage = format!("[{label}]\n");
    let defined = definitions(&source);
    assert_eq!(defined.len(), 1);
    let original = parse(&format!("{source}\n{usage}"), &[], &[]);
    let rewritten = rewritten(&source, &usage, &defined);
    assert_eq!(first_link(&original.nodes).as_deref(), Some("/ok"));
    assert_eq!(original.nodes, rewritten.nodes);
}
