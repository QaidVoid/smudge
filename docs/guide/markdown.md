# Markdown

<p class="lead">Show markdown that grows while it is on screen, such as a reply streamed in tokens. Each appended token parses and lays out only the block it lands in, and the finished document renders as a parse of the whole of it would, short of one <a href="/reference/limits#widgets">limit on reference links</a>.</p>

## Setting it up

Markdown lives behind the `markdown` feature of `smudge`:

```toml
[dependencies]
smudge = { version = "0.3", features = ["markdown"] }
```

## Showing a document

A `MarkdownSource` holds the document, and `markdown` shows it. Push text into the source as it arrives, and every view of it follows:

```rust
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
```

`follow_end` keeps the view at the end of the document while the signal reads `true`, the way a scroll view's `follow_end` does, so a reply stays in view as it streams. A wheel step up writes `false`, and writing `true` jumps back to the end. `on_link` runs with a link's destination when it is clicked. Without it, links show as text. `clear` empties the source for the next document.

The source divides into top-level blocks: paragraphs, headings, lists, quotes, code blocks, tables and thematic breaks. A block closes once the next one starts, and from then on it keeps its widgets, its layout and its glyph runs, so a token that grows the open block costs the same at any length of the document. A block with reference links is the exception: it parses with the document's link definitions written ahead of it. Opening a block, and a new paragraph, list item or table row in the open block, also renew the document's list of blocks or of selectable text, which takes longer as the document grows. The list mounts only the blocks near the viewport, so a transcript of thousands of blocks stays cheap to scroll. The whole document sits in one [selection scope](./text-and-input#selecting-text), so a drag or the keyboard selects across blocks, and copy takes the text of every block it covers.

A reference link whose definition has not arrived shows its source text, `[like this]`. When a later block brings the definition, only the blocks that cite its label update, and nothing above or below them mounts again.

## What renders

Headings, paragraphs, emphasis, strong text, inline code, links, bullet and numbered lists, block quotes, fenced and indented code, thematic breaks and tables render. HTML shows as its source, and an image shows its description. Every line ending, a carriage return or a carriage return and line feed included, reads as a line feed.
