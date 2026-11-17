//! Selection across read-only text blocks.
//!
//! A [`selection_scope`] holds one selection over many labels and rich text
//! paragraphs, the blocks, in the order of a list of keys. The selection's
//! ends are a block's key and a byte offset into its text, so a selection
//! survives scrolling and virtualization: a block that mounts again finds
//! its part from the scope, and copy reads the text of blocks that are not
//! mounted through the scope's `text_of`.
//!
//! The scope takes the presses and holds the keyboard focus, which copy
//! needs. A block opts in with [`SelectableExt::selectable`] and listens with
//! [`Element::on_drag_over`], turning the point under a drag into an offset
//! with its own layout, so the hit test runs in the block's own coordinates
//! wherever a scroll view moved it. A drag held past the edge of a scroll
//! view inside the scope scrolls it, and the rows that pass the edge extend
//! the selection.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;
use std::rc::Rc;

use parley::editing::Selection as ParagraphSelection;
use parley::layout::Affinity;
use parley::{Brush, Cursor};
use smudge_core::element::{Element, IntoElement, Value, show};
use smudge_core::geometry::{Color, Corners, Point, Rect};
use smudge_core::input::{Drag, Handled, Key, KeyEvent};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{
    Owner, RenderEffect, RwSignal, prelude::*, provide_context, use_context,
};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{PaintCx, Panel, panel};
use web_time::Instant;

use crate::chrome::{DOUBLE_CLICK_DISTANCE, DOUBLE_CLICK_TIME, SELECTION_ALPHA};
use crate::label::Label;
use crate::layout::TextLayout;
use crate::rich_text::{LayoutSink, RichText};

/// One end of a selection: a block and a byte offset into its text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Endpoint<K> {
    /// The key of the block.
    pub(crate) key: K,
    /// The byte offset into the block's text.
    pub(crate) offset: usize,
}

/// What a drag found under the pointer: the block, the offset there, and the
/// word around it.
struct Hit<K> {
    /// The offset under the pointer.
    at: Endpoint<K>,
    /// The byte range of the word under the pointer.
    word: Range<usize>,
}

/// The order of a scope's blocks: their keys, and the position of each key,
/// built once each time the keys change, so a drag looks a key up instead of
/// scanning for it.
pub(crate) struct Order<K> {
    /// The keys in order.
    keys: Rc<[K]>,
    /// The position of each key in `keys`.
    positions: HashMap<K, usize>,
}

impl<K: Clone + Eq + Hash> Order<K> {
    /// Indexes `keys`. A key listed twice keeps its first position.
    pub(crate) fn new(keys: Rc<[K]>) -> Order<K> {
        let mut positions = HashMap::with_capacity(keys.len());
        for (at, key) in keys.iter().enumerate() {
            positions.entry(key.clone()).or_insert(at);
        }
        Order { keys, positions }
    }

    /// Returns the position of `key`, or `None` when it is not listed.
    fn position(&self, key: &K) -> Option<usize> {
        self.positions.get(key).copied()
    }
}

/// Returns `a` and `b` in block order, then offset order, or `None` when
/// either key is not in `order`.
pub(crate) fn ordered<K: Clone + Eq + Hash>(
    order: &Order<K>,
    a: &Endpoint<K>,
    b: &Endpoint<K>,
) -> Option<(Endpoint<K>, Endpoint<K>)> {
    let first = order.position(&a.key)?;
    let second = order.position(&b.key)?;
    if (first, a.offset) <= (second, b.offset) {
        Some((a.clone(), b.clone()))
    } else {
        Some((b.clone(), a.clone()))
    }
}

/// Returns the byte range the selection from `start` to `end`, in order,
/// covers in the block of `key`, with `usize::MAX` for a block selected to
/// its end, or `None` when it covers nothing of it.
pub(crate) fn block_range<K: Clone + Eq + Hash>(
    order: &Order<K>,
    start: &Endpoint<K>,
    end: &Endpoint<K>,
    key: &K,
) -> Option<Range<usize>> {
    let at = order.position(key)?;
    let first = order.position(&start.key)?;
    let last = order.position(&end.key)?;
    if at < first || at > last {
        return None;
    }
    let from = if at == first { start.offset } else { 0 };
    let to = if at == last { end.offset } else { usize::MAX };
    (from < to).then_some(from..to)
}

/// Joins the selected text of every block from `start` to `end`, in order,
/// with a newline between blocks, reading each block's text through
/// `text_of`.
pub(crate) fn join<K: Clone + Eq + Hash>(
    order: &Order<K>,
    start: &Endpoint<K>,
    end: &Endpoint<K>,
    text_of: impl Fn(&K) -> String,
) -> String {
    let (Some(first), Some(last)) = (order.position(&start.key), order.position(&end.key)) else {
        return String::new();
    };
    let mut joined = String::new();
    for (at, key) in order.keys.iter().enumerate().take(last + 1).skip(first) {
        let text = text_of(key);
        let from = if at == first { start.offset } else { 0 };
        let to = if at == last { end.offset } else { text.len() };
        let from = floor_boundary(&text, from);
        let to = floor_boundary(&text, to).max(from);
        if at > first {
            joined.push('\n');
        }
        joined.push_str(&text[from..to]);
    }
    joined
}

/// Moves the ends of the selection from `start` to `end`, ordered in `old`,
/// onto the blocks that `new` still lists: an end whose block is gone moves
/// to the nearest surviving block inside the selection, the start to its
/// beginning and the end to its end. Returns `None` when no block between
/// them survives.
pub(crate) fn carried<K: Clone + Eq + Hash>(
    old: &Order<K>,
    new: &Order<K>,
    start: &Endpoint<K>,
    end: &Endpoint<K>,
) -> Option<(Endpoint<K>, Endpoint<K>)> {
    let first = old.position(&start.key)?;
    let last = old.position(&end.key)?;
    let between = &old.keys[first..=last];
    let start = if new.position(&start.key).is_some() {
        start.clone()
    } else {
        let key = between.iter().find(|key| new.position(key).is_some())?;
        Endpoint {
            key: key.clone(),
            offset: 0,
        }
    };
    let end = if new.position(&end.key).is_some() {
        end.clone()
    } else {
        let key = between
            .iter()
            .rev()
            .find(|key| new.position(key).is_some())?;
        Endpoint {
            key: key.clone(),
            offset: usize::MAX,
        }
    };
    ordered(new, &start, &end)
}

/// Returns `offset` clamped into `text` and moved back to a character
/// boundary.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The signal of a mounted block's selected range.
type RangeSignal = RwSignal<Option<Range<usize>>>;

/// The selection a scope holds, shared with its blocks through reactive
/// context.
pub(crate) struct Scope<K> {
    /// The keys of every block in order, mounted or not.
    keys: Box<dyn Fn() -> Rc<[K]>>,
    /// The order the keys gave when they last changed.
    order: RefCell<Rc<Order<K>>>,
    /// The text of any block, for copy.
    text_of: Box<dyn Fn(&K) -> String>,
    /// The end the selection started from.
    anchor: RefCell<Option<Endpoint<K>>>,
    /// The end the pointer moves.
    focus: RefCell<Option<Endpoint<K>>>,
    /// What the drag found under the pointer last, cleared when the drag
    /// ends, whichever node took its press.
    over: RefCell<Option<Hit<K>>>,
    /// The selected range of each mounted block, by key, then by the id it
    /// registered with, since one key can be mounted more than once.
    blocks: RefCell<HashMap<K, Vec<(u64, RangeSignal)>>>,
    /// The id the next registration takes.
    next_block: Cell<u64>,
    /// Whether a press the scope took is held.
    pressing: Cell<bool>,
    /// When and where the last press began, for a double click.
    last_press: Cell<Option<(Instant, Point)>>,
}

impl<K: Clone + Eq + Hash + 'static> Scope<K> {
    /// Returns the selection's ends in order in `order`, or `None` when it is
    /// empty.
    fn ends_in(&self, order: &Order<K>) -> Option<(Endpoint<K>, Endpoint<K>)> {
        let anchor = self.anchor.borrow();
        let focus = self.focus.borrow();
        let (Some(anchor), Some(focus)) = (anchor.as_ref(), focus.as_ref()) else {
            return None;
        };
        if anchor == focus {
            return None;
        }
        ordered(order, anchor, focus)
    }

    /// Reads the keys again when they changed, carries the selection's ends
    /// onto the blocks that remain, and updates the mounted blocks.
    fn refresh(&self) {
        let keys = (self.keys)();
        let old = Rc::clone(&self.order.borrow());
        if !Rc::ptr_eq(&old.keys, &keys) {
            let new = Rc::new(Order::new(keys));
            // The ends are carried in order and handed back to the anchor and
            // the focus they came from, so a drag keeps moving its own end.
            let anchor_first = self
                .ends_in(&old)
                .is_none_or(|(start, _)| self.anchor.borrow().as_ref() == Some(&start));
            let ends = self
                .ends_in(&old)
                .and_then(|(start, end)| carried(&old, &new, &start, &end));
            let (anchor, focus) = match ends {
                Some((start, end)) if anchor_first => (Some(start), Some(end)),
                Some((start, end)) => (Some(end), Some(start)),
                None if self.pressing.get() => {
                    (self.anchor.borrow().clone(), self.focus.borrow().clone())
                }
                None => (None, None),
            };
            *self.anchor.borrow_mut() = anchor;
            *self.focus.borrow_mut() = focus;
            *self.order.borrow_mut() = new;
        }
        self.update();
    }

    /// Sets the range of every mounted block from the selection, waking only
    /// the blocks whose range changed.
    fn update(&self) {
        let order = Rc::clone(&self.order.borrow());
        let ends = self.ends_in(&order);
        let mut changes: Vec<(RangeSignal, Option<Range<usize>>)> = Vec::new();
        for (key, blocks) in self.blocks.borrow().iter() {
            let range = ends
                .as_ref()
                .and_then(|(start, end)| block_range(&order, start, end, key));
            for (_, signal) in blocks {
                if signal.read(|current| *current != range) {
                    changes.push((signal.clone(), range.clone()));
                }
            }
        }
        for (signal, range) in changes {
            signal.set(range);
        }
    }

    /// Registers a mounted block of `key`, and returns its id and the signal
    /// of its selected range, starting from the selection as it is.
    fn register(&self, key: &K) -> (u64, RangeSignal) {
        let order = Rc::clone(&self.order.borrow());
        let range = self
            .ends_in(&order)
            .and_then(|(start, end)| block_range(&order, &start, &end, key));
        let signal = RwSignal::new(range);
        let id = self.next_block.get();
        self.next_block.set(id + 1);
        self.blocks
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .push((id, signal.clone()));
        (id, signal)
    }

    /// Forgets the block of `key` that registered as `id`, leaving any other
    /// block of the same key in place.
    fn unregister(&self, key: &K, id: u64) {
        let mut blocks = self.blocks.borrow_mut();
        if let Some(list) = blocks.get_mut(key) {
            list.retain(|(block, _)| *block != id);
            if list.is_empty() {
                blocks.remove(key);
            }
        }
    }

    /// Starts a selection where the press found a block, selecting the word
    /// there on a double click, or clears it when the press found none.
    fn press(&self, at: Point) {
        self.pressing.set(true);
        let now = smudge_core::timer::now();
        let double = self.last_press.get().is_some_and(|(then, there)| {
            now.duration_since(then) <= DOUBLE_CLICK_TIME
                && (at.x - there.x).powi(2) + (at.y - there.y).powi(2)
                    <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
        });
        self.last_press
            .set(if double { None } else { Some((now, at)) });
        let (anchor, focus) = match self.over.borrow().as_ref() {
            Some(hit) if double => (
                Some(Endpoint {
                    key: hit.at.key.clone(),
                    offset: hit.word.start,
                }),
                Some(Endpoint {
                    key: hit.at.key.clone(),
                    offset: hit.word.end,
                }),
            ),
            Some(hit) => (Some(hit.at.clone()), Some(hit.at.clone())),
            None => (None, None),
        };
        *self.anchor.borrow_mut() = anchor;
        *self.focus.borrow_mut() = focus;
        self.update();
    }

    /// Records what a drag found under the pointer, and moves the
    /// selection's moving end there while the scope's press is held. A block
    /// calls it each time a drag reaches it, by a pointer move or by content
    /// that scrolled under the pointer.
    fn drag_over(&self, hit: Hit<K>) {
        let at = hit.at.clone();
        *self.over.borrow_mut() = Some(hit);
        if !self.pressing.get() || self.anchor.borrow().is_none() {
            return;
        }
        if self.focus.borrow().as_ref() != Some(&at) {
            *self.focus.borrow_mut() = Some(at);
            self.update();
        }
    }

    /// Ends the drag, whichever node took its press.
    fn release(&self) {
        self.pressing.set(false);
        *self.over.borrow_mut() = None;
    }

    /// Returns the selected text of every block in order, with a newline
    /// between blocks, or `None` when nothing is selected.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let order = Rc::clone(&self.order.borrow());
        let (start, end) = self.ends_in(&order)?;
        Some(join(&order, &start, &end, &*self.text_of))
    }

    /// Writes the selected text to the clipboard.
    fn copy(&self) {
        if let Some(text) = self.selected_text() {
            smudge_core::clipboard::with(|clipboard| clipboard.set(text));
        }
    }
}

/// Paints the rectangles of `range` in `layout`, placed at the node's
/// origin, in the selection color the text inputs use.
pub(crate) fn paint_selected<B: Brush>(
    layout: &parley::Layout<B>,
    range: Range<usize>,
    cx: &PaintCx<'_>,
    out: &mut Vec<PaintOp>,
) {
    let color = Color {
        a: SELECTION_ALPHA,
        ..cx.theme.color(ColorToken::ACCENT)
    };
    let origin = cx.rect.origin();
    let selection = ParagraphSelection::new(
        Cursor::from_byte_index(layout, range.start, Affinity::Downstream),
        Cursor::from_byte_index(layout, range.end, Affinity::Downstream),
    );
    selection.geometry_with(layout, |area, _| {
        let rect = Rect::new(
            origin.x + area.x0 as f32,
            origin.y + area.y0 as f32,
            origin.x + area.x1 as f32,
            origin.y + area.y1 as f32,
        );
        // A truncated text keeps its hidden lines in the layout, so the
        // selection is cut to the node, where the text shows.
        let rect = rect.intersect(cx.rect);
        if rect.width() > 0.0 && rect.height() > 0.0 {
            out.push(PaintOp::FillRect {
                rect,
                radius: Corners::ZERO,
                color,
            });
        }
    });
}

/// Returns what lies under `at` in `layout`, for the block of `key`.
fn hit<K, B: Brush>(key: K, layout: &parley::Layout<B>, at: Point) -> Hit<K> {
    let offset = Cursor::from_point(layout, at.x, at.y).index();
    let word = ParagraphSelection::word_from_point(layout, at.x, at.y).text_range();
    Hit {
        at: Endpoint { key, offset },
        word,
    }
}

/// Creates a scope whose labels and rich text paragraphs share one
/// selection across blocks.
///
/// `keys` returns the key of every block in order, including blocks a
/// virtual list has not mounted, and `text_of` returns any block's plain
/// text, which copy reads. A block takes part with
/// [`SelectableExt::selectable`] and the key it has in `keys`. `content`
/// builds what the scope shows, inside the scope, so its blocks find it.
///
/// A press on a block starts a selection there, and a drag extends it,
/// across blocks and through a scroll view that scrolls while the drag is
/// held past its edge. A double click selects a word. The scope takes the
/// keyboard focus, and Ctrl+C, or Cmd+C on macOS, copies the selected text
/// of every block in order, with a newline between blocks. A press that
/// finds no block clears the selection. When `keys` changes, an end whose
/// block left moves to the nearest block of the selection that remains, so
/// trimming the oldest lines of a log keeps the rest of the selection, and a
/// selection none of whose blocks remains is cleared. A block that mounts
/// again shows its part of the selection.
///
/// `keys` runs inside an effect, and the scope reads the order again only
/// when it returns another `Rc`, so it must read the signals its list
/// changes with and hand back the same `Rc` until the list changes.
///
/// `text_of` must return the text the block shows, because the selection's
/// offsets are byte offsets into it: a label's text, or the text of a rich
/// text paragraph's spans joined with nothing between them.
///
/// The selection starts only from a press the scope takes. A row inside the
/// scope that takes presses itself, with a press or hover handler or a hover
/// style, keeps them, and a press on a link takes the keyboard focus from
/// the scope, so copy needs a press on the scope's text again.
///
/// ```
/// use std::rc::Rc;
///
/// use smudge_core::widget::panel;
/// use smudge_text::{SelectableExt, label, selection_scope};
///
/// let lines: Rc<[String]> = Rc::from([String::from("first"), String::from("second")]);
/// let texts = Rc::clone(&lines);
/// let scope = selection_scope(
///     || Rc::from([0usize, 1]),
///     move |at: &usize| texts[*at].clone(),
///     move || {
///         panel()
///             .column()
///             .child(label(lines[0].clone()).selectable(0usize))
///             .child(label(lines[1].clone()).selectable(1usize))
///     },
/// );
/// ```
pub fn selection_scope<K: Clone + Eq + Hash + 'static, E: IntoElement>(
    keys: impl Fn() -> Rc<[K]> + 'static,
    text_of: impl Fn(&K) -> String + 'static,
    content: impl Fn() -> E + 'static,
) -> Element<Panel> {
    let keys = smudge_core::hot::wrap(keys);
    let first = Rc::new(Order::new(keys()));
    let scope = Rc::new(Scope {
        keys: Box::new(keys),
        order: RefCell::new(first),
        text_of: Box::new(smudge_core::hot::wrap1_ref(text_of)),
        anchor: RefCell::new(None),
        focus: RefCell::new(None),
        over: RefCell::new(None),
        blocks: RefCell::new(HashMap::new()),
        next_block: Cell::new(0),
        pressing: Cell::new(false),
        last_press: Cell::new(None),
    });
    {
        let scope = Rc::clone(&scope);
        RenderEffect::new(move || scope.refresh());
    }
    let content = smudge_core::hot::wrap(content);
    let body = Rc::clone(&scope);
    let down = Rc::clone(&scope);
    let up = Rc::clone(&scope);
    let ended = Rc::clone(&scope);
    let keys = Rc::clone(&scope);
    panel()
        .column()
        .on_pointer_down(move |at| down.press(at))
        .on_pointer_up(move |_| up.release())
        // The scope's rectangle holds every press made inside it, so it hears
        // the end of a drag whose press a link or a row inside it took.
        .on_drag(move |step| {
            if matches!(step, Drag::Ended) {
                ended.release();
            }
            Handled::No
        })
        .on_key(move |event: &KeyEvent| {
            if event.pressed && event.modifiers.command() && is_copy(&event.key) {
                keys.copy();
                return Handled::Yes;
            }
            Handled::No
        })
        // The content mounts under an owner of its own, which carries the
        // scope, so blocks built inside it later, such as a virtual list's
        // rows, find the scope and siblings outside it do not.
        .child(
            show(
                || true,
                move || {
                    provide_context(Rc::clone(&body));
                    content()
                },
            )
            // The slot lays the content out as the scope's own column would,
            // stretched across and growing into the scope's height.
            .column()
            .grow(1.0),
        )
}

/// Returns whether `key` is the C of the copy shortcut.
fn is_copy(key: &Key) -> bool {
    matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("c"))
}

/// Makes a label or a rich text paragraph a block of the selection scope
/// around it.
///
/// Import the trait to use it.
pub trait SelectableExt: Sized {
    /// Makes the element the block of `key` in the [`selection_scope`]
    /// around it, so a drag over it selects its text and it paints its part
    /// of the selection.
    ///
    /// `key` must be the key the scope's `keys` lists for this block, of the
    /// same type. Outside a scope with keys of that type it does nothing,
    /// and the element behaves and costs as it would without it.
    fn selectable<K: Clone + Eq + Hash + 'static>(self, key: K) -> Self;
}

/// The key of a selectable block's layout sharing binding.
struct ShareKey;

/// The key of a selectable block's selected range binding.
struct SelectedKey;

/// Registers the block of `key` with `scope` for the current owner's
/// lifetime, and returns the signal of its selected range.
fn join_scope<K: Clone + Eq + Hash + 'static>(scope: &Rc<Scope<K>>, key: &K) -> RangeSignal {
    let (id, range) = scope.register(key);
    let scope = Rc::clone(scope);
    let key = key.clone();
    Owner::on_cleanup(move || scope.unregister(&key, id));
    range
}

impl SelectableExt for Element<Label> {
    fn selectable<K: Clone + Eq + Hash + 'static>(self, key: K) -> Self {
        let Some(scope) = use_context::<Rc<Scope<K>>>() else {
            return self;
        };
        let range = join_scope(&scope, &key);
        let shared: Rc<RefCell<Option<TextLayout>>> = Rc::new(RefCell::new(None));
        let layout = Rc::clone(&shared);
        self.bind(ShareKey, Value::Static(shared), crate::label::share_layout)
            .bind(SelectedKey, move || range.get(), crate::label::set_selected)
            .on_drag_over(move |at| {
                if let Some(layout) = layout.borrow().as_ref() {
                    scope.drag_over(hit(key.clone(), &layout.layout, at));
                }
            })
    }
}

impl SelectableExt for Element<RichText> {
    fn selectable<K: Clone + Eq + Hash + 'static>(self, key: K) -> Self {
        let Some(scope) = use_context::<Rc<Scope<K>>>() else {
            return self;
        };
        let range = join_scope(&scope, &key);
        let sink: LayoutSink = Rc::new(RefCell::new(None));
        let layout = Rc::clone(&sink);
        self.bind(
            ShareKey,
            Value::Static(sink),
            crate::rich_text::share_layout,
        )
        .bind(
            SelectedKey,
            move || range.get(),
            crate::rich_text::set_selected,
        )
        .on_drag_over(move |at| {
            let shared = layout.borrow();
            let Some(shared) = shared.as_ref() else {
                return;
            };
            if let Some(layout) = shared.borrow().as_ref() {
                scope.drag_over(hit(key.clone(), &layout.layout, at));
            }
        })
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
