//! Markdown for Smudge, rendered from a source that grows while it shows,
//! as a streamed reply does.
//!
//! The source divides into top-level blocks. A block closes once a later
//! one starts, and keeps its widgets, its layout and its glyph runs from
//! then on, so an appended token parses and lays out the open block alone.
//! A token that changes a link definition also parses again the blocks
//! that cite its label. The finished document renders as a parse of the
//! whole document would, reference links included, short of the parser's
//! limit on how much text references may expand to, which each block
//! counts on its own.

mod document;
mod model;
mod source;
mod split;
mod view;

pub use source::MarkdownSource;
pub use view::{MarkdownOptions, markdown};
