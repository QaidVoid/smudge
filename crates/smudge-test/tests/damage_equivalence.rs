//! Property test that a surface repainted from damage equals a full repaint
//! and a fresh mount of the same scene.
//!
//! Every case builds a random tree of panels and text, applies a random
//! sequence of edits, and checks the surface after every frame three ways. The
//! comparison with a full repaint of the same display list fails when the
//! damage misses pixels. The comparison with a fresh app built from the final
//! value of every signal fails when the display list itself is stale, which a
//! missed invalidation causes and the first comparison cannot see. The check
//! that no work is left pending after a frame fails when the tree would need
//! another frame to settle, which the second layout pass after a hover
//! recompute exists to prevent. A difference in any of them is a stale pixel
//! on screen.
//!
//! Scenes carry style variants, and the second half of every case drives
//! pointer and keyboard events into them, so hover, press, focus and disabled
//! state reach the oracle like signal edits do. The case is generated in two
//! segments. The first applies every edit kind, all before the first event:
//! widths, visibility, hidden state and the viewport, which change geometry,
//! along with disabled toggles and the edits that only repaint. The second
//! applies events, disabled toggles and the edits that only repaint.
//! Every event of the second segment therefore meets the same tree the fresh
//! mount replays it on, which is what lets the mount reproduce the
//! interaction state by replaying the same events and disabled toggles on the
//! final tree, and the comparison to stay exact.
//!
//! The root also mounts a keyed list through `for_each`, whose rows show one
//! color and size per key, and whose rows with an even key overlap out of the
//! flow. List edits append a key, remove one, or reverse the list, so mounts,
//! removals, moves and restacks reach the oracle like any other geometry
//! edit. A row is built once per key and reads nothing but its key,
//! which is what lets a fresh mount rebuild the same rows from the final
//! list.
//!
//! The root also mounts a scroll view. A scroll edit moves the view's
//! content and its thumb without a layout, and an offset past either end
//! checks the clamp. The view's content holds an absolutely positioned badge,
//! which a scroll must leave where its insets put it, and a second, nested
//! scroll view with its own tracked offset, so a scroll of one view is a
//! damage computation over a subtree that contains another scroll. The view
//! measures its heights when it paints and applies them on the next frame, so
//! after every frame the case runs the frames those measurements ask for,
//! checking each one the same way, before it compares with a fresh mount.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use smudge_core::element::{AnyElement, Element, IntoElement, for_each, show};
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*, signal_local};
use smudge_core::style::{Direction, Edges, Length, px};
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_test::Harness;
use smudge_text::{TextLayout, TextStyle, TextSystem};
use smudge_widgets::{ScrollExt, ScrollView, column, scroll};

/// The viewport a scene starts in, in logical pixels.
const VIEWPORT: Size = Size {
    width: 320.0,
    height: 240.0,
};

/// The number of cases each property test runs when `PROPTEST_CASES` is unset.
const DEFAULT_CASES: u32 = 64;

/// The height of the scroll view the root mounts, in logical pixels.
const SCROLL_VIEW_HEIGHT: f32 = 96.0;

/// The height of one row of the scroll view's content, in logical pixels.
const SCROLL_ROW_HEIGHT: f32 = 30.0;

/// How many rows of content the scroll view holds.
const SCROLL_ROW_COUNT: usize = 5;

/// The deepest level a generated node may sit at, counting the root as one.
const MAX_DEPTH: u32 = 4;

/// The most children one generated node asks for.
const MAX_CHILDREN: u8 = 3;

/// The most frames the geometry segment runs, one comparison after each.
const MAX_GEOMETRY_FRAMES: usize = 5;

/// The most frames the event segment runs, one comparison after each.
const MAX_EVENT_FRAMES: usize = 4;

/// The most edits one frame applies, so a frame can carry several changes at
/// once.
const MAX_EDITS_PER_FRAME: usize = 4;

/// The proptest configuration the gate runs.
///
/// `PROPTEST_CASES` overrides the case count, so a longer run needs no change
/// to this file. A failure is written to `tests/proptest-regressions/`, beside
/// the test that found it, and that file is kept under version control.
fn config() -> ProptestConfig {
    ProptestConfig {
        cases: DEFAULT_CASES,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/proptest-regressions/damage_equivalence.txt"
        )))),
        ..ProptestConfig::default()
    }
}

/// A fixed length of up to 120 logical pixels or an automatic one.
fn any_length() -> impl Strategy<Value = Length> {
    prop_oneof![(0.0f32..=120.0).prop_map(Length::px), Just(Length::Auto)]
}

/// Any straight-alpha color, translucent ones included.
fn any_color() -> impl Strategy<Value = Color> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
        .prop_map(|(red, green, blue, alpha)| Color::rgba(red, green, blue, alpha))
}

/// The main axis a generated node lays its children out along.
fn any_direction() -> impl Strategy<Value = Direction> {
    prop_oneof![Just(Direction::Row), Just(Direction::Column)]
}

/// Any viewport a viewport edit can move the scene to, in logical pixels.
fn any_viewport() -> impl Strategy<Value = Size> {
    (160.0f32..=480.0, 120.0f32..=360.0).prop_map(|(width, height)| Size { width, height })
}

/// Any scale factor the test renders at.
fn any_scale() -> impl Strategy<Value = f32> {
    prop_oneof![Just(1.0), Just(1.25), Just(1.5), Just(2.0)]
}

/// The properties of one generated node, before any signal exists for them.
#[derive(Clone, Debug)]
struct NodeSpec {
    /// How many children the node asks for, before the budget and the depth
    /// limit cap it.
    children: u8,
    /// The width the node lays out with, when no signal supplies it.
    width: Length,
    /// The height the node lays out with.
    height: Length,
    /// The padding on all four sides.
    padding: f32,
    /// The space between adjacent children.
    gap: f32,
    /// The main axis the node lays its children out along.
    direction: Direction,
    /// The fill behind the node, when a signal does not supply one.
    background: Option<Color>,
    /// The border width, the same on all four sides.
    border: f32,
    /// The color of the border.
    border_color: Color,
    /// The corner radius, the same on all four corners, when no signal supplies
    /// it.
    radius: f32,
    /// Whether the node clips its children.
    clip: bool,
    /// Whether the node leaves the normal flow and places itself with its
    /// insets.
    absolute: bool,
    /// The offsets from the edges of the containing block, which only take
    /// effect on an absolutely positioned node.
    inset: Edges<Length>,
    /// Whether the width is read from a signal instead of being fixed.
    bind_width: bool,
    /// Whether the background is read from a signal instead of being fixed.
    bind_background: bool,
    /// Whether the corner radius is read from a signal instead of being fixed.
    bind_radius: bool,
    /// Whether the node is hidden through a signal instead of being laid out.
    bind_hidden: bool,
    /// Whether the node is mounted through a visibility signal.
    show: bool,
    /// Whether the node paints a line of text instead of only a panel.
    text: bool,
    /// Whether the text is read from a signal instead of being fixed.
    bind_text: bool,
    /// Whether the text color is read from a signal instead of being fixed.
    bind_glyph_color: bool,
    /// Whether the node restyles while it is in the hovered chain.
    hover_variant: bool,
    /// Whether the hovered patch also changes the width, which moves layout.
    hover_layout: bool,
    /// Whether the node restyles while a primary press is held on it.
    pressed_variant: bool,
    /// Whether the node can take the keyboard focus and restyles while it
    /// holds it.
    focused_variant: bool,
    /// Whether the node restyles while it is disabled.
    disabled_variant: bool,
    /// Whether the node's disabled state is read from a signal.
    bind_disabled: bool,
}

/// A node's layout properties.
fn any_layout() -> impl Strategy<Value = (u8, Length, Length, f32, f32, Direction)> {
    (
        0..=MAX_CHILDREN,
        any_length(),
        any_length(),
        0.0f32..=8.0,
        0.0f32..=8.0,
        any_direction(),
    )
}

/// A node's paint properties.
fn any_paint() -> impl Strategy<Value = (Option<Color>, f32, Color, f32, bool)> {
    (
        prop::option::of(any_color()),
        0.0f32..=3.0,
        any_color(),
        0.0f32..=12.0,
        any::<bool>(),
    )
}

/// Whether a node leaves the flow, and the offsets it places itself with when
/// it does.
fn any_position() -> impl Strategy<Value = (bool, Edges<Length>)> {
    (
        any::<bool>(),
        (any_length(), any_length(), any_length(), any_length()).prop_map(
            |(top, right, bottom, left)| Edges {
                top,
                right,
                bottom,
                left,
            },
        ),
    )
}

/// Which of a node's properties read a property instead of being fixed.
fn any_bindings() -> impl Strategy<Value = (bool, bool, bool, bool, bool)> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
}

/// Whether a node paints text, and which of its text properties read a signal.
fn any_text() -> impl Strategy<Value = (bool, bool, bool)> {
    (any::<bool>(), any::<bool>(), any::<bool>())
}

/// Which interaction states restyle the node, and whether its disabled state
/// is read from a signal.
///
/// Most panels carry a hovered patch, and half of those change the width with
/// it, so moving the pointer around a dense scene often restyles a node that
/// takes part in layout. That is the change whose invalidation needs the
/// second layout pass of a frame to settle.
fn any_variants() -> impl Strategy<Value = (bool, bool, bool, bool, bool, bool)> {
    (
        prop::bool::weighted(0.75),
        prop::bool::weighted(0.5),
        prop::bool::weighted(0.4),
        prop::bool::weighted(0.35),
        prop::bool::weighted(0.35),
        prop::bool::weighted(0.3),
    )
}

/// Any node of a generated scene.
fn any_spec() -> impl Strategy<Value = NodeSpec> {
    (
        any_layout(),
        any_paint(),
        any_position(),
        any_bindings(),
        any_text(),
        any_variants(),
    )
        .prop_map(|(layout, paint, position, bindings, text, variants)| {
            let (children, width, height, padding, gap, direction) = layout;
            let (background, border, border_color, radius, clip) = paint;
            let (absolute, inset) = position;
            let (bind_width, bind_background, bind_radius, bind_hidden, show) = bindings;
            let (text, bind_text, bind_glyph_color) = text;
            let (
                hover_variant,
                hover_layout,
                pressed_variant,
                focused_variant,
                disabled_variant,
                bind_disabled,
            ) = variants;
            NodeSpec {
                children,
                width,
                height,
                padding,
                gap,
                direction,
                background,
                border,
                border_color,
                radius,
                clip,
                absolute,
                inset,
                bind_width,
                bind_background,
                bind_radius,
                bind_hidden,
                show,
                text,
                bind_text,
                bind_glyph_color,
                hover_variant,
                hover_layout,
                pressed_variant,
                focused_variant,
                disabled_variant,
                bind_disabled,
            }
        })
}

/// A generated scene before any signal exists for it.
#[derive(Clone, Debug)]
struct SceneData {
    /// The properties of each node, in the order the nodes were created. The
    /// root is node zero and unused entries are dropped.
    nodes: Vec<NodeSpec>,
    /// The children of each node, indexed the same way.
    children: Vec<Vec<usize>>,
}

impl SceneData {
    /// Arranges `nodes` into a tree of at most their number of nodes and at
    /// most [`MAX_DEPTH`] levels.
    ///
    /// The result always holds a property of every kind an edit can target, so
    /// that every generated edit sequence changes something: the root reads
    /// its width, background and corner radius through a property, and the
    /// first child is mounted through a visibility property and, when no other
    /// node has one, hidden through a hidden property. The root itself is
    /// never mounted through one or hidden, because the harness builds it
    /// directly.
    fn new(mut nodes: Vec<NodeSpec>) -> SceneData {
        nodes[0].children = nodes[0].children.max(1);

        let mut children = vec![Vec::new(); nodes.len()];
        let mut next = 1;
        let mut stack = vec![(0usize, 1u32)];
        while let Some((index, depth)) = stack.pop() {
            let room = nodes.len().saturating_sub(next);
            let asked = usize::from(nodes[index].children);
            let count = if depth >= MAX_DEPTH {
                0
            } else {
                asked.min(room)
            };
            children[index] = (next..next + count).collect();
            next += count;
            for child in children[index].iter().rev() {
                stack.push((*child, depth + 1));
            }
        }

        nodes.truncate(next);
        children.truncate(next);

        nodes[0].show = false;
        nodes[0].bind_hidden = false;
        nodes[0].bind_width = true;
        nodes[0].bind_background = true;
        nodes[0].bind_radius = true;
        nodes[0].hover_variant = false;
        nodes[0].pressed_variant = false;
        nodes[0].focused_variant = false;
        nodes[0].disabled_variant = false;
        nodes[0].bind_disabled = false;
        if !nodes[1..].iter().any(|node| node.show) {
            nodes[1].show = true;
        }
        if !nodes[1..].iter().any(|node| node.bind_hidden) {
            nodes[1].bind_hidden = true;
        }
        if !nodes[1..].iter().any(|node| node.text) {
            nodes[1].text = true;
        }
        if !nodes[1..].iter().any(|node| node.text && node.bind_text) {
            nodes[1].text = true;
            nodes[1].bind_text = true;
        }
        if !nodes[1..]
            .iter()
            .any(|node| node.text && node.bind_glyph_color)
        {
            nodes[1].text = true;
            nodes[1].bind_glyph_color = true;
        }

        // A style patch is only visible on a panel, which paints the
        // background and the border a patch sets. When every child paints
        // text, one panel is appended so the interaction edits always have a
        // target that shows its patches.
        let variant = match nodes[1..].iter().position(|node| !node.text) {
            Some(offset) => offset + 1,
            None => {
                let index = nodes.len();
                nodes.push(NodeSpec {
                    children: 0,
                    width: Length::Auto,
                    height: Length::Auto,
                    padding: 2.0,
                    gap: 2.0,
                    direction: Direction::Row,
                    background: None,
                    border: 0.0,
                    border_color: Color::BLACK,
                    radius: 0.0,
                    clip: false,
                    absolute: false,
                    inset: Edges::all(Length::Auto),
                    bind_width: false,
                    bind_background: false,
                    bind_radius: false,
                    bind_hidden: false,
                    show: false,
                    text: false,
                    bind_text: false,
                    bind_glyph_color: false,
                    hover_variant: false,
                    hover_layout: false,
                    pressed_variant: false,
                    focused_variant: false,
                    disabled_variant: false,
                    bind_disabled: false,
                });
                children[0].push(index);
                children.push(Vec::new());
                index
            }
        };
        nodes[variant].hover_variant = true;
        nodes[variant].hover_layout = true;
        nodes[variant].pressed_variant = true;
        nodes[variant].focused_variant = true;
        nodes[variant].disabled_variant = true;
        nodes[variant].bind_disabled = true;

        SceneData { nodes, children }
    }

    /// How many properties of each kind the scene reads through one.
    fn signal_counts(&self) -> SignalCounts {
        SignalCounts {
            colors: self
                .nodes
                .iter()
                .filter(|node| node.bind_background)
                .count(),
            widths: self.nodes.iter().filter(|node| node.bind_width).count(),
            radii: self.nodes.iter().filter(|node| node.bind_radius).count(),
            shows: self.nodes.iter().filter(|node| node.show).count(),
            hiddens: self.nodes.iter().filter(|node| node.bind_hidden).count(),
            texts: self
                .nodes
                .iter()
                .filter(|node| node.text && node.bind_text)
                .count(),
            glyph_colors: self
                .nodes
                .iter()
                .filter(|node| node.text && node.bind_glyph_color)
                .count(),
            disableds: self.nodes.iter().filter(|node| node.bind_disabled).count(),
        }
    }
}

/// How many properties of each kind a scene reads through one.
#[derive(Copy, Clone, Debug)]
struct SignalCounts {
    colors: usize,
    widths: usize,
    radii: usize,
    shows: usize,
    hiddens: usize,
    texts: usize,
    glyph_colors: usize,
    disableds: usize,
}

/// Reads one property of a scene.
type Getter<T> = Rc<dyn Fn() -> T>;

/// The text a node starts with, chosen from its index.
fn initial_text(index: usize) -> String {
    let words = ["Hand", "Box", "Text", "Jump", "Lazy", "Fox", "Grim", "Wolf"];
    words[index % words.len()].to_owned()
}

/// The color a node's glyphs start with, chosen from its index.
fn initial_glyph_color(index: usize) -> Color {
    Color::rgb((index * 37) as u8, (index * 73) as u8, (index * 109) as u8)
}

/// The background the hovered patch of the node at `index` paints.
fn hover_background(index: usize) -> Color {
    Color::rgb(
        (index * 53 + 40) as u8,
        (index * 29 + 180) as u8,
        (index * 91 + 20) as u8,
    )
}

/// The width the hovered patch of the node at `index` lays it out with.
fn hover_width(index: usize) -> f32 {
    24.0 + (index * 17 % 32) as f32
}

/// The background the pressed patch of the node at `index` paints.
fn pressed_background(index: usize) -> Color {
    Color::rgb(
        (index * 71 + 10) as u8,
        (index * 13 + 30) as u8,
        (index * 47 + 200) as u8,
    )
}

/// The border color the focused patch of the node at `index` paints.
fn focus_border_color(index: usize) -> Color {
    Color::rgb(
        (index * 23 + 200) as u8,
        (index * 97 + 60) as u8,
        (index * 31 + 5) as u8,
    )
}

/// The background the disabled patch of the node at `index` paints.
fn disabled_background(index: usize) -> Color {
    Color::rgb(
        (index * 19 + 90) as u8,
        (index * 43 + 90) as u8,
        (index * 67 + 90) as u8,
    )
}

/// Writes one property of a reactive scene.
type Setter<T> = Box<dyn Fn(T)>;

/// One property of a scene: the value it shows, and how a scene reads it.
///
/// A reactive scene reads a signal, so an edit reaches the mounted elements
/// through their bindings. A static scene reads the cell alone, so a fresh
/// mount of the scene shows the values the reactive scene held when the mount
/// was built, without any signal. [`Tracked::write`] keeps the cell current in
/// both, which is what makes the fresh mount a picture of the final state.
struct Tracked<T: Copy> {
    /// Reads the value the scene shows now.
    get: Getter<T>,
    /// Writes the reactive scene's signal. A static scene has no signal, so
    /// its writer does nothing.
    set: Setter<T>,
    /// The value the scene shows now.
    value: Rc<Cell<T>>,
}

impl<T: Copy + 'static> Tracked<T> {
    /// Creates a property that edits change through a signal.
    ///
    /// The signal is created before the app, so it lives outside every node's
    /// owner and survives a node being removed and mounted again. The current
    /// value is kept in a cell rather than read back from the signal, because
    /// a read outside a binding makes the reactive runtime print a warning.
    fn reactive(value: T) -> Tracked<T> {
        let (read, write) = signal_local(value);
        Tracked {
            get: Rc::new(move || read.get()),
            set: Box::new(move |value| write.set(value)),
            value: Rc::new(Cell::new(value)),
        }
    }

    /// Creates a property that holds one value for the scene's whole life.
    fn static_value(value: T) -> Tracked<T> {
        let value = Rc::new(Cell::new(value));
        Tracked {
            get: Rc::new({
                let value = Rc::clone(&value);
                move || value.get()
            }),
            set: Box::new(|_| ()),
            value,
        }
    }

    /// Writes a new value through the cell and the signal, so the reactive
    /// scene and a fresh mount of it show the same thing.
    fn write(&self, value: T) {
        self.value.set(value);
        (self.set)(value);
    }
}

/// One text property of a scene: the string it shows, and how a scene reads
/// it.
///
/// It mirrors [`Tracked`] for values that do not fit in a [`Cell`]: the
/// current text is kept in a refcell, the reactive scene reads a signal, and a
/// static scene reads the refcell alone.
struct TextTracked {
    /// Reads the text the scene shows now.
    get: Getter<String>,
    /// Writes the reactive scene's signal. A static scene has no signal, so
    /// its writer does nothing.
    set: Box<dyn Fn(String)>,
    /// The text the scene shows now.
    value: Rc<RefCell<String>>,
}

impl TextTracked {
    /// Creates a text that edits change through a signal.
    fn reactive(value: String) -> TextTracked {
        let initial = value.clone();
        let (read, write) = signal_local(value);
        TextTracked {
            get: Rc::new(move || read.get()),
            set: Box::new(move |value| write.set(value)),
            value: Rc::new(RefCell::new(initial)),
        }
    }

    /// Creates a text that holds one string for the scene's whole life.
    fn static_value(value: String) -> TextTracked {
        let value = Rc::new(RefCell::new(value));
        TextTracked {
            get: Rc::new({
                let value = Rc::clone(&value);
                move || value.borrow().clone()
            }),
            set: Box::new(|_| ()),
            value,
        }
    }

    /// Writes a new string through the refcell and the signal, so the reactive
    /// scene and a fresh mount of it show the same thing.
    fn write(&self, value: String) {
        *self.value.borrow_mut() = value.clone();
        (self.set)(value);
    }
}

/// A node that paints one line of text on no background of its own.
///
/// The layout is built the first time the node paints with its current text
/// and kept until the text changes, so an edit to another node never relays
/// out this one. The color is read at paint time, so a color edit repaints
/// without rebuilding the layout.
struct TextBlock {
    /// The text the node paints.
    text: String,
    /// The color the glyphs paint with.
    color: Color,
    /// The layout of [`TextBlock::text`], kept with the text it was built
    /// from.
    layout: RefCell<Option<(String, TextLayout)>>,
}

impl TextBlock {
    /// Creates a text node that paints `text` in `color`.
    fn new(text: String, color: Color) -> TextBlock {
        TextBlock {
            text,
            color,
            layout: RefCell::new(None),
        }
    }
}

impl Widget for TextBlock {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let mut cache = self.layout.borrow_mut();
        if cache.as_ref().is_none_or(|(text, _)| text != &self.text) {
            let style = TextStyle::default();
            *cache = Some((self.text.clone(), TextLayout::new(&self.text, &style, None)));
        }
        let layout = &cache.as_ref().expect("the layout was just built").1;
        for run in layout.glyph_runs_at(cx.rect.origin(), self.color) {
            out.push(PaintOp::Glyphs(Rc::new(run)));
        }
    }
}

/// Writes one node's text. The node's own size does not depend on it, because
/// a text node measures nothing, so an edit repaints without laying out.
fn set_text(widget: &mut TextBlock, text: String) -> smudge_core::tree::Dirty {
    widget.text = text;
    smudge_core::tree::Dirty::Paint
}

/// Writes one node's glyph color, which repaints the node.
fn set_glyph_color(widget: &mut TextBlock, color: Color) -> smudge_core::tree::Dirty {
    widget.color = color;
    smudge_core::tree::Dirty::Paint
}

/// One keyed list of a scene: the keys it mounts, and how a scene reads them.
///
/// It mirrors [`TextTracked`] for a list of keys: the current keys are kept in
/// a refcell, the reactive scene reads a signal, and a static scene reads the
/// refcell alone.
struct ListTracked {
    /// Reads the keys the scene mounts now.
    get: Getter<Vec<u64>>,
    /// Writes the reactive scene's signal. A static scene has no signal, so
    /// its writer does nothing.
    set: Box<dyn Fn(Vec<u64>)>,
    /// The keys the scene mounts now.
    value: Rc<RefCell<Vec<u64>>>,
}

impl ListTracked {
    /// Creates a list that edits change through a signal.
    fn reactive(value: Vec<u64>) -> ListTracked {
        let initial = value.clone();
        let (read, write) = signal_local(value);
        ListTracked {
            get: Rc::new(move || read.get()),
            set: Box::new(move |value| write.set(value)),
            value: Rc::new(RefCell::new(initial)),
        }
    }

    /// Creates a list that holds one set of keys for the scene's whole life.
    fn static_value(value: Vec<u64>) -> ListTracked {
        let value = Rc::new(RefCell::new(value));
        ListTracked {
            get: Rc::new({
                let value = Rc::clone(&value);
                move || value.borrow().clone()
            }),
            set: Box::new(|_| ()),
            value,
        }
    }

    /// Writes a new list of keys through the refcell and the signal, so the
    /// reactive scene and a fresh mount of it mount the same rows.
    fn write(&self, value: Vec<u64>) {
        *self.value.borrow_mut() = value.clone();
        (self.set)(value);
    }
}

/// The offset signal of a scene's scroll view.
///
/// The view writes the offset it clamped back to the signal, so once a scene
/// has settled the signal holds the position on screen, and a fresh mount
/// starts from that value. A clamp is not undone by a later resize that
/// widens the range, so the value the edits wrote is not enough to rebuild
/// the scene.
struct ScrollTracked {
    /// The offset signal the scroll view's element is bound to.
    signal: RwSignal<f32>,
}

impl ScrollTracked {
    /// Creates an offset signal at `value`.
    fn new(value: f32) -> ScrollTracked {
        ScrollTracked {
            signal: RwSignal::new(value),
        }
    }

    /// Returns the offset the signal holds now, without tracking it.
    fn value(&self) -> f32 {
        self.signal.read(|offset| *offset)
    }

    /// Scrolls the scene's view to `offset`, which the view clamps.
    fn write(&self, offset: f32) {
        self.signal.set(offset);
    }
}

/// Where a scene's bound property values come from.
#[derive(Clone, Copy)]
enum Values<'a> {
    /// Signals at the values their specs start with, which is what a reactive
    /// scene reads.
    Signals,
    /// The values another scene holds now, which is what a fresh mount of it
    /// reads, except for the disabled bits. Those are signals at the values
    /// the slice holds, because the mount replays the toggles that followed.
    Current(&'a Scene, &'a [bool]),
}

/// One node of a built scene.
struct SceneNode {
    /// The properties the node's element is built from.
    spec: NodeSpec,
    /// The indices of the node's children.
    children: Vec<usize>,
    /// The background the node paints, when a property supplies it.
    color: Option<Getter<Color>>,
    /// The width the node lays out with, when a property supplies it.
    width: Option<Getter<Length>>,
    /// The corner radius the node paints with, when a property supplies it.
    radius: Option<Getter<f32>>,
    /// The hidden state of the node, when a property supplies it.
    hidden: Option<Getter<bool>>,
    /// The visibility the node is mounted with, when a property supplies it.
    show: Option<Getter<bool>>,
    /// The disabled state of the node, when a property supplies it.
    disabled: Option<Getter<bool>>,
    /// The text the node paints, when a property supplies it.
    text: Option<Getter<String>>,
    /// The color the node's glyphs paint with, when a property supplies it.
    glyph_color: Option<Getter<Color>>,
}

/// A built scene: its nodes and the properties its bindings read.
///
/// The handles stay here so an edit can change one property without rebuilding
/// the scene, and so a fresh mount of the scene can read the current value of
/// every property.
struct Scene {
    nodes: Vec<SceneNode>,
    colors: Vec<Tracked<Color>>,
    widths: Vec<Tracked<Length>>,
    radii: Vec<Tracked<f32>>,
    shows: Vec<Tracked<bool>>,
    hiddens: Vec<Tracked<bool>>,
    disableds: Vec<Tracked<bool>>,
    texts: Vec<TextTracked>,
    glyph_colors: Vec<Tracked<Color>>,
    /// The keys of the keyed list the root mounts, which a list edit changes.
    list: ListTracked,
    /// The offset of the scroll view the root mounts, which a scroll edit
    /// changes.
    scroll: ScrollTracked,
    /// The offset of the scroll view nested inside the outer one, which an
    /// inner scroll edit changes.
    inner_scroll: ScrollTracked,
    /// The viewport the scene is laid out in, which a viewport edit changes.
    viewport: Cell<Size>,
    /// The scale the scene renders at, which a viewport edit changes.
    scale: Cell<f32>,
}

impl Scene {
    /// Creates the reactive scene: every bound property is a signal at the
    /// value its spec starts with.
    ///
    /// Signals are what let an edit reach the mounted elements through their
    /// bindings.
    fn reactive(data: &SceneData, scale: f32) -> Scene {
        Scene::build(data, Values::Signals, VIEWPORT, scale)
    }

    /// Creates the scene that shows what `from` shows now: every bound
    /// property holds the value `from`'s property has at this moment, and the
    /// viewport and scale are copied from it.
    ///
    /// The disabled bits are the exception. They are signals at `disabled`,
    /// the values `from` held when its first event ran, so that replaying the
    /// events and toggles since then brings the mount to the state the
    /// reactive scene reached.
    fn fresh(data: &SceneData, from: &Scene, disabled: &[bool]) -> Scene {
        Scene::build(
            data,
            Values::Current(from, disabled),
            from.viewport.get(),
            from.scale.get(),
        )
    }

    /// Returns the disabled bits the scene shows now.
    fn disabled_values(&self) -> Vec<bool> {
        self.disableds
            .iter()
            .map(|disabled| disabled.value.get())
            .collect()
    }

    /// Creates a scene whose bound properties come from `values`.
    fn build(data: &SceneData, values: Values<'_>, viewport: Size, scale: f32) -> Scene {
        let mut scene = Scene {
            nodes: Vec::with_capacity(data.nodes.len()),
            colors: Vec::new(),
            widths: Vec::new(),
            radii: Vec::new(),
            shows: Vec::new(),
            hiddens: Vec::new(),
            disableds: Vec::new(),
            texts: Vec::new(),
            glyph_colors: Vec::new(),
            list: match values {
                Values::Signals => ListTracked::reactive(vec![1, 2, 3]),
                Values::Current(from, _) => {
                    ListTracked::static_value(from.list.value.borrow().clone())
                }
            },
            scroll: match values {
                Values::Signals => ScrollTracked::new(0.0),
                Values::Current(from, _) => ScrollTracked::new(from.scroll.value()),
            },
            inner_scroll: match values {
                Values::Signals => ScrollTracked::new(0.0),
                Values::Current(from, _) => ScrollTracked::new(from.inner_scroll.value()),
            },
            viewport: Cell::new(viewport),
            scale: Cell::new(scale),
        };
        let (
            mut color,
            mut width,
            mut radius,
            mut hidden,
            mut show,
            mut text,
            mut glyph_color,
            mut disabled,
        ) = (
            0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize,
        );
        for (index, spec) in data.nodes.iter().enumerate() {
            let color = if spec.bind_background {
                let value = match values {
                    Values::Signals => {
                        Tracked::reactive(spec.background.unwrap_or(Color::TRANSPARENT))
                    }
                    Values::Current(from, _) => {
                        Tracked::static_value(from.colors[color].value.get())
                    }
                };
                color += 1;
                let get = value.get.clone();
                scene.colors.push(value);
                Some(get)
            } else {
                None
            };
            let width = if spec.bind_width {
                let value = match values {
                    Values::Signals => Tracked::reactive(spec.width),
                    Values::Current(from, _) => {
                        Tracked::static_value(from.widths[width].value.get())
                    }
                };
                width += 1;
                let get = value.get.clone();
                scene.widths.push(value);
                Some(get)
            } else {
                None
            };
            let radius = if spec.bind_radius {
                let value = match values {
                    Values::Signals => Tracked::reactive(spec.radius),
                    Values::Current(from, _) => {
                        Tracked::static_value(from.radii[radius].value.get())
                    }
                };
                radius += 1;
                let get = value.get.clone();
                scene.radii.push(value);
                Some(get)
            } else {
                None
            };
            let hidden = if spec.bind_hidden {
                let value = match values {
                    Values::Signals => Tracked::reactive(false),
                    Values::Current(from, _) => {
                        Tracked::static_value(from.hiddens[hidden].value.get())
                    }
                };
                hidden += 1;
                let get = value.get.clone();
                scene.hiddens.push(value);
                Some(get)
            } else {
                None
            };
            let show = if spec.show {
                let value = match values {
                    Values::Signals => Tracked::reactive(true),
                    Values::Current(from, _) => Tracked::static_value(from.shows[show].value.get()),
                };
                show += 1;
                let get = value.get.clone();
                scene.shows.push(value);
                Some(get)
            } else {
                None
            };
            let disabled = if spec.bind_disabled {
                let value = match values {
                    Values::Signals => Tracked::reactive(false),
                    Values::Current(_, start) => Tracked::reactive(start[disabled]),
                };
                disabled += 1;
                let get = value.get.clone();
                scene.disableds.push(value);
                Some(get)
            } else {
                None
            };
            let text = if spec.text && spec.bind_text {
                let value = match values {
                    Values::Signals => TextTracked::reactive(initial_text(index)),
                    Values::Current(from, _) => {
                        let current = from.texts[text].value.borrow().clone();
                        TextTracked::static_value(current)
                    }
                };
                text += 1;
                let get = value.get.clone();
                scene.texts.push(value);
                Some(get)
            } else {
                None
            };
            let glyph_color = if spec.text && spec.bind_glyph_color {
                let value = match values {
                    Values::Signals => Tracked::reactive(initial_glyph_color(index)),
                    Values::Current(from, _) => {
                        Tracked::static_value(from.glyph_colors[glyph_color].value.get())
                    }
                };
                glyph_color += 1;
                let get = value.get.clone();
                scene.glyph_colors.push(value);
                Some(get)
            } else {
                None
            };
            scene.nodes.push(SceneNode {
                spec: spec.clone(),
                children: data.children[index].clone(),
                color,
                width,
                radius,
                hidden,
                show,
                disabled,
                text,
                glyph_color,
            });
        }
        scene
    }
}

/// Builds the element of the node at `index`, with every child of every node
/// built the same way.
///
/// Every bound property comes from the scene: from a signal in a reactive
/// scene, and from a plain value in a static one. The builder itself is the
/// same, so a fresh mount differs from the reactive scene only in where its
/// values come from.
fn build_node(scene: &Rc<Scene>, index: usize) -> AnyElement {
    if scene.nodes[index].spec.text {
        build_text_block(scene, index).into_any()
    } else {
        build_panel(scene, index).into_any()
    }
}

/// Builds the element of the text node at `index`.
fn build_text_block(scene: &Rc<Scene>, index: usize) -> Element<TextBlock> {
    let node = &scene.nodes[index];
    let spec = &node.spec;
    // The bindings run once at mount, before anything paints, so the values
    // the node starts with are only placeholders for the case where a property
    // has no binding. A reactive scene's getter reads a signal, which no build
    // may do, so the placeholder is the value the signal starts with.
    let mut element = Element::new(TextBlock::new(
        initial_text(index),
        initial_glyph_color(index),
    ))
    .width(spec.width)
    .height(spec.height);
    if let Some(text) = &node.text {
        let text = Rc::clone(text);
        element = element.bind(move || text(), set_text);
    }
    if let Some(glyph_color) = &node.glyph_color {
        let glyph_color = Rc::clone(glyph_color);
        element = element.bind(move || glyph_color(), set_glyph_color);
    }
    append_children(scene, index, element)
}

/// Builds the element of the panel node at `index`.
fn build_panel(scene: &Rc<Scene>, index: usize) -> Element<Panel> {
    let node = &scene.nodes[index];
    let spec = &node.spec;

    let mut element = panel()
        .height(spec.height)
        .padding(px(spec.padding))
        .gap(Size {
            width: spec.gap,
            height: spec.gap,
        })
        .direction(spec.direction)
        .border(spec.border)
        .border_color(spec.border_color)
        .clip(spec.clip);
    if spec.absolute {
        element = element.absolute().inset(spec.inset);
    }
    if spec.hover_variant {
        element = if spec.hover_layout {
            let width = hover_width(index);
            let background = hover_background(index);
            element.hover(move |p| p.width(Length::px(width)).background(background))
        } else {
            let background = hover_background(index);
            element.hover(move |p| p.background(background))
        };
    }
    if spec.pressed_variant {
        let background = pressed_background(index);
        element = element.pressed(move |p| p.background(background));
    }
    if spec.focused_variant {
        let border_color = focus_border_color(index);
        element = element
            .focusable()
            .focused(move |p| p.border(1.0).border_color(border_color));
    }
    if spec.disabled_variant {
        let background = disabled_background(index);
        element = element.disabled_style(move |p| p.background(background));
    }
    if let Some(disabled) = &node.disabled {
        let disabled = Rc::clone(disabled);
        element = element.disabled(move || disabled());
    }
    element = match &node.width {
        Some(width) => {
            let width = Rc::clone(width);
            element.width(move || width())
        }
        None => element.width(spec.width),
    };
    element = match &node.radius {
        Some(radius) => {
            let radius = Rc::clone(radius);
            element.corner_radius(move || radius().into())
        }
        None => element.corner_radius(spec.radius),
    };
    element = match (&node.color, spec.background) {
        (Some(color), _) => {
            let color = Rc::clone(color);
            element.background(move || color().into())
        }
        (None, Some(color)) => element.background(color),
        (None, None) => element,
    };
    if let Some(hidden) = &node.hidden {
        let hidden = Rc::clone(hidden);
        element = element.hidden(move || hidden());
    }

    let element = append_children(scene, index, element);
    if index == 0 {
        let list = Rc::clone(&scene.list.get);
        element
            .child(for_each(move || list(), |item| *item, list_row).direction(Direction::Column))
            .child(scroll_view(&scene.scroll, &scene.inner_scroll))
    } else {
        element
    }
}

/// The scroll view the root mounts.
///
/// The view asks for [`SCROLL_VIEW_HEIGHT`] and its content is
/// [`SCROLL_ROW_COUNT`] rows of [`SCROLL_ROW_HEIGHT`]. The root's column can
/// shrink the view, so a viewport edit changes the range the offset clamps
/// to and the thumb's length. The view has a border and padding, so the clip
/// inside the border and the content box reach the comparisons, and the rows
/// paint distinct colors, so a scroll moves pixels they can tell apart.
fn scroll_view(tracked: &ScrollTracked, inner: &ScrollTracked) -> Element<ScrollView> {
    let mut content = column();
    for index in 0..SCROLL_ROW_COUNT {
        content = content.child(
            panel()
                .width(px(120.0))
                .height(px(SCROLL_ROW_HEIGHT))
                .background(Color::rgb(
                    (index * 61 + 30) as u8,
                    (index * 83 + 70) as u8,
                    (index * 47 + 120) as u8,
                )),
        );
    }
    content = content.child(inner_view(inner));
    scroll(content)
        .offset(tracked.signal.clone())
        .height(px(SCROLL_VIEW_HEIGHT))
        .border(2.0)
        .border_color(Color::rgb(40, 40, 40))
        .padding(px(3.0))
        // A badge the scroll must leave put: it hangs off the view itself,
        // outside the flow, so a scroll moves the content under it.
        .child(
            panel()
                .width(px(16.0))
                .height(px(16.0))
                .background(Color::rgb(250, 240, 10))
                .absolute()
                .inset(Edges {
                    top: Length::px(4.0),
                    left: Length::px(4.0),
                    right: Length::Auto,
                    bottom: Length::Auto,
                }),
        )
}

/// The scroll view nested inside the outer one.
///
/// Its content is taller than its viewport, so an edit can scroll it, and its
/// rows paint distinct colors, so the two views' pixels can be told apart.
fn inner_view(tracked: &ScrollTracked) -> Element<ScrollView> {
    let mut content = column();
    for index in 0..3 {
        content = content.child(
            panel()
                .width(px(60.0))
                .height(px(20.0))
                .background(Color::rgb(
                    (index * 100 + 5) as u8,
                    (index * 40 + 200) as u8,
                    (index * 90 + 60) as u8,
                )),
        );
    }
    scroll(content)
        .offset(tracked.signal.clone())
        .height(px(40.0))
}

/// The row the keyed list mounts for its key.
///
/// The row reads nothing but its key, so a fresh mount of the scene builds the
/// same rows the reactive scene has mounted. A row with an even key leaves the
/// flow at an offset its key picks, so it overlaps its neighbors, and a
/// reorder restacks it without moving any rectangle.
fn list_row(key: u64) -> Element<Panel> {
    let row = panel()
        .width(px(24.0 + (key % 4) as f32 * 8.0))
        .height(px(12.0))
        .background(Color::rgb(
            (key * 53 % 200 + 40) as u8,
            (key * 97 % 200 + 30) as u8,
            (key * 31 % 200 + 50) as u8,
        ));
    if key.is_multiple_of(2) {
        row.absolute().inset(Edges {
            top: Length::px((key % 3) as f32 * 4.0),
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::px((key % 4) as f32 * 6.0),
        })
    } else {
        row
    }
}

/// Appends the children of the node at `index` to `element`, each built the
/// same way.
fn append_children<W: Widget>(
    scene: &Rc<Scene>,
    index: usize,
    mut element: Element<W>,
) -> Element<W> {
    for &child in &scene.nodes[index].children {
        element = match &scene.nodes[child].show {
            Some(visible) => {
                let scene = Rc::clone(scene);
                let visible = Rc::clone(visible);
                element.child(show(move || visible(), move || build_node(&scene, child)))
            }
            None => element.child(build_node(scene, child)),
        };
    }
    element
}

/// One change a case applies to a built scene.
#[derive(Clone, Debug)]
enum Edit {
    /// Sets the background property at this index to a color.
    Color(usize, Color),
    /// Sets the width property at this index to a length.
    Width(usize, Length),
    /// Sets the corner radius property at this index to a radius.
    Radius(usize, f32),
    /// Flips the visibility property at this index, which removes the node it
    /// mounts or mounts it again.
    Show(usize),
    /// Flips the hidden property at this index, which lays the node out as
    /// empty or lays it out again.
    Hidden(usize),
    /// Sets the text property at this index to a string.
    Text(usize, String),
    /// Sets the glyph color property at this index to a color.
    GlyphColor(usize, Color),
    /// Moves the viewport the scene is laid out in, or changes its scale.
    Viewport(Size, f32),
    /// Flips the disabled property at this index, which takes the node out of
    /// hit testing or puts it back.
    Disabled(usize),
    /// Appends a key to the keyed list, unless the list has it already.
    ListAppend(u64),
    /// Removes the keyed list item nearest to this index.
    ListRemove(usize),
    /// Reverses the keyed list.
    ListReverse,
    /// Scrolls the scene's scroll view to this offset, in logical pixels.
    Scroll(f32),
    /// Scrolls the scroll view nested inside it to this offset.
    InnerScroll(f32),
    /// Moves the pointer to this position, in logical pixels.
    Move(Point),
    /// Presses the primary button at this position.
    DownAt(Point),
    /// Releases the primary button at this position.
    UpAt(Point),
    /// Presses and releases Tab, or Shift+Tab when the flag is set.
    Tab(bool),
}

impl Edit {
    /// Applies the edit, so the next frame sees it.
    ///
    /// Every edit but a viewport one writes a property of the scene. A
    /// viewport edit changes the window the harness lays the scene out in,
    /// which is why it needs the harness. An event goes through the same
    /// [`App::handle_event`] a window delivers events with.
    fn apply(&self, scene: &Scene, harness: &mut Harness) {
        match *self {
            Edit::Color(index, color) => scene.colors[index].write(color),
            Edit::Width(index, width) => scene.widths[index].write(width),
            Edit::Radius(index, radius) => scene.radii[index].write(radius),
            Edit::Show(index) => {
                let show = &scene.shows[index];
                show.write(!show.value.get());
            }
            Edit::Hidden(index) => {
                let hidden = &scene.hiddens[index];
                hidden.write(!hidden.value.get());
            }
            Edit::Text(index, ref text) => scene.texts[index].write(text.clone()),
            Edit::GlyphColor(index, color) => scene.glyph_colors[index].write(color),
            Edit::Viewport(viewport, scale) => {
                scene.viewport.set(viewport);
                scene.scale.set(scale);
                harness.app().set_viewport(viewport, scale);
            }
            Edit::Disabled(index) => {
                let disabled = &scene.disableds[index];
                disabled.write(!disabled.value.get());
            }
            Edit::ListAppend(key) => {
                let mut keys = scene.list.value.borrow().clone();
                if !keys.contains(&key) {
                    keys.push(key);
                    scene.list.write(keys);
                }
            }
            Edit::ListRemove(index) => {
                let mut keys = scene.list.value.borrow().clone();
                if !keys.is_empty() {
                    keys.remove(index % keys.len());
                    scene.list.write(keys);
                }
            }
            Edit::ListReverse => {
                let mut keys = scene.list.value.borrow().clone();
                keys.reverse();
                scene.list.write(keys);
            }
            Edit::Scroll(offset) => scene.scroll.write(offset),
            Edit::InnerScroll(offset) => scene.inner_scroll.write(offset),
            Edit::Move(position) => harness.move_to(position),
            Edit::DownAt(position) => harness.event(InputEvent::PointerDown {
                position,
                button: PointerButton::Primary,
            }),
            Edit::UpAt(position) => harness.event(InputEvent::PointerUp {
                position,
                button: PointerButton::Primary,
            }),
            Edit::Tab(shift) => {
                let modifiers = Modifiers {
                    shift,
                    control: false,
                    alt: false,
                    meta: false,
                };
                for pressed in [true, false] {
                    harness.event(InputEvent::Key(KeyEvent {
                        key: Key::Tab,
                        pressed,
                        repeat: false,
                        modifiers,
                        text: None,
                    }));
                }
            }
        }
    }

    /// Replays the edit on the fresh mount of `scene`, when it is an event or
    /// a disabled toggle.
    ///
    /// The mount was built from the final value of every other property, so
    /// those edits have nothing left to change there. Its interaction state
    /// comes from the events and the disabled toggles, which it receives in
    /// the same order and in the same frame groups the reactive scene
    /// received them.
    fn replay(&self, scene: &Scene, harness: &mut Harness) {
        match self {
            Edit::Move(_) | Edit::DownAt(_) | Edit::UpAt(_) | Edit::Tab(_) | Edit::Disabled(_) => {
                self.apply(scene, harness)
            }
            Edit::Color(..)
            | Edit::Width(..)
            | Edit::Radius(..)
            | Edit::Show(_)
            | Edit::Hidden(_)
            | Edit::Text(..)
            | Edit::GlyphColor(..)
            | Edit::Viewport(..)
            | Edit::ListAppend(_)
            | Edit::ListRemove(_)
            | Edit::ListReverse
            | Edit::Scroll(_)
            | Edit::InnerScroll(_) => {}
        }
    }
}

/// Which property an edit writes.
#[derive(Copy, Clone, Debug)]
enum EditKind {
    /// A background color.
    Color,
    /// A width.
    Width,
    /// A corner radius.
    Radius,
    /// A visibility flip.
    Show,
    /// A hidden flip.
    Hidden,
    /// A text change.
    Text,
    /// A glyph color change.
    GlyphColor,
    /// A viewport or scale change.
    Viewport,
    /// A disabled flip.
    Disabled,
    /// A keyed list append.
    ListAppend,
    /// A keyed list removal.
    ListRemove,
    /// A keyed list reversal.
    ListReverse,
    /// A scroll offset change.
    Scroll,
    /// A scroll offset change of the nested view.
    InnerScroll,
}

/// Any edit kind, and every scene has a target for each of them.
fn any_edit_kind() -> impl Strategy<Value = EditKind> {
    prop_oneof![
        Just(EditKind::Color),
        Just(EditKind::Width),
        Just(EditKind::Radius),
        Just(EditKind::Show),
        Just(EditKind::Hidden),
        Just(EditKind::Text),
        Just(EditKind::GlyphColor),
        Just(EditKind::Viewport),
        Just(EditKind::Disabled),
        Just(EditKind::ListAppend),
        Just(EditKind::ListRemove),
        Just(EditKind::ListReverse),
        Just(EditKind::Scroll),
        Just(EditKind::InnerScroll),
    ]
}

/// A short string over a fixed alphabet, which the bundled font covers.
fn any_string() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9 .,!?]{0,8}").expect("the pattern is valid")
}

/// One edit, targeting a property that `counts` describes.
fn any_edit(counts: SignalCounts) -> impl Strategy<Value = Edit> {
    any_edit_kind().prop_flat_map(move |kind| match kind {
        EditKind::Color => (0..counts.colors, any_color())
            .prop_map(|(index, color)| Edit::Color(index, color))
            .boxed(),
        EditKind::Width => (0..counts.widths, any_length())
            .prop_map(|(index, width)| Edit::Width(index, width))
            .boxed(),
        EditKind::Radius => (0..counts.radii, 0.0f32..=12.0)
            .prop_map(|(index, radius)| Edit::Radius(index, radius))
            .boxed(),
        EditKind::Show => (0..counts.shows).prop_map(Edit::Show).boxed(),
        EditKind::Hidden => (0..counts.hiddens).prop_map(Edit::Hidden).boxed(),
        EditKind::Text => (0..counts.texts, any_string())
            .prop_map(|(index, text)| Edit::Text(index, text))
            .boxed(),
        EditKind::GlyphColor => (0..counts.glyph_colors, any_color())
            .prop_map(|(index, color)| Edit::GlyphColor(index, color))
            .boxed(),
        EditKind::Viewport => (any_viewport(), any_scale())
            .prop_map(|(viewport, scale)| Edit::Viewport(viewport, scale))
            .boxed(),
        EditKind::Disabled => (0..counts.disableds).prop_map(Edit::Disabled).boxed(),
        EditKind::ListAppend => (0u64..8).prop_map(Edit::ListAppend).boxed(),
        EditKind::ListRemove => (0..4usize).prop_map(Edit::ListRemove).boxed(),
        EditKind::ListReverse => Just(Edit::ListReverse).boxed(),
        // The view's range is at least 0 to 64, and offsets past either end
        // check the clamp and the write-back.
        EditKind::Scroll => (-20.0f32..=200.0).prop_map(Edit::Scroll).boxed(),
        // The nested view's range is 0 to 20, so most values clamp.
        EditKind::InnerScroll => (-20.0f32..=200.0).prop_map(Edit::InnerScroll).boxed(),
    })
}

/// The edits one frame applies: one to four of them, which the frame carries
/// together.
fn any_edit_group(counts: SignalCounts) -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(any_edit(counts), 1..=MAX_EDITS_PER_FRAME)
}

/// One item of the event segment, before the walk gives the moves their
/// absolute positions.
///
/// A move is a relative step, so a sequence of moves wanders around the scene
/// and rides the node boundaries instead of jumping between unrelated points.
#[derive(Clone, Debug)]
enum RawEdit {
    /// A property edit that changes no geometry.
    Property(Edit),
    /// Moves the pointer by this offset from its current position.
    MoveBy(f32, f32),
    /// Presses the primary button where the pointer is.
    Press,
    /// Releases the primary button where the pointer is.
    Release,
    /// Presses and releases Tab, or Shift+Tab when the flag is set.
    Tab(bool),
}

/// One item of the event segment.
///
/// Events are the most common items, and moves the most common events, so the
/// pointer keeps meeting the variants. The remaining items are the edits that
/// change no geometry, which a frame can carry next to events: the ones that
/// only repaint, and disabled toggles, which can land on a node that is
/// hovered, pressed or focused.
fn any_raw_event(counts: SignalCounts) -> impl Strategy<Value = RawEdit> {
    prop_oneof![
        3 => (-48.0f32..=48.0, -48.0f32..=48.0)
            .prop_map(|(dx, dy)| RawEdit::MoveBy(dx, dy)),
        1 => Just(RawEdit::Press),
        1 => Just(RawEdit::Release),
        1 => any::<bool>().prop_map(RawEdit::Tab),
        1 => (0..counts.disableds).prop_map(|index| RawEdit::Property(Edit::Disabled(index))),
        2 => (0..counts.colors, any_color())
            .prop_map(|(index, color)| RawEdit::Property(Edit::Color(index, color))),
        1 => (0..counts.radii, 0.0f32..=12.0)
            .prop_map(|(index, radius)| RawEdit::Property(Edit::Radius(index, radius))),
        1 => (0..counts.texts, any_string())
            .prop_map(|(index, text)| RawEdit::Property(Edit::Text(index, text))),
        1 => (0..counts.glyph_colors, any_color())
            .prop_map(|(index, color)| RawEdit::Property(Edit::GlyphColor(index, color))),
    ]
}

/// One frame of the event segment.
fn any_raw_group(counts: SignalCounts) -> impl Strategy<Value = Vec<RawEdit>> {
    prop::collection::vec(any_raw_event(counts), 1..=MAX_EDITS_PER_FRAME)
}

/// Turns relative moves into absolute positions by following them in order.
///
/// The walk starts near the middle of the default viewport and strays a little
/// past the edges, so a move can also land where no node is. Press and release
/// happen where the pointer stands, which is where a real pointer is.
fn absolute_edits(groups: Vec<Vec<RawEdit>>) -> Vec<Vec<Edit>> {
    let mut at = Point {
        x: VIEWPORT.width / 2.0,
        y: VIEWPORT.height / 2.0,
    };
    groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .map(|raw| match raw {
                    RawEdit::Property(edit) => edit,
                    RawEdit::MoveBy(dx, dy) => {
                        at.x = (at.x + dx).clamp(-12.0, VIEWPORT.width + 12.0);
                        at.y = (at.y + dy).clamp(-12.0, VIEWPORT.height + 12.0);
                        Edit::Move(at)
                    }
                    RawEdit::Press => Edit::DownAt(at),
                    RawEdit::Release => Edit::UpAt(at),
                    RawEdit::Tab(shift) => Edit::Tab(shift),
                })
                .collect()
        })
        .collect()
}

/// The frames of the event segment: one to four, each carrying events,
/// disabled toggles and the edits that only repaint.
fn any_event_frames(counts: SignalCounts) -> impl Strategy<Value = Vec<Vec<Edit>>> {
    prop::collection::vec(any_raw_group(counts), 1..=MAX_EVENT_FRAMES).prop_map(absolute_edits)
}

/// A generated case: a scene, the edits of its geometry segment, and the edits
/// and events of its event segment, one group per frame.
fn any_case() -> impl Strategy<Value = (SceneData, Vec<Vec<Edit>>, Vec<Vec<Edit>>)> {
    prop::collection::vec(any_spec(), 2..=40)
        .prop_map(SceneData::new)
        .prop_flat_map(|data| {
            let counts = data.signal_counts();
            (
                Just(data),
                prop::collection::vec(any_edit_group(counts), 1..=MAX_GEOMETRY_FRAMES),
                any_event_frames(counts),
            )
        })
}

/// Panics when the app still holds work that only a frame can finish.
///
/// A frame runs the layout pass, the hover recompute that follows it and the
/// second layout pass the recompute can ask for, then paints everything that
/// was marked. When the function reports pending work after a frame, the frame
/// left the tree half settled and the screen would only catch up on the next
/// one, which is the failure a missing second layout pass causes.
fn assert_settled(harness: &mut Harness) {
    assert!(
        !harness.app().needs_frame(),
        "the frame left work pending, so the screen would not be settled \
         until another frame ran"
    );
}

/// Runs the frames that writes made during a frame ask for, and checks each
/// one like the frame before it.
///
/// The scroll view measures its heights when it paints, so a geometry change
/// reaches its clamp and its thumb one frame later. The measurement wakes the
/// app, which is how a window learns to run that frame, and the harness
/// records the wake. The frames run until no write wakes the app, which is
/// the state a window shows once it stops drawing.
fn settle(harness: &mut Harness) {
    for _ in 0..4 {
        if !harness.woken() {
            return;
        }
        harness.frame();
        assert_settled(harness);
        harness.assert_matches_full_repaint();
    }
    panic!("the scene still woke the app after four frames, so it never settles");
}

/// Requires the surface of `harness` to match a fresh mount of the final state
/// of `data`, byte for byte.
///
/// The fresh mount reads no signal but its disabled bits: every other bound
/// property is the value the reactive scene holds right now, which is what
/// [`Tracked`] and [`TextTracked`] keep. The disabled bits start at
/// `disabled`, the values they held before the first event. `applied_events`
/// are the event groups the reactive scene has run so far, and the mount
/// receives their events and disabled toggles in the same order, one frame per
/// group, after a first frame that lays the final values out. A difference
/// means the incremental frames left the screen stale, which the full-repaint
/// comparison cannot see because it repaints the same stale list.
///
/// The mount runs the same core through the same events, so a restyle that
/// never happens is missed in both scenes alike. What the comparison catches
/// is a restyle or a geometry edit whose invalidation the incremental frames
/// lost.
fn assert_matches_fresh_mount(
    harness: &Harness,
    scene: &Scene,
    data: &SceneData,
    disabled: &[bool],
    applied_events: &[Vec<Edit>],
) {
    let fresh = Rc::new(Scene::fresh(data, scene, disabled));
    let built = Rc::clone(&fresh);
    let viewport = scene.viewport.get();
    let scale = scene.scale.get();
    let mut mount = Harness::new(viewport.width, viewport.height, scale, move || {
        build_node(&built, 0)
    });
    mount.frame();
    assert_settled(&mut mount);
    settle(&mut mount);
    for edits in applied_events {
        for edit in edits {
            edit.replay(&fresh, &mut mount);
        }
        mount.frame();
        assert_settled(&mut mount);
        settle(&mut mount);
    }
    harness.assert_matches(&mount);
}

/// Builds `data` at `scale`, applies the edits of each frame, and checks the
/// surface after every frame.
///
/// The geometry segment runs first. All of its edits land before the first
/// event, and none of them moves the pointer, so they reach the fresh mount
/// through the final values alone. The event segment runs second, and the fresh mount
/// replays its events and disabled toggles on the final tree, which meets the
/// same nodes the reactive scene's events met.
///
/// The first frame repaints the whole surface, so the comparison only starts
/// to depend on the damage after the first edit.
fn check_scene(scale: f32, data: &SceneData, geometry: &[Vec<Edit>], events: &[Vec<Edit>]) {
    TextSystem::bundled_only().install();
    let scene = Rc::new(Scene::reactive(data, scale));
    let built = Rc::clone(&scene);
    let mut harness = Harness::new(VIEWPORT.width, VIEWPORT.height, scale, move || {
        build_node(&built, 0)
    });

    harness.frame();
    assert_settled(&mut harness);
    harness.assert_matches_full_repaint();
    settle(&mut harness);
    assert_matches_fresh_mount(&harness, &scene, data, &scene.disabled_values(), &[]);

    for edits in geometry {
        for edit in edits {
            edit.apply(&scene, &mut harness);
        }
        harness.frame();
        assert_settled(&mut harness);
        harness.assert_matches_full_repaint();
        settle(&mut harness);
        assert_matches_fresh_mount(&harness, &scene, data, &scene.disabled_values(), &[]);
    }

    let disabled = scene.disabled_values();
    for (index, edits) in events.iter().enumerate() {
        for edit in edits {
            edit.apply(&scene, &mut harness);
        }
        harness.frame();
        assert_settled(&mut harness);
        harness.assert_matches_full_repaint();
        settle(&mut harness);
        assert_matches_fresh_mount(&harness, &scene, data, &disabled, &events[..=index]);
    }
}

/// Declares the damage equivalence property test once per scale factor.
macro_rules! damage_equivalence_tests {
    ($($name:ident at $scale:expr;)*) => {
        proptest! {
            #![proptest_config(config())]
            $(
                #[test]
                fn $name((data, geometry, events) in any_case()) {
                    check_scene($scale, &data, &geometry, &events);
                }
            )*
        }
    };
}

damage_equivalence_tests! {
    damage_matches_full_repaint_at_scale_one at 1.0;
    damage_matches_full_repaint_at_scale_one_and_a_quarter at 1.25;
    damage_matches_full_repaint_at_scale_one_and_a_half at 1.5;
    damage_matches_full_repaint_at_scale_two at 2.0;
}
