//! The widget that shows a [`MarkdownSource`].

use std::rc::Rc;

use smudge_core::element::{AnyElement, Element, IntoElement, Slot, for_each};
use smudge_core::geometry::Size;
use smudge_core::reactive::{Memo, RwSignal, prelude::*};
use smudge_core::style::{Align, pct, px};
use smudge_core::theme::{ColorToken, RadiusToken};
use smudge_core::widget::{Panel, panel};
use smudge_text::{
    FontWeight, RichText, RichTextExt, SelectableExt, Span, SpanStyle, current_text_style, label,
    rich_text, selection_scope,
};
use smudge_widgets::{ScrollExt, row, virtual_list_keyed};

use crate::model::{Inline, Node};
use crate::source::{
    BlockKey, Leaf, MarkdownSource, children_at, inlines_at, leaf_at, node_at, node_start,
};

/// What a click on a link runs, with the link's destination.
type LinkHandler = Rc<dyn Fn(&str)>;

/// How a [`markdown`] view behaves, built with its methods.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_markdown::MarkdownOptions;
///
/// let options = MarkdownOptions::default()
///     .follow_end(RwSignal::new(true))
///     .on_link(|destination| println!("open {destination}"));
/// ```
#[derive(Clone, Default)]
pub struct MarkdownOptions {
    /// The signal the view follows the end of the document with.
    follow: Option<RwSignal<bool>>,
    /// What a click on a link runs.
    on_link: Option<LinkHandler>,
}

impl MarkdownOptions {
    /// Keeps the view at the end of the document while `follow` reads
    /// `true`, as [`ScrollExt::follow_end`] describes, so a streamed reply
    /// stays in view as it grows.
    pub fn follow_end(mut self, follow: RwSignal<bool>) -> MarkdownOptions {
        self.follow = Some(follow);
        self
    }

    /// Runs `handler` with a link's destination when the link is clicked,
    /// and keeps running its newest code across hot patches. Without a
    /// handler, links show as text.
    pub fn on_link(mut self, handler: impl Fn(&str) + 'static) -> MarkdownOptions {
        self.on_link = Some(Rc::new(smudge_core::hot::wrap1_ref::<str, (), _>(handler)));
        self
    }
}

/// The height a block is assumed to have before it is measured.
const ESTIMATE: f32 = 32.0;

/// The space between two blocks, and between the blocks inside a list
/// item or a quote.
const GAP: f32 = 8.0;

/// Shows `source`, growing as it grows.
///
/// The blocks render through a keyed virtual list, which mounts only the
/// blocks near the viewport, and sit in a selection scope, so the whole
/// document selects and copies across blocks. A closed block keeps its
/// widgets, and the open one updates its text in place.
///
/// Headings, paragraphs, emphasis, strong text, inline code, links,
/// lists, block quotes, code blocks, thematic breaks and tables render.
/// HTML shows as its source, and an image as its description.
///
/// ```
/// use smudge_markdown::{MarkdownOptions, MarkdownSource, markdown};
///
/// let reply = MarkdownSource::new();
/// let view = markdown(&reply, MarkdownOptions::default());
/// reply.push("**Done.** Three tests passed.\n");
/// ```
pub fn markdown(source: &MarkdownSource, options: MarkdownOptions) -> Element<Panel> {
    let keys = source.clone();
    let texts = source.clone();
    let rows = source.clone();
    let options = Rc::new(options);
    selection_scope(
        move || keys.leaves(),
        move |leaf: &Leaf| texts.leaf_text(leaf),
        move || {
            let keys = rows.clone();
            let blocks = rows.clone();
            let built = Rc::clone(&options);
            let list = virtual_list_keyed(
                move || keys.keys(),
                |key| *key,
                ESTIMATE,
                move |key| block_view(&blocks, *key, &built),
            )
            .width(pct(100.0))
            .grow(1.0);
            match &options.follow {
                Some(follow) => list.follow_end(follow.clone()),
                None => list,
            }
        },
    )
}

/// The row of the block `key` names.
fn block_view(source: &MarkdownSource, key: BlockKey, options: &Rc<MarkdownOptions>) -> AnyElement {
    let Some(nodes) = source.nodes(key) else {
        return panel().into_any();
    };
    let view = View {
        nodes,
        block: key,
        options: Rc::clone(options),
    };
    view.children(Rc::from([]))
        .padding_xy(Size {
            width: 0.0,
            height: GAP / 2.0,
        })
        .into_any()
}

/// What every widget of one block's row reads from.
#[derive(Clone)]
struct View {
    /// The block's nodes.
    nodes: RwSignal<Rc<[Node]>>,
    /// The block's key, which names its leaves.
    block: BlockKey,
    /// The view's options.
    options: Rc<MarkdownOptions>,
}

/// What kind of node sits at a place, which decides the widget built for
/// it, so a node that changes kind gets a new widget.
fn kind(node: &Node) -> u8 {
    match node {
        Node::Heading { level, .. } => *level,
        Node::Paragraph { .. } => 10,
        Node::Text { .. } => 11,
        Node::List { start: None, .. } => 12,
        Node::List { start: Some(_), .. } => 13,
        Node::Quote { .. } => 14,
        Node::Code { .. } => 15,
        Node::Html { .. } => 16,
        Node::Rule => 17,
        Node::Table { .. } => 18,
    }
}

impl View {
    /// A column of the nodes the container at `path` holds, which mounts a
    /// widget for each node once and again only when its kind or its start
    /// in the source changes.
    fn children(&self, path: Rc<[u32]>) -> Element<Slot> {
        let nodes = self.nodes.clone();
        let at = Rc::clone(&path);
        let view = self.clone();
        for_each(
            move || {
                nodes.with(|nodes| {
                    children_at(nodes, &at)
                        .map(|children| {
                            children
                                .iter()
                                .enumerate()
                                .map(|(index, node)| (index as u32, kind(node), node_start(node)))
                                .collect()
                        })
                        .unwrap_or_default()
                })
            },
            |entry| *entry,
            move |(index, _, _)| {
                let mut child: Vec<u32> = path.to_vec();
                child.push(index);
                view.node(Rc::from(child))
            },
        )
        .column()
        .gap(Size {
            width: 0.0,
            height: GAP,
        })
        .width(pct(100.0))
    }

    /// The widget of the node at `path`.
    fn node(&self, path: Rc<[u32]>) -> AnyElement {
        let node = self.nodes.read(|nodes| node_at(nodes, &path).cloned());
        let Some(node) = node else {
            return panel().into_any();
        };
        let base = current_text_style().size;
        match node {
            Node::Heading { level, .. } => {
                let scale = [2.0, 1.6, 1.3, 1.15, 1.0, 0.9][usize::from(level.clamp(1, 6)) - 1];
                self.leaf(path)
                    .text_size(base * scale)
                    .weight(FontWeight::BOLD)
                    .into_any()
            }
            Node::Paragraph { .. } | Node::Text { .. } | Node::Html { .. } => {
                self.leaf(path).into_any()
            }
            Node::Code { .. } => panel()
                .width(pct(100.0))
                .padding(px(8.0))
                .corner_radius(RadiusToken::MEDIUM)
                .background(ColorToken::SURFACE_HOVER)
                .child(self.leaf(path))
                .into_any(),
            Node::Rule => panel()
                .width(pct(100.0))
                .height(px(1.0))
                .background(ColorToken::BORDER)
                .into_any(),
            Node::Quote { .. } => row()
                .width(pct(100.0))
                .gap(Size {
                    width: 10.0,
                    height: 0.0,
                })
                .align_items(Align::Stretch)
                .child(panel().width(px(3.0)).background(ColorToken::BORDER))
                .child(self.children(path).grow(1.0))
                .into_any(),
            Node::List { start, .. } => self.list(path, start).into_any(),
            Node::Table { .. } => self.table(path).into_any(),
        }
    }

    /// A list's items, each a marker beside the item's nodes.
    fn list(&self, path: Rc<[u32]>, start: Option<u64>) -> Element<Slot> {
        let nodes = self.nodes.clone();
        let at = Rc::clone(&path);
        let view = self.clone();
        for_each(
            move || {
                nodes.with(|nodes| match node_at(nodes, &at) {
                    Some(Node::List { items, .. }) => (0..items.len() as u32).collect(),
                    _ => Vec::new(),
                })
            },
            |item| *item,
            move |item| {
                let marker = match start {
                    Some(first) => format!("{}.", first + u64::from(item)),
                    None => String::from("•"),
                };
                let mut inside: Vec<u32> = path.to_vec();
                inside.push(item);
                row()
                    .width(pct(100.0))
                    .gap(Size {
                        width: 6.0,
                        height: 0.0,
                    })
                    .child(label(marker).min_width(px(22.0)).shrink(0.0))
                    .child(view.children(Rc::from(inside)).grow(1.0).shrink(1.0))
            },
        )
        .column()
        .gap(Size {
            width: 0.0,
            height: GAP / 2.0,
        })
        .width(pct(100.0))
    }

    /// A table's rows, the header first, each a row of equal cells.
    fn table(&self, path: Rc<[u32]>) -> Element<Slot> {
        let nodes = self.nodes.clone();
        let at = Rc::clone(&path);
        let view = self.clone();
        for_each(
            move || {
                nodes.with(|nodes| match node_at(nodes, &at) {
                    Some(Node::Table { head, rows, .. }) => std::iter::once(head)
                        .chain(rows.iter())
                        .enumerate()
                        .map(|(row, cells)| (row as u32, cells.len() as u32))
                        .collect(),
                    _ => Vec::new(),
                })
            },
            |entry| *entry,
            move |(row_index, columns)| {
                let mut cells = row().width(pct(100.0));
                for column in 0..columns {
                    let mut cell: Vec<u32> = path.to_vec();
                    cell.extend([row_index, column]);
                    let text = view.leaf(Rc::from(cell));
                    let text = if row_index == 0 {
                        text.weight(FontWeight::BOLD)
                    } else {
                        text
                    };
                    cells = cells.child(
                        panel()
                            .grow(1.0)
                            .basis(px(0.0))
                            .padding(px(6.0))
                            .border(1.0)
                            .border_color(ColorToken::BORDER)
                            .child(text),
                    );
                }
                cells
            },
        )
        .column()
        .width(pct(100.0))
        .border(1.0)
        .border_color(ColorToken::BORDER)
    }

    /// The paragraph of text at `path`, selectable as one leaf, whose runs
    /// update in place while the block stays open.
    fn leaf(&self, path: Rc<[u32]>) -> Element<RichText> {
        let nodes = self.nodes.clone();
        let at = Rc::clone(&path);
        let runs =
            Memo::new(move || nodes.with(|nodes| inlines_at(nodes, &at).unwrap_or_default()));
        let on_link = self.options.on_link.clone();
        let text = rich_text(move || runs.with(|runs| spans_of(runs, on_link.as_ref())));
        let key = self
            .nodes
            .read(|nodes| leaf_at(self.block.generation, nodes, &path));
        match key {
            Some(key) => text.selectable(key),
            None => text,
        }
    }
}

/// The spans that show `runs`, with links that call `on_link`.
fn spans_of(runs: &[Inline], on_link: Option<&LinkHandler>) -> Rc<[Span]> {
    runs.iter()
        .map(|run| {
            let link = match (&run.link, on_link) {
                (Some(dest), Some(on_link)) => {
                    let dest = dest.clone();
                    let on_link = Rc::clone(on_link);
                    let open: Rc<dyn Fn()> = Rc::new(move || on_link(&dest));
                    Some(open)
                }
                _ => None,
            };
            Span::new(
                run.text.clone(),
                SpanStyle {
                    weight: run.strong.then_some(FontWeight::BOLD),
                    italic: run.emphasis,
                    monospace: run.code,
                    link,
                    ..SpanStyle::default()
                },
            )
        })
        .collect()
}
