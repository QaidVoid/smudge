//! Mirrors docs/guide/markdown.md: its Rust samples, held as functions.
//!
//! Every ```rust block of the page appears here exactly as the page
//! shows it, reindented one level inside the wrapper each block needs.
//! The page is the source of truth: change it here and there together.
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

#[test]
fn showing() {
    use smudge::markdown::{MarkdownOptions, MarkdownSource, markdown};
    use smudge::prelude::*;
    
    let reply = MarkdownSource::new();
    let following = RwSignal::new(true);
    let view = markdown(
        &reply,
        MarkdownOptions::default()
            .follow_end(following)
            .on_link(|destination| println!("open {destination}")),
    )
    .height(px(480.0));
    
    reply.push("# Answer\n\nThe build ");
    reply.push("**passed**.\n");
}
