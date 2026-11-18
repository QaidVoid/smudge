//! Fuzzes the markdown document with random text pushed in random pieces.
//!
//! The first byte sets how many of the following bytes are piece lengths,
//! and the rest is the text. Nothing may panic, the document must hold the
//! text with its line endings turned into line feeds, the blocks the pieces
//! leave must be the ones splitting that text at once gives, and their
//! nodes must add up to the parse of the whole of it, wherever no parse can
//! run out of the parser's budget for expanding references, which each
//! block counts on its own.

#![no_main]

use libfuzzer_sys::fuzz_target;

// The target reads the document's own modules, which the crate keeps
// private, and calls only part of them.
#[allow(dead_code)]
#[path = "../../crates/smudge-markdown/src/document.rs"]
mod document;
#[path = "../../crates/smudge-markdown/src/model.rs"]
mod model;
#[path = "../../crates/smudge-markdown/src/split.rs"]
mod split;

fuzz_target!(|data: &[u8]| {
    let Some((&count, rest)) = data.split_first() else {
        return;
    };
    let (lengths, text) = rest.split_at(usize::from(count % 32).min(rest.len()));
    let text = String::from_utf8_lossy(text);
    let mut document = document::Document::default();
    let mut from = 0;
    for &length in lengths {
        let mut to = (from + usize::from(length) + 1).min(text.len());
        while !text.is_char_boundary(to) {
            to += 1;
        }
        document.push(&text[from..to]);
        from = to;
    }
    document.push(&text[from..]);

    let text = document::normalize_line_endings(&text);
    assert_eq!(document.text(), text, "the text of {text:?}");
    let starts: Vec<usize> = document.blocks().iter().map(|block| block.start).collect();
    assert_eq!(starts, split::split(&text), "the blocks of {text:?}");
    // Every parse may expand references by at least this many bytes, and
    // no reference expands by more than the longest destination and title.
    const BUDGET: usize = 100_000;
    let parser = pulldown_cmark::Parser::new_ext(&text, model::options());
    let longest = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| {
            definition.dest.len() + definition.title.as_ref().map_or(0, |title| title.len())
        })
        .max()
        .unwrap_or(0);
    if text.matches('[').count() * longest >= BUDGET {
        return;
    }
    let nodes: Vec<model::Node> = document
        .blocks()
        .iter()
        .flat_map(|block| block.nodes.iter().cloned())
        .collect();
    assert_eq!(nodes, model::document(&text), "the structure of {text:?}");
});
