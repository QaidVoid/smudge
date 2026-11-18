//! What a block of markdown renders as, parsed with the link reference
//! definitions of the whole document.

use std::cell::RefCell;
use std::iter::Peekable;
use std::ops::Range;

use pulldown_cmark::{
    BrokenLink, CowStr, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd,
};

/// The parser extensions every parse uses, the splitter's included, so a
/// block parses the same wherever it is read.
pub(crate) fn options() -> Options {
    Options::ENABLE_TABLES
}

/// One run of inline text in one style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Inline {
    /// The text the run shows.
    pub(crate) text: String,
    /// Whether the run is strong.
    pub(crate) strong: bool,
    /// Whether the run is emphasized.
    pub(crate) emphasis: bool,
    /// Whether the run is inline code.
    pub(crate) code: bool,
    /// The destination of the link the run belongs to, if any.
    pub(crate) link: Option<String>,
}

impl Inline {
    /// Whether `self` and `other` share every style, so their text can
    /// form one run.
    fn same_style(&self, other: &Inline) -> bool {
        self.strong == other.strong
            && self.emphasis == other.emphasis
            && self.code == other.code
            && self.link == other.link
    }
}

/// One block-level element.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    /// A heading of the given level, from 1 to 6.
    Heading {
        /// Where the heading starts in the source.
        at: usize,
        /// The heading's level.
        level: u8,
        /// The heading's text.
        text: Vec<Inline>,
    },
    /// A paragraph.
    Paragraph {
        /// Where the paragraph starts in the source.
        at: usize,
        /// The paragraph's text.
        text: Vec<Inline>,
    },
    /// The text of a tight list item, which no paragraph wraps.
    Text {
        /// Where the text starts in the source.
        at: usize,
        /// The text.
        text: Vec<Inline>,
    },
    /// A list, numbered from `start` when it is ordered.
    List {
        /// Where the list starts in the source.
        at: usize,
        /// The first number of an ordered list, or `None` for a bullet list.
        start: Option<u64>,
        /// The blocks of each item.
        items: Vec<Vec<Node>>,
    },
    /// A block quote.
    Quote {
        /// Where the quote starts in the source.
        at: usize,
        /// The blocks the quote holds.
        children: Vec<Node>,
    },
    /// A fenced or indented code block.
    Code {
        /// Where the block starts in the source.
        at: usize,
        /// The block's text.
        text: String,
    },
    /// An HTML block, shown as its source.
    Html {
        /// Where the block starts in the source.
        at: usize,
        /// The block's source.
        text: String,
    },
    /// A thematic break.
    Rule,
    /// A table.
    Table {
        /// Where the table starts in the source.
        at: usize,
        /// The cells of the header row.
        head: Vec<Vec<Inline>>,
        /// The cells of each body row.
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

/// A link reference definition, written back as markdown that parses to
/// the same label, destination and title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Definition {
    /// The label, as the parser keys it.
    label: String,
    /// The definition's line, ending in a line feed.
    line: String,
}

/// A parsed block: what it renders as, the link reference definitions it
/// holds, and the labels it looked up.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    /// The block's elements.
    pub(crate) nodes: Vec<Node>,
    /// The definitions the block holds, in order.
    pub(crate) definitions: Vec<Definition>,
    /// Every label the parse looked a definition up for, found or not, as
    /// the parser read it. A definition of another label cannot change
    /// the block, short of the parser's limit on how much text references
    /// expand to, which grows with the text a parse reads.
    pub(crate) labels: Vec<String>,
}

/// Parses `block` against the definitions of the blocks `before` it and
/// `after` it, so its references resolve as they do in the whole
/// document, where the first definition of a label wins.
///
/// A block that looks no label up parses alone, since no definition can
/// change it. Otherwise every definition goes ahead of the block, a blank
/// line apart, since text after the block could join what it leaves
/// open, such as a fence in a list item. A later block's definition is
/// left out when the block or an earlier one defines its label already.
pub(crate) fn parse(
    block: &str,
    base: usize,
    before: &[Definition],
    after: &[Definition],
) -> Parsed {
    let alone = parse_from(block, 0, base);
    if alone.labels.is_empty() || (before.is_empty() && after.is_empty()) {
        return alone;
    }
    let mut prefix: String = before
        .iter()
        .map(|definition| definition.line.as_str())
        .collect();
    if !after.is_empty() {
        let mut probe = prefix.clone();
        if !probe.is_empty() {
            probe.push('\n');
        }
        probe.push_str(block);
        let defined = Parser::new_ext(&probe, options());
        let defined = defined.reference_definitions();
        for definition in after {
            if defined.get(&definition.label).is_none() {
                prefix.push_str(&definition.line);
            }
        }
    }
    let mut text = prefix;
    if !text.is_empty() {
        text.push('\n');
    }
    let from = text.len();
    text.push_str(block);
    parse_from(&text, from, base)
}

/// Parses `text`, of which the block is the part from `from` on, and the
/// part before it only definitions. The block starts at `base` in the
/// source, which places its nodes there.
fn parse_from(text: &str, from: usize, base: usize) -> Parsed {
    let missed = RefCell::new(Vec::new());
    let note_miss = |link: BrokenLink<'_>| -> Option<(CowStr<'_>, CowStr<'_>)> {
        missed.borrow_mut().push(link.reference.to_string());
        None
    };
    let parser = Parser::new_with_broken_link_callback(text, options(), Some(note_miss));
    let mut own: Vec<(usize, Definition)> = parser
        .reference_definitions()
        .iter()
        .filter(|(_, definition)| definition.span.start >= from)
        .map(|(label, definition)| {
            let title = definition.title.as_deref();
            (
                definition.span.start,
                Definition {
                    label: String::from(label),
                    line: definition_line(label, &definition.dest, title),
                },
            )
        })
        .collect();
    own.sort_by_key(|(start, _)| *start);
    let mut labels = Vec::new();
    let events = parser.into_offset_iter().inspect(|(event, _)| {
        if let Event::Start(Tag::Link { link_type, id, .. } | Tag::Image { link_type, id, .. }) =
            event
            && matches!(
                link_type,
                LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
            )
        {
            labels.push(id.to_string());
        }
    });
    let nodes = nodes_of(events, from, base);
    labels.extend(missed.into_inner());
    Parsed {
        nodes,
        definitions: own.into_iter().map(|(_, definition)| definition).collect(),
        labels,
    }
}

/// Runs `read` with a test of whether one of `definitions` defines a
/// label, matched as the parser matches labels.
pub(crate) fn with_defined<R>(
    definitions: &[Definition],
    read: impl FnOnce(&dyn Fn(&str) -> bool) -> R,
) -> R {
    let text: String = definitions
        .iter()
        .map(|definition| definition.line.as_str())
        .collect();
    let parser = Parser::new_ext(&text, options());
    let defined = parser.reference_definitions();
    read(&|label| defined.get(label).is_some())
}

/// The structure the parse of a whole document gives, which the blocks of
/// a document must add up to.
pub(crate) fn document(text: &str) -> Vec<Node> {
    nodes_of(Parser::new_ext(text, options()).into_offset_iter(), 0, 0)
}

/// A definition line that parses back to `label`, `dest` and `title`.
/// Every ASCII punctuation character of the destination and the title is
/// escaped, so none of them reads as markup or as an entity.
///
/// The label stays as it is, since labels match on their raw text. One
/// that ends in an odd run of backslashes would escape its closing
/// bracket, so a space, which matching trims, keeps them apart. An even
/// run escapes itself and gets no space, which a label of the longest
/// length could not take.
fn definition_line(label: &str, dest: &str, title: Option<&str>) -> String {
    let trailing = label.len() - label.trim_end_matches('\\').len();
    let gap = if trailing % 2 == 1 { " " } else { "" };
    let mut line = format!("[{label}{gap}]: <{}>", escaped(dest));
    if let Some(title) = title {
        line.push_str(" \"");
        line.push_str(&escaped(title));
        line.push('"');
    }
    line.push('\n');
    line
}

/// `text` with a backslash before every ASCII punctuation character.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            // A destination holds no line break, and a title no blank
            // line, so the breaks an entity decoded to go back as entities.
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            ch => {
                if ch.is_ascii_punctuation() {
                    out.push('\\');
                }
                out.push(ch);
            }
        }
    }
    out
}

/// The elements `events` describe.
fn nodes_of<'a>(
    events: impl Iterator<Item = (Event<'a>, Range<usize>)>,
    from: usize,
    base: usize,
) -> Vec<Node> {
    Reader {
        events: events.peekable(),
        depth: 0,
        from,
        base,
        start: 0,
        item: None,
    }
    .blocks()
}

/// How deep lists and quotes nest in the model. The contents of a
/// container deeper than this join the container around it, so reading,
/// showing and dropping a document recurse a bounded number of times.
const MAX_DEPTH: usize = 32;

/// Whether `tag` opens a block that holds other blocks.
fn is_container(tag: &Tag<'_>) -> bool {
    is_block(tag)
        && !matches!(
            tag,
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::Table(_)
        )
}

/// Whether `tag` opens a block-level element rather than inline markup.
fn is_block(tag: &Tag<'_>) -> bool {
    !matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

/// Builds elements from a stream of parser events.
struct Reader<'a, I: Iterator<Item = (Event<'a>, Range<usize>)>> {
    /// The events left to read, each with its range in the parsed text.
    events: Peekable<I>,
    /// How many containers enclose the elements being read.
    depth: usize,
    /// Where the block starts in the parsed text.
    from: usize,
    /// Where the block starts in the source.
    base: usize,
    /// Where the event read last starts in the parsed text.
    start: usize,
    /// Where the list item being read starts in the source, until its first
    /// leaf takes it as its own start. A tight item's text starts at its
    /// first inline and a loose item's paragraph before it, so the item's
    /// start keeps the leaf's identity when the list turns loose.
    item: Option<usize>,
}

impl<'a, I: Iterator<Item = (Event<'a>, Range<usize>)>> Reader<'a, I> {
    /// Takes the next event, noting where it starts.
    fn next(&mut self) -> Option<Event<'a>> {
        let (event, range) = self.events.next()?;
        self.start = range.start;
        Some(event)
    }

    /// The next event, left to read.
    fn peek(&mut self) -> Option<&Event<'a>> {
        self.events.peek().map(|(event, _)| event)
    }

    /// The start a leaf found at `at` takes: that of the list item it is
    /// the first leaf of, if it is one.
    fn anchor(&mut self, at: usize) -> usize {
        self.item.take().unwrap_or(at)
    }

    /// Where the event read last starts in the source.
    fn last(&self) -> usize {
        self.base + self.start.saturating_sub(self.from)
    }

    /// Where the next event, or else the event read last, starts in the
    /// source.
    fn here(&mut self) -> usize {
        let start = self
            .events
            .peek()
            .map_or(self.start, |(_, range)| range.start);
        self.base + start.saturating_sub(self.from)
    }

    /// Reads elements up to the end of the container they sit in, which it
    /// consumes, or to the end of the events.
    fn blocks(&mut self) -> Vec<Node> {
        let mut nodes = Vec::new();
        while let Some(event) = self.peek() {
            match event {
                Event::End(_) => {
                    self.next();
                    break;
                }
                Event::Rule => {
                    self.next();
                    nodes.push(Node::Rule);
                }
                Event::Start(tag) if is_block(tag) => {
                    let at = self.here();
                    let Some(Event::Start(tag)) = self.next() else {
                        break;
                    };
                    if is_container(&tag) && self.depth >= MAX_DEPTH {
                        self.flatten(&mut nodes);
                    } else {
                        self.depth += 1;
                        nodes.push(self.block(tag, at));
                        self.depth -= 1;
                    }
                }
                _ => {
                    let at = self.here();
                    let at = self.anchor(at);
                    nodes.push(Node::Text {
                        at,
                        text: self.inlines(false),
                    });
                }
            }
        }
        nodes
    }

    /// Reads the contents of a container whose start was just read onto
    /// `nodes`, up to and including its end, with every container inside
    /// it read the same way, so nothing nests further.
    fn flatten(&mut self, nodes: &mut Vec<Node>) {
        let mut open = 1usize;
        while open > 0 {
            let Some(event) = self.peek() else {
                break;
            };
            match event {
                Event::End(end) => {
                    // An item that ends with no leaf leaves its start to no
                    // one, as the list reader does.
                    if matches!(end, TagEnd::Item) {
                        self.item = None;
                    }
                    self.next();
                    open -= 1;
                }
                Event::Rule => {
                    self.next();
                    nodes.push(Node::Rule);
                }
                Event::Start(tag) if is_container(tag) => {
                    let item = matches!(tag, Tag::Item);
                    self.next();
                    if item {
                        self.item = Some(self.last());
                    }
                    open += 1;
                }
                Event::Start(tag) if is_block(tag) => {
                    let at = self.here();
                    let Some(Event::Start(tag)) = self.next() else {
                        break;
                    };
                    nodes.push(self.block(tag, at));
                }
                _ => {
                    let at = self.here();
                    let at = self.anchor(at);
                    nodes.push(Node::Text {
                        at,
                        text: self.inlines(false),
                    });
                }
            }
        }
    }

    /// Reads the element `tag` opened at `at` in the source, up to and
    /// including its end.
    fn block(&mut self, tag: Tag<'a>, at: usize) -> Node {
        match tag {
            Tag::Paragraph => Node::Paragraph {
                at: self.anchor(at),
                text: self.inlines(true),
            },
            Tag::Heading { level, .. } => Node::Heading {
                at: self.anchor(at),
                level: heading_level(level),
                text: self.inlines(true),
            },
            Tag::CodeBlock(_) => Node::Code {
                at: self.anchor(at),
                text: self.raw(),
            },
            Tag::HtmlBlock => Node::Html {
                at: self.anchor(at),
                text: self.raw(),
            },
            Tag::List(start) => {
                let mut items = Vec::new();
                while let Some(event) = self.next() {
                    match event {
                        Event::Start(Tag::Item) => {
                            self.item = Some(self.last());
                            items.push(self.blocks());
                            self.item = None;
                        }
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                Node::List { at, start, items }
            }
            Tag::Table(_) => {
                let mut head = Vec::new();
                let mut rows = Vec::new();
                while let Some(event) = self.next() {
                    match event {
                        Event::Start(Tag::TableHead) => head = self.cells(),
                        Event::Start(Tag::TableRow) => rows.push(self.cells()),
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                Node::Table {
                    at: self.anchor(at),
                    head,
                    rows,
                }
            }
            // Block quotes, and the containers the options leave off,
            // hold blocks.
            _ => Node::Quote {
                at,
                children: self.blocks(),
            },
        }
    }

    /// Reads the cells of a table row, up to and including the row's end.
    fn cells(&mut self) -> Vec<Vec<Inline>> {
        let mut cells = Vec::new();
        while let Some(event) = self.next() {
            match event {
                Event::Start(Tag::TableCell) => cells.push(self.inlines(true)),
                Event::End(_) => break,
                _ => {}
            }
        }
        cells
    }

    /// Reads the text of a code or HTML block, up to and including its end.
    fn raw(&mut self) -> String {
        let mut text = String::new();
        while let Some(event) = self.next() {
            match event {
                Event::Text(part) | Event::Html(part) => text.push_str(&part),
                Event::End(_) => break,
                _ => {}
            }
        }
        text
    }

    /// Reads inline content. With `to_end`, it reads up to and including
    /// the end of the element it belongs to. Without, it stops before the
    /// next block-level event, as the text of a tight list item does.
    fn inlines(&mut self, to_end: bool) -> Vec<Inline> {
        let mut runs: Vec<Inline> = Vec::new();
        let mut open: Vec<Tag<'a>> = Vec::new();
        while let Some(event) = self.peek() {
            match event {
                Event::Start(tag) if is_block(tag) => break,
                Event::Rule => break,
                Event::End(_) if open.is_empty() => {
                    if to_end {
                        self.next();
                    }
                    break;
                }
                _ => {}
            }
            let Some(event) = self.next() else {
                break;
            };
            let (text, code) = match event {
                Event::Start(tag) => {
                    open.push(tag);
                    continue;
                }
                Event::End(_) => {
                    open.pop();
                    continue;
                }
                Event::Rule => continue,
                Event::Text(text)
                | Event::Html(text)
                | Event::InlineHtml(text)
                | Event::InlineMath(text)
                | Event::DisplayMath(text)
                | Event::FootnoteReference(text) => (text.into_string(), false),
                Event::Code(text) => (text.into_string(), true),
                Event::SoftBreak => (String::from(" "), false),
                Event::HardBreak => (String::from("\n"), false),
                Event::TaskListMarker(done) => {
                    (String::from(if done { "[x] " } else { "[ ] " }), false)
                }
            };
            push_run(&mut runs, &open, text, code);
        }
        runs
    }
}

/// Appends `text` in the style the `open` inline elements give it to
/// `runs`, joining the last run when the styles match.
fn push_run(runs: &mut Vec<Inline>, open: &[Tag<'_>], text: String, code: bool) {
    if text.is_empty() {
        return;
    }
    let mut run = Inline {
        text,
        code,
        ..Inline::default()
    };
    for tag in open {
        match tag {
            Tag::Strong => run.strong = true,
            Tag::Emphasis => run.emphasis = true,
            Tag::Link { dest_url, .. } => run.link = Some(dest_url.to_string()),
            _ => {}
        }
    }
    match runs.last_mut() {
        Some(last) if last.same_style(&run) => last.text.push_str(&run.text),
        _ => runs.push(run),
    }
}

/// A heading level as a number from 1 to 6.
fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
