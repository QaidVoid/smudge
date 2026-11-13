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
use smudge_core::image::Image;
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*, signal_local};
use smudge_core::style::{Direction, Edges, Length, px};
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_test::Harness;
use smudge_text::{TextLayout, TextStyle, TextSystem};
use smudge_widgets::{ImageView, ScrollExt, ScrollView, column, image, scroll};

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

/// The most nodes a generated scene starts with, before any appended one.
const MAX_NODES: usize = 40;

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
    /// Whether the node paints an image instead of a panel or text.
    image: bool,
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
                image: false,
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
    /// directly, and it always paints a panel, because the keyed list and the
    /// scroll views mount only under one.
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
        nodes[0].text = false;
        nodes[0].image = false;
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
                    image: false,
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

    /// Marks the nodes whose `flags` entry is set as image nodes, and makes
    /// sure at least one node paints an image.
    ///
    /// The flags come from their own strategy, drawn after the rest of the
    /// case, so adding images left the seeds of older failures meaning what
    /// they did. An image node reads every property a panel reads, so the
    /// flags change no property count but the images'. The root stays a
    /// panel, and a text node stays text. When no other node is an image, the
    /// first child that paints no text becomes one; the variant target
    /// guarantees there is such a child.
    fn with_images(mut self, flags: &[bool]) -> SceneData {
        for (node, &flag) in self.nodes.iter_mut().zip(flags).skip(1) {
            node.image = flag;
        }
        if !self.nodes[1..].iter().any(|node| node.image && !node.text) {
            let target = self.nodes[1..]
                .iter()
                .position(|node| !node.text)
                .expect("the variant target paints no text")
                + 1;
            self.nodes[target].image = true;
        }
        self
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
            images: self
                .nodes
                .iter()
                .filter(|node| node.image && !node.text)
                .count(),
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
    images: usize,
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

/// A `seed`ed three by three gradient, so an image edit has a small set of
/// distinct images to swap between. Seeds run 0 to 7; the eighth seed an
/// edit can draw clears the source instead.
fn gradient(seed: u8) -> Image {
    let seed = u32::from(seed);
    let mut pixels = Vec::with_capacity(3 * 3 * 4);
    for y in 0..3u32 {
        for x in 0..3u32 {
            let alpha = if (x + y) % 2 == 0 { 255 } else { 160 };
            pixels.push(((seed + x * 40) % 256) as u8);
            pixels.push(((seed * 7 + y * 40) % 256) as u8);
            pixels.push(((seed * 13 + 60) % 256) as u8);
            pixels.push(alpha);
        }
    }
    Image::from_straight_rgba(3, 3, pixels).expect("the gradient fills the buffer")
}

/// Any seed an image edit draws: 0 to 7 pick a gradient, 8 clears the
/// source, so the empty-source path of the widget reaches the oracle.
fn any_image_seed() -> impl Strategy<Value = u8> {
    0u8..=8
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

/// One image property of a scene: the pixels it shows, and how a scene reads
/// them.
///
/// It mirrors [`TextTracked`] for values that do not fit in a [`Cell`]: the
/// current image is kept in a refcell, the reactive scene reads a signal, and
/// a static scene reads the refcell alone.
struct ImageTracked {
    /// Reads the image the scene shows now.
    get: Getter<Option<Image>>,
    /// Writes the reactive scene's signal. A static scene has no signal, so
    /// its writer does nothing.
    set: Box<dyn Fn(Option<Image>)>,
    /// The image the scene shows now.
    value: Rc<RefCell<Option<Image>>>,
}

impl ImageTracked {
    /// Creates an image that edits change through a signal.
    fn reactive(value: Option<Image>) -> ImageTracked {
        let initial = value.clone();
        let (read, write) = signal_local(value);
        ImageTracked {
            get: Rc::new(move || read.get()),
            set: Box::new(move |value| write.set(value)),
            value: Rc::new(RefCell::new(initial)),
        }
    }

    /// Creates an image that holds one value for the scene's whole life.
    fn static_value(value: Option<Image>) -> ImageTracked {
        let value = Rc::new(RefCell::new(value));
        ImageTracked {
            get: Rc::new({
                let value = Rc::clone(&value);
                move || value.borrow().clone()
            }),
            set: Box::new(|_| ()),
            value,
        }
    }

    /// Writes a new image through the refcell and the signal, so the reactive
    /// scene and a fresh mount of it show the same thing.
    fn write(&self, value: Option<Image>) {
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

/// The key of a text node's text binding.
struct TextKey;

/// The key of a text node's glyph color binding.
struct GlyphColorKey;

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
    /// The image the node paints, when a property supplies it.
    image: Option<Getter<Option<Image>>>,
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
    images: Vec<ImageTracked>,
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
            images: Vec::new(),
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
            mut image,
        ) = (
            0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize,
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
            let image = if spec.image && !spec.text {
                let value = match values {
                    Values::Signals => ImageTracked::reactive(Some(gradient(index as u8))),
                    Values::Current(from, _) => {
                        let current = from.images[image].value.borrow().clone();
                        ImageTracked::static_value(current)
                    }
                };
                image += 1;
                let get = value.get.clone();
                scene.images.push(value);
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
                image,
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
    if scene.nodes[index].spec.image && !scene.nodes[index].spec.text {
        build_image_node(scene, index).into_any()
    } else if scene.nodes[index].spec.text {
        build_text_block(scene, index).into_any()
    } else {
        build_panel(scene, index).into_any()
    }
}

/// Builds the image node at `index`.
///
/// It reads every property a panel reads and carries the same box, so the
/// layout, style and visibility edits reach it the way they reach a panel,
/// and its padding and border inset the image from its background. A swap
/// between equal-size sources repaints the image's rectangle and nothing
/// else; a clear or a size change relays the node out, which is the layout
/// path the seed that clears the source reaches.
fn build_image_node(scene: &Rc<Scene>, index: usize) -> Element<ImageView> {
    let element = match &scene.nodes[index].image {
        Some(source) => {
            let source = Rc::clone(source);
            image(move || source())
        }
        None => image(Option::<Image>::None),
    };
    append_children(scene, index, style_node(scene, index, element))
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
        element = element.bind(TextKey, move || text(), set_text);
    }
    if let Some(glyph_color) = &node.glyph_color {
        let glyph_color = Rc::clone(glyph_color);
        element = element.bind(GlyphColorKey, move || glyph_color(), set_glyph_color);
    }
    append_children(scene, index, element)
}

/// Builds the element of the panel node at `index`.
fn build_panel(scene: &Rc<Scene>, index: usize) -> Element<Panel> {
    let element = append_children(scene, index, style_node(scene, index, panel()));
    if index == 0 {
        let list = Rc::clone(&scene.list.get);
        element
            .child(for_each(move || list(), |item| *item, list_row).direction(Direction::Column))
            .child(scroll_view(&scene.scroll, &scene.inner_scroll))
    } else {
        element
    }
}

/// Applies the box, the style variants and the bound properties of the node
/// at `index` to `element`, which a panel and an image node share.
fn style_node<W: Widget>(scene: &Rc<Scene>, index: usize, element: Element<W>) -> Element<W> {
    let node = &scene.nodes[index];
    let spec = &node.spec;

    let mut element = element
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
    element
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
    /// Replaces the image source at this index, or clears it.
    Image(usize, Option<Image>),
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
            Edit::Image(index, ref image) => scene.images[index].write(image.clone()),
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
            | Edit::Image(..)
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
///
/// The image flags and the image edits are drawn after everything else, so
/// the seeds the regression file kept from before images existed still
/// generate the scenes and edits they recorded, now with images added.
fn any_case() -> impl Strategy<Value = (SceneData, Vec<Vec<Edit>>, Vec<Vec<Edit>>)> {
    (any_base_case(), any_image_plan()).prop_map(
        |((data, mut geometry, events), (flags, image_edits))| {
            let data = data.with_images(&flags);
            let images = data.signal_counts().images;
            for (group, edits) in geometry.iter_mut().zip(image_edits) {
                group.extend(edits.into_iter().map(|(index, seed)| {
                    Edit::Image(index.index(images), (seed < 8).then(|| gradient(seed)))
                }));
            }
            (data, geometry, events)
        },
    )
}

/// A case without images: a scene, its geometry edits and its event frames.
fn any_base_case() -> impl Strategy<Value = (SceneData, Vec<Vec<Edit>>, Vec<Vec<Edit>>)> {
    prop::collection::vec(any_spec(), 2..=MAX_NODES)
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

/// The image part of a case: a flag for every node a scene can hold, and
/// for every geometry frame up to two image edits, each an image property
/// picked by position and a seed that [`any_image_seed`] describes.
fn any_image_plan() -> impl Strategy<Value = (Vec<bool>, Vec<Vec<(prop::sample::Index, u8)>>)> {
    (
        prop::collection::vec(any::<bool>(), MAX_NODES + 1),
        prop::collection::vec(
            prop::collection::vec((any::<prop::sample::Index>(), any_image_seed()), 0..=2),
            MAX_GEOMETRY_FRAMES,
        ),
    )
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

/// A scene, the edits of its geometry segment and those of its event segment.
type Case = (SceneData, Vec<Vec<Edit>>, Vec<Vec<Edit>>);

/// The cases behind two seeds of the regression file that were recorded
/// while each node drew its own image flag, before the image plan moved to a
/// strategy of its own. The seeds no longer generate these cases, so they
/// are kept as data: for each seed, the case it generated and the smaller
/// case proptest shrank it to.
#[rustfmt::skip]
fn first_image_strategy_cases() -> Vec<Case> {
    vec![
        (
            SceneData {
                nodes: vec![
                    NodeSpec { children: 2, width: Length::Auto, height: Length::Px(35.448734), padding: 6.172589, gap: 5.391601, direction: Direction::Column, background: None, border: 1.126722, border_color: Color { r: 54, g: 166, b: 20, a: 108 }, radius: 8.725183, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Px(104.56829) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Px(37.86434), padding: 5.589048, gap: 2.9332893, direction: Direction::Row, background: None, border: 0.8256354, border_color: Color { r: 225, g: 84, b: 172, a: 210 }, radius: 9.622329, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(45.087433), bottom: Length::Auto, left: Length::Px(48.982925) }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 3, width: Length::Px(115.8967), height: Length::Px(54.07467), padding: 3.6107488, gap: 6.103102, direction: Direction::Column, background: None, border: 0.28458706, border_color: Color { r: 87, g: 229, b: 121, a: 230 }, radius: 6.0041018, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(12.9605055), bottom: Length::Px(5.33184), left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Px(39.252712), height: Length::Px(74.42163), padding: 5.7661915, gap: 3.9479623, direction: Direction::Column, background: Some(Color { r: 69, g: 6, b: 14, a: 164 }), border: 2.286792, border_color: Color { r: 88, g: 104, b: 132, a: 147 }, radius: 9.514615, clip: false, absolute: false, inset: Edges { top: Length::Px(0.00086383405), right: Length::Auto, bottom: Length::Px(89.36668), left: Length::Px(10.434255) }, bind_width: false, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: false },
                    NodeSpec { children: 2, width: Length::Px(51.430134), height: Length::Px(90.14198), padding: 7.5408254, gap: 0.9391404, direction: Direction::Row, background: None, border: 0.18031308, border_color: Color { r: 4, g: 51, b: 50, a: 30 }, radius: 10.526427, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(43.492664), bottom: Length::Px(62.638706), left: Length::Px(64.958626) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 0, width: Length::Px(85.1741), height: Length::Px(40.650085), padding: 1.9025575, gap: 7.3749366, direction: Direction::Column, background: None, border: 1.5459536, border_color: Color { r: 159, g: 142, b: 179, a: 165 }, radius: 10.976462, clip: false, absolute: true, inset: Edges { top: Length::Px(4.9318104), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: false },
                    NodeSpec { children: 3, width: Length::Px(77.68155), height: Length::Px(44.240734), padding: 5.6354876, gap: 3.3513935, direction: Direction::Column, background: Some(Color { r: 200, g: 91, b: 44, a: 176 }), border: 2.427538, border_color: Color { r: 239, g: 220, b: 60, a: 228 }, radius: 6.3040276, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Px(82.61925) }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 3.077374, gap: 6.266622, direction: Direction::Row, background: Some(Color { r: 4, g: 138, b: 172, a: 24 }), border: 1.1509877, border_color: Color { r: 58, g: 115, b: 119, a: 112 }, radius: 10.552148, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(46.368694), bottom: Length::Px(100.89143), left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 1.782927, gap: 3.0044503, direction: Direction::Row, background: Some(Color { r: 178, g: 159, b: 85, a: 182 }), border: 1.240352, border_color: Color { r: 33, g: 88, b: 92, a: 229 }, radius: 4.072737, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(60.798832), left: Length::Px(64.04918) }, bind_width: false, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 2, width: Length::Px(91.88855), height: Length::Px(111.20018), padding: 5.627167, gap: 2.159307, direction: Direction::Column, background: Some(Color { r: 120, g: 240, b: 10, a: 230 }), border: 0.78831446, border_color: Color { r: 253, g: 106, b: 155, a: 81 }, radius: 9.028052, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(69.104706), left: Length::Px(6.9023037) }, bind_width: false, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Px(2.4180765), padding: 5.1052203, gap: 7.902674, direction: Direction::Column, background: None, border: 0.9611314, border_color: Color { r: 42, g: 218, b: 189, a: 125 }, radius: 11.128218, clip: false, absolute: true, inset: Edges { top: Length::Px(80.763504), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 2, width: Length::Auto, height: Length::Auto, padding: 3.2565627, gap: 4.01096, direction: Direction::Row, background: None, border: 1.5070136, border_color: Color { r: 19, g: 254, b: 106, a: 87 }, radius: 4.183633, clip: true, absolute: true, inset: Edges { top: Length::Px(71.26463), right: Length::Px(114.57188), bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: true, show: false, text: true, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 2, width: Length::Auto, height: Length::Px(82.983086), padding: 6.2785425, gap: 6.621998, direction: Direction::Row, background: Some(Color { r: 227, g: 194, b: 223, a: 97 }), border: 1.424069, border_color: Color { r: 108, g: 242, b: 203, a: 135 }, radius: 10.31094, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(70.08039), bottom: Length::Px(101.21103), left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: true, show: false, text: true, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Auto, height: Length::Px(24.628963), padding: 6.495573, gap: 4.404489, direction: Direction::Column, background: Some(Color { r: 105, g: 26, b: 89, a: 178 }), border: 0.5616599, border_color: Color { r: 75, g: 34, b: 106, a: 120 }, radius: 11.04623, clip: false, absolute: true, inset: Edges { top: Length::Px(59.418243), right: Length::Px(104.557686), bottom: Length::Px(114.539894), left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: true, show: false, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: true, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Px(0.8950497), padding: 7.00368, gap: 2.1010723, direction: Direction::Row, background: Some(Color { r: 64, g: 29, b: 16, a: 242 }), border: 0.31010342, border_color: Color { r: 204, g: 52, b: 98, a: 143 }, radius: 7.0718164, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(107.73754), left: Length::Px(105.371025) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 0, width: Length::Px(68.044846), height: Length::Px(89.71813), padding: 2.0475514, gap: 1.0570676, direction: Direction::Row, background: None, border: 2.0111852, border_color: Color { r: 26, g: 209, b: 141, a: 168 }, radius: 7.67408, clip: true, absolute: false, inset: Edges { top: Length::Px(8.945305), right: Length::Auto, bottom: Length::Px(95.4137), left: Length::Auto }, bind_width: false, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 0, width: Length::Px(103.53603), height: Length::Px(67.49067), padding: 5.4699945, gap: 1.642934, direction: Direction::Row, background: Some(Color { r: 159, g: 11, b: 67, a: 248 }), border: 2.343492, border_color: Color { r: 180, g: 210, b: 170, a: 121 }, radius: 3.2543333, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: true, image: false },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 2.5661473, gap: 2.5665705, direction: Direction::Row, background: Some(Color { r: 108, g: 39, b: 182, a: 59 }), border: 1.528279, border_color: Color { r: 61, g: 241, b: 21, a: 250 }, radius: 4.243484, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(35.0535), bottom: Length::Px(76.45695), left: Length::Px(29.406702) }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true },
                ],
                children: vec![vec![1, 2], vec![3, 4, 5], vec![11, 12, 13], vec![6, 7, 8], vec![9, 10], vec![], vec![], vec![], vec![], vec![], vec![], vec![14, 15], vec![16, 17], vec![], vec![], vec![], vec![], vec![]],
            },
            vec![
                vec![Edit::Disabled(1), Edit::InnerScroll(151.73155)],
                vec![Edit::Image(1, None), Edit::ListReverse, Edit::ListRemove(0), Edit::InnerScroll(142.65848)],
                vec![Edit::Viewport(Size { width: 168.32367, height: 196.65868 }, 1.0), Edit::Image(1, Some(gradient(3)))],
                vec![Edit::ListAppend(7), Edit::Show(9)],
                vec![Edit::Viewport(Size { width: 192.72708, height: 325.11703 }, 2.0), Edit::Color(4, Color { r: 80, g: 243, b: 22, a: 83 }), Edit::Viewport(Size { width: 344.75284, height: 335.385 }, 1.25), Edit::Radius(3, 9.072614)],
            ],
            vec![
                vec![Edit::Move(Point { x: 194.39325, y: 77.772156 }), Edit::UpAt(Point { x: 194.39325, y: 77.772156 }), Edit::GlyphColor(0, Color { r: 198, g: 76, b: 59, a: 182 }), Edit::Radius(8, 0.8350722)],
                vec![Edit::Move(Point { x: 219.75507, y: 91.74988 }), Edit::UpAt(Point { x: 219.75507, y: 91.74988 }), Edit::GlyphColor(0, Color { r: 2, g: 38, b: 47, a: 135 })],
            ],
        ),
        (
            SceneData {
                nodes: vec![
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Px(37.86434), padding: 5.589048, gap: 2.9332893, direction: Direction::Row, background: None, border: 0.8256354, border_color: Color { r: 225, g: 84, b: 172, a: 210 }, radius: 9.622329, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(45.087433), bottom: Length::Auto, left: Length::Px(48.982925) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 3, width: Length::Px(115.8967), height: Length::Px(54.07467), padding: 3.6107488, gap: 6.103102, direction: Direction::Column, background: None, border: 0.28458706, border_color: Color { r: 87, g: 229, b: 121, a: 230 }, radius: 6.0041018, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(12.9605055), bottom: Length::Px(5.33184), left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Px(39.252712), height: Length::Px(74.42163), padding: 5.7661915, gap: 3.9479623, direction: Direction::Column, background: Some(Color { r: 69, g: 6, b: 14, a: 164 }), border: 2.286792, border_color: Color { r: 88, g: 104, b: 132, a: 147 }, radius: 9.514615, clip: false, absolute: false, inset: Edges { top: Length::Px(0.00086383405), right: Length::Auto, bottom: Length::Px(89.36668), left: Length::Px(10.434255) }, bind_width: false, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: false },
                    NodeSpec { children: 2, width: Length::Px(51.430134), height: Length::Px(90.14198), padding: 7.5408254, gap: 0.9391404, direction: Direction::Row, background: None, border: 0.18031308, border_color: Color { r: 4, g: 51, b: 50, a: 30 }, radius: 10.526427, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(43.492664), bottom: Length::Px(62.638706), left: Length::Px(64.958626) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 0, width: Length::Px(85.1741), height: Length::Px(40.650085), padding: 1.9025575, gap: 7.3749366, direction: Direction::Column, background: None, border: 1.5459536, border_color: Color { r: 159, g: 142, b: 179, a: 165 }, radius: 10.976462, clip: false, absolute: true, inset: Edges { top: Length::Px(4.9318104), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: false },
                    NodeSpec { children: 3, width: Length::Px(77.68155), height: Length::Px(44.240734), padding: 5.6354876, gap: 3.3513935, direction: Direction::Column, background: Some(Color { r: 200, g: 91, b: 44, a: 176 }), border: 2.427538, border_color: Color { r: 239, g: 220, b: 60, a: 228 }, radius: 6.3040276, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Px(82.61925) }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 3.077374, gap: 6.266622, direction: Direction::Row, background: Some(Color { r: 4, g: 138, b: 172, a: 24 }), border: 1.1509877, border_color: Color { r: 58, g: 115, b: 119, a: 112 }, radius: 10.552148, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(46.368694), bottom: Length::Px(100.89143), left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 1.782927, gap: 3.0044503, direction: Direction::Row, background: Some(Color { r: 178, g: 159, b: 85, a: 182 }), border: 1.240352, border_color: Color { r: 33, g: 88, b: 92, a: 229 }, radius: 4.072737, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(60.798832), left: Length::Px(64.04918) }, bind_width: false, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 2, width: Length::Px(91.88855), height: Length::Px(111.20018), padding: 5.627167, gap: 2.159307, direction: Direction::Column, background: Some(Color { r: 120, g: 240, b: 10, a: 230 }), border: 0.78831446, border_color: Color { r: 253, g: 106, b: 155, a: 81 }, radius: 9.028052, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(69.104706), left: Length::Px(6.9023037) }, bind_width: false, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Px(2.4180765), padding: 5.1052203, gap: 7.902674, direction: Direction::Column, background: None, border: 0.9611314, border_color: Color { r: 42, g: 218, b: 189, a: 125 }, radius: 11.128218, clip: false, absolute: true, inset: Edges { top: Length::Px(80.763504), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 2, width: Length::Auto, height: Length::Auto, padding: 3.2565627, gap: 4.01096, direction: Direction::Row, background: None, border: 1.5070136, border_color: Color { r: 19, g: 254, b: 106, a: 87 }, radius: 4.183633, clip: true, absolute: true, inset: Edges { top: Length::Px(71.26463), right: Length::Px(114.57188), bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: true, show: false, text: true, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 2, width: Length::Auto, height: Length::Px(82.983086), padding: 6.2785425, gap: 6.621998, direction: Direction::Row, background: Some(Color { r: 227, g: 194, b: 223, a: 97 }), border: 1.424069, border_color: Color { r: 108, g: 242, b: 203, a: 135 }, radius: 10.31094, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(70.08039), bottom: Length::Px(101.21103), left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: true, show: false, text: true, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Auto, height: Length::Px(24.628963), padding: 6.495573, gap: 4.404489, direction: Direction::Column, background: Some(Color { r: 105, g: 26, b: 89, a: 178 }), border: 0.5616599, border_color: Color { r: 75, g: 34, b: 106, a: 120 }, radius: 11.04623, clip: false, absolute: true, inset: Edges { top: Length::Px(59.418243), right: Length::Px(104.557686), bottom: Length::Px(114.539894), left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: true, show: false, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: true, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Px(0.8950497), padding: 7.00368, gap: 2.1010723, direction: Direction::Row, background: Some(Color { r: 64, g: 29, b: 16, a: 242 }), border: 0.31010342, border_color: Color { r: 204, g: 52, b: 98, a: 143 }, radius: 7.0718164, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(107.73754), left: Length::Px(105.371025) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 0, width: Length::Px(68.044846), height: Length::Px(89.71813), padding: 2.0475514, gap: 1.0570676, direction: Direction::Row, background: None, border: 2.0111852, border_color: Color { r: 26, g: 209, b: 141, a: 168 }, radius: 7.67408, clip: true, absolute: false, inset: Edges { top: Length::Px(8.945305), right: Length::Auto, bottom: Length::Px(95.4137), left: Length::Auto }, bind_width: false, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 0, width: Length::Px(103.53603), height: Length::Px(67.49067), padding: 5.4699945, gap: 1.642934, direction: Direction::Row, background: Some(Color { r: 159, g: 11, b: 67, a: 248 }), border: 2.343492, border_color: Color { r: 180, g: 210, b: 170, a: 121 }, radius: 3.2543333, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: true, image: false },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 2.5661473, gap: 2.5665705, direction: Direction::Row, background: Some(Color { r: 108, g: 39, b: 182, a: 59 }), border: 1.528279, border_color: Color { r: 61, g: 241, b: 21, a: 250 }, radius: 4.243484, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(35.0535), bottom: Length::Px(76.45695), left: Length::Px(29.406702) }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 4.020834, gap: 5.4302993, direction: Direction::Column, background: None, border: 0.38868198, border_color: Color { r: 189, g: 118, b: 174, a: 149 }, radius: 1.5905874, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(57.657528), bottom: Length::Px(18.08034), left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: true, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 2, width: Length::Px(1.8405118), height: Length::Auto, padding: 6.173014, gap: 6.507891, direction: Direction::Column, background: None, border: 1.5204725, border_color: Color { r: 66, g: 64, b: 70, a: 32 }, radius: 2.9650028, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(11.03873), bottom: Length::Px(63.808884), left: Length::Px(53.966545) }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: true, show: true, text: false, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: true, image: true },
                    NodeSpec { children: 2, width: Length::Auto, height: Length::Px(53.82057), padding: 0.7800085, gap: 0.30162942, direction: Direction::Column, background: Some(Color { r: 164, g: 56, b: 90, a: 250 }), border: 0.42081338, border_color: Color { r: 41, g: 67, b: 191, a: 217 }, radius: 8.5350895, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Px(31.163967) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: true, show: false, text: true, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 2, width: Length::Px(87.142746), height: Length::Px(7.4322786), padding: 2.839135, gap: 0.11796351, direction: Direction::Column, background: Some(Color { r: 204, g: 100, b: 155, a: 9 }), border: 0.78058976, border_color: Color { r: 47, g: 123, b: 252, a: 196 }, radius: 10.247074, clip: false, absolute: true, inset: Edges { top: Length::Px(12.57998), right: Length::Px(114.65555), bottom: Length::Auto, left: Length::Px(28.375944) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: true, show: true, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Px(20.592388), height: Length::Auto, padding: 7.2603817, gap: 0.52885234, direction: Direction::Column, background: Some(Color { r: 139, g: 63, b: 19, a: 231 }), border: 1.1711266, border_color: Color { r: 223, g: 86, b: 190, a: 92 }, radius: 0.5918814, clip: false, absolute: true, inset: Edges { top: Length::Px(3.7580633), right: Length::Px(55.720173), bottom: Length::Px(84.57943), left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: true, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: true, image: false },
                    NodeSpec { children: 1, width: Length::Px(2.9690902), height: Length::Auto, padding: 5.8474426, gap: 7.738557, direction: Direction::Row, background: Some(Color { r: 20, g: 147, b: 179, a: 101 }), border: 2.7827864, border_color: Color { r: 189, g: 18, b: 58, a: 51 }, radius: 7.6529818, clip: false, absolute: true, inset: Edges { top: Length::Px(105.01049), right: Length::Auto, bottom: Length::Auto, left: Length::Px(41.551632) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 6.232303, gap: 2.9698226, direction: Direction::Row, background: None, border: 2.6614482, border_color: Color { r: 111, g: 56, b: 79, a: 150 }, radius: 7.2174172, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: false, image: false },
                ],
                children: vec![vec![1, 2, 3], vec![4, 5, 6], vec![10, 11, 12], vec![17, 18], vec![], vec![7, 8, 9], vec![], vec![], vec![], vec![], vec![13, 14], vec![15, 16], vec![], vec![], vec![], vec![], vec![], vec![19, 20, 21], vec![22, 23], vec![], vec![], vec![], vec![], vec![]],
            },
            vec![
                vec![Edit::Viewport(Size { width: 160.0, height: 120.0 }, 1.0)],
            ],
            vec![
                vec![Edit::Disabled(0)],
            ],
        ),
        (
            SceneData {
                nodes: vec![
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 3.7645786, gap: 2.155126, direction: Direction::Column, background: None, border: 0.968374, border_color: Color { r: 2, g: 222, b: 18, a: 226 }, radius: 1.5801493, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(17.584173), left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: false, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 3, width: Length::Px(14.072466), height: Length::Auto, padding: 3.789862, gap: 2.033609, direction: Direction::Row, background: Some(Color { r: 37, g: 225, b: 153, a: 148 }), border: 0.4709828, border_color: Color { r: 62, g: 186, b: 67, a: 21 }, radius: 7.6075397, clip: true, absolute: false, inset: Edges { top: Length::Px(41.495773), right: Length::Px(72.657524), bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: true, bind_disabled: false, image: false },
                    NodeSpec { children: 3, width: Length::Px(86.966125), height: Length::Auto, padding: 0.84681976, gap: 2.8076894, direction: Direction::Column, background: Some(Color { r: 61, g: 30, b: 126, a: 4 }), border: 0.4987102, border_color: Color { r: 131, g: 180, b: 96, a: 29 }, radius: 1.6237, clip: true, absolute: true, inset: Edges { top: Length::Px(20.90008), right: Length::Auto, bottom: Length::Px(93.476234), left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 6.057809, gap: 1.6023473, direction: Direction::Row, background: Some(Color { r: 115, g: 73, b: 142, a: 145 }), border: 1.6511965, border_color: Color { r: 175, g: 147, b: 213, a: 232 }, radius: 0.6776339, clip: true, absolute: true, inset: Edges { top: Length::Px(39.47943), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Px(61.765137), height: Length::Auto, padding: 0.85120946, gap: 0.95010024, direction: Direction::Row, background: Some(Color { r: 226, g: 81, b: 240, a: 186 }), border: 2.2426782, border_color: Color { r: 60, g: 101, b: 17, a: 53 }, radius: 5.7743363, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(28.703823), bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Auto, height: Length::Px(88.11439), padding: 3.4001927, gap: 7.549609, direction: Direction::Column, background: None, border: 0.57856405, border_color: Color { r: 44, g: 168, b: 10, a: 37 }, radius: 3.7204359, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(16.92067), bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: true, image: true },
                    NodeSpec { children: 2, width: Length::Px(59.12697), height: Length::Px(80.87143), padding: 5.2976694, gap: 7.607315, direction: Direction::Column, background: Some(Color { r: 150, g: 195, b: 208, a: 215 }), border: 2.7282326, border_color: Color { r: 176, g: 40, b: 125, a: 233 }, radius: 3.5051274, clip: true, absolute: true, inset: Edges { top: Length::Px(115.60308), right: Length::Auto, bottom: Length::Px(69.97305), left: Length::Px(66.02197) }, bind_width: false, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Px(94.74095), padding: 4.269309, gap: 1.1295856, direction: Direction::Column, background: Some(Color { r: 246, g: 194, b: 177, a: 81 }), border: 0.8461132, border_color: Color { r: 15, g: 60, b: 149, a: 64 }, radius: 2.429125, clip: false, absolute: false, inset: Edges { top: Length::Px(5.674514), right: Length::Auto, bottom: Length::Px(75.03263), left: Length::Auto }, bind_width: false, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Px(68.40522), padding: 5.556398, gap: 0.03207418, direction: Direction::Row, background: Some(Color { r: 84, g: 232, b: 185, a: 254 }), border: 1.968023, border_color: Color { r: 187, g: 0, b: 152, a: 217 }, radius: 11.037146, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(0.1456055), bottom: Length::Px(85.241), left: Length::Px(67.23364) }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                ],
                children: vec![vec![1], vec![2, 3, 4], vec![5, 6, 7], vec![8], vec![], vec![], vec![], vec![], vec![]],
            },
            vec![
                vec![Edit::InnerScroll(172.50739), Edit::Text(1, "IWu.".to_string()), Edit::InnerScroll(128.87663)],
                vec![Edit::Scroll(96.45899), Edit::Radius(0, 4.3364897)],
                vec![Edit::Viewport(Size { width: 234.70947, height: 141.02023 }, 2.0)],
                vec![Edit::ListAppend(6)],
            ],
            vec![
                vec![Edit::DownAt(Point { x: 160.0, y: 120.0 }), Edit::Tab(false), Edit::Move(Point { x: 195.33284, y: 137.07268 })],
                vec![Edit::Text(0, ",?7?..,?".to_string()), Edit::Color(1, Color { r: 147, g: 184, b: 196, a: 165 })],
                vec![Edit::Move(Point { x: 175.1018, y: 146.35692 }), Edit::UpAt(Point { x: 175.1018, y: 146.35692 })],
                vec![Edit::Color(0, Color { r: 174, g: 247, b: 186, a: 155 })],
            ],
        ),
        (
            SceneData {
                nodes: vec![
                    NodeSpec { children: 3, width: Length::Px(86.966125), height: Length::Auto, padding: 0.84681976, gap: 2.8076894, direction: Direction::Column, background: Some(Color { r: 61, g: 30, b: 126, a: 4 }), border: 0.4987102, border_color: Color { r: 131, g: 180, b: 96, a: 29 }, radius: 1.6237, clip: true, absolute: true, inset: Edges { top: Length::Px(20.90008), right: Length::Auto, bottom: Length::Px(93.476234), left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 6.057809, gap: 1.6023473, direction: Direction::Row, background: Some(Color { r: 115, g: 73, b: 142, a: 145 }), border: 1.6511965, border_color: Color { r: 175, g: 147, b: 213, a: 232 }, radius: 0.6776339, clip: true, absolute: true, inset: Edges { top: Length::Px(39.47943), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: false, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Px(61.765137), height: Length::Auto, padding: 0.85120946, gap: 0.95010024, direction: Direction::Row, background: Some(Color { r: 226, g: 81, b: 240, a: 186 }), border: 2.2426782, border_color: Color { r: 60, g: 101, b: 17, a: 53 }, radius: 5.7743363, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(28.703823), bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 0, width: Length::Auto, height: Length::Px(88.11439), padding: 3.4001927, gap: 7.549609, direction: Direction::Column, background: None, border: 0.57856405, border_color: Color { r: 44, g: 168, b: 10, a: 37 }, radius: 3.7204359, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(16.92067), bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true },
                    NodeSpec { children: 2, width: Length::Px(59.12697), height: Length::Px(80.87143), padding: 5.2976694, gap: 7.607315, direction: Direction::Column, background: Some(Color { r: 150, g: 195, b: 208, a: 215 }), border: 2.7282326, border_color: Color { r: 176, g: 40, b: 125, a: 233 }, radius: 3.5051274, clip: true, absolute: true, inset: Edges { top: Length::Px(115.60308), right: Length::Auto, bottom: Length::Px(69.97305), left: Length::Px(66.02197) }, bind_width: false, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                    NodeSpec { children: 3, width: Length::Auto, height: Length::Px(94.74095), padding: 4.269309, gap: 1.1295856, direction: Direction::Column, background: Some(Color { r: 246, g: 194, b: 177, a: 81 }), border: 0.8461132, border_color: Color { r: 15, g: 60, b: 149, a: 64 }, radius: 2.429125, clip: false, absolute: false, inset: Edges { top: Length::Px(5.674514), right: Length::Auto, bottom: Length::Px(75.03263), left: Length::Auto }, bind_width: false, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: false, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false },
                    NodeSpec { children: 1, width: Length::Auto, height: Length::Px(68.40522), padding: 5.556398, gap: 0.03207418, direction: Direction::Row, background: Some(Color { r: 84, g: 232, b: 185, a: 254 }), border: 1.968023, border_color: Color { r: 187, g: 0, b: 152, a: 217 }, radius: 11.037146, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(0.1456055), bottom: Length::Px(85.241), left: Length::Px(67.23364) }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: true, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true },
                ],
                children: vec![vec![1, 2, 3], vec![4], vec![], vec![], vec![5, 6], vec![], vec![]],
            },
            vec![
                vec![Edit::Viewport(Size { width: 160.0, height: 120.0 }, 1.0)],
            ],
            vec![
                vec![Edit::Move(Point { x: 166.16211, y: 116.51529 })],
                vec![Edit::Move(Point { x: 147.68515, y: 160.29877 }), Edit::Color(2, Color { r: 92, g: 246, b: 254, a: 168 }), Edit::Text(0, "".to_string()), Edit::Move(Point { x: 170.47397, y: 167.93658 })],
                vec![Edit::DownAt(Point { x: 170.47397, y: 167.93658 }), Edit::DownAt(Point { x: 170.47397, y: 167.93658 }), Edit::Move(Point { x: 181.67816, y: 181.99153 })],
            ],
        ),
    ]
}

/// Cases the oracle found where a hover patch that changes layout moves its
/// node off a resting pointer. Before a layout stopped taking such a hover
/// away, the hover flipped with every frame that laid out, so the scene never
/// settled or showed a state that depended on how many passes had run. The
/// cases are kept as data, under the seed prefix each was found with,
/// because their seeds only regenerate them under the case strategy of the
/// time. The image seeds stand in for the recorded images, whose sizes the
/// failures depend on and whose pixels they do not.
#[rustfmt::skip]
fn hover_flip_cases() -> Vec<Case> {
    vec![
        // d31561be
        (SceneData { nodes: vec![NodeSpec { children: 2, width: Length::Auto, height: Length::Px(22.135395), padding: 7.1271276, gap: 3.816992, direction: Direction::Row, background: None, border: 0.92691267, border_color: Color { r: 205, g: 170, b: 114, a: 249 }, radius: 2.8342714, clip: false, absolute: true, inset: Edges { top: Length::Px(86.60999), right: Length::Auto, bottom: Length::Px(97.30402), left: Length::Px(24.720861) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: false, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Auto, height: Length::Px(115.05155), padding: 6.03812, gap: 2.402843, direction: Direction::Row, background: Some(Color { r: 235, g: 144, b: 73, a: 82 }), border: 0.79171973, border_color: Color { r: 64, g: 222, b: 131, a: 83 }, radius: 11.292285, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Px(62.70579) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 7.7613564, gap: 0.133545, direction: Direction::Row, background: Some(Color { r: 251, g: 83, b: 51, a: 182 }), border: 0.7793249, border_color: Color { r: 49, g: 159, b: 226, a: 42 }, radius: 8.5028925, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(78.93151), left: Length::Px(19.413181) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: false }, NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 6.110388, gap: 0.15034229, direction: Direction::Row, background: Some(Color { r: 116, g: 236, b: 165, a: 230 }), border: 0.89120346, border_color: Color { r: 58, g: 177, b: 125, a: 253 }, radius: 10.132989, clip: false, absolute: false, inset: Edges { top: Length::Px(80.685295), right: Length::Auto, bottom: Length::Px(111.68387), left: Length::Px(77.18086) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: true, text: true, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 1, width: Length::Auto, height: Length::Auto, padding: 3.919617, gap: 3.869909, direction: Direction::Column, background: None, border: 2.9327772, border_color: Color { r: 110, g: 15, b: 229, a: 29 }, radius: 2.5559003, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(4.920566), bottom: Length::Px(8.859781), left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: true, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: true }, NodeSpec { children: 3, width: Length::Auto, height: Length::Px(32.495037), padding: 1.1422375, gap: 6.6334805, direction: Direction::Row, background: Some(Color { r: 98, g: 86, b: 65, a: 229 }), border: 2.4477189, border_color: Color { r: 18, g: 186, b: 69, a: 39 }, radius: 6.5804048, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: true, image: true }, NodeSpec { children: 2, width: Length::Px(69.20226), height: Length::Px(57.680252), padding: 0.09475468, gap: 6.268997, direction: Direction::Row, background: Some(Color { r: 113, g: 137, b: 236, a: 38 }), border: 1.5249767, border_color: Color { r: 98, g: 216, b: 38, a: 60 }, radius: 0.095000796, clip: true, absolute: false, inset: Edges { top: Length::Px(93.51294), right: Length::Px(96.25661), bottom: Length::Px(75.06952), left: Length::Px(62.10578) }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: true, show: false, text: true, bind_text: false, bind_glyph_color: false, hover_variant: false, hover_layout: true, pressed_variant: false, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 7.2222548, gap: 3.7930589, direction: Direction::Column, background: None, border: 1.707228, border_color: Color { r: 99, g: 192, b: 100, a: 114 }, radius: 2.248687, clip: false, absolute: true, inset: Edges { top: Length::Px(57.89299), right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: false, bind_hidden: false, show: true, text: true, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: true, bind_disabled: false, image: true }, NodeSpec { children: 3, width: Length::Px(74.70028), height: Length::Auto, padding: 4.665137, gap: 7.9404497, direction: Direction::Row, background: Some(Color { r: 205, g: 27, b: 91, a: 209 }), border: 0.026089218, border_color: Color { r: 46, g: 236, b: 109, a: 145 }, radius: 3.5172212, clip: true, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(18.460613), bottom: Length::Px(25.144638), left: Length::Px(114.2589) }, bind_width: true, bind_background: false, bind_radius: true, bind_hidden: false, show: true, text: true, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: true, image: true }, NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 1.6206552, gap: 3.31681, direction: Direction::Row, background: Some(Color { r: 227, g: 195, b: 221, a: 85 }), border: 2.847082, border_color: Color { r: 141, g: 173, b: 64, a: 51 }, radius: 8.931896, clip: true, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(45.17426), bottom: Length::Px(98.21473), left: Length::Px(25.457132) }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: true, show: true, text: false, bind_text: true, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: true, image: true }, NodeSpec { children: 0, width: Length::Auto, height: Length::Px(77.79444), padding: 4.1618657, gap: 2.4673324, direction: Direction::Row, background: Some(Color { r: 233, g: 125, b: 211, a: 165 }), border: 1.9711708, border_color: Color { r: 196, g: 107, b: 190, a: 124 }, radius: 2.9041157, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: true, show: false, text: false, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }], children: vec![vec![1, 2], vec![], vec![3, 4, 5], vec![6], vec![7], vec![8, 9, 10], vec![], vec![], vec![], vec![], vec![]] }, vec![vec![Edit::Viewport(Size { width: 160.0, height: 120.0 }, 1.0), Edit::Image(0, Some(gradient(1)))]], vec![vec![Edit::UpAt(Point { x: 160.0, y: 120.0 })], vec![Edit::Tab(false)]]),
        // eb4961e9
        (SceneData { nodes: vec![NodeSpec { children: 1, width: Length::Px(66.42263), height: Length::Auto, padding: 6.5076947, gap: 6.1716876, direction: Direction::Column, background: None, border: 1.4091809, border_color: Color { r: 152, g: 102, b: 18, a: 162 }, radius: 5.34025, clip: false, absolute: false, inset: Edges { top: Length::Px(52.99718), right: Length::Auto, bottom: Length::Px(21.616596), left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: true, bind_glyph_color: false, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 3.8388216, gap: 3.6751714, direction: Direction::Column, background: None, border: 2.5316968, border_color: Color { r: 50, g: 137, b: 1, a: 33 }, radius: 2.046954, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Px(29.964094) }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: true, show: true, text: true, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: true, focused_variant: true, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 2.0, gap: 2.0, direction: Direction::Row, background: None, border: 0.0, border_color: Color { r: 0, g: 0, b: 0, a: 255 }, radius: 0.0, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true }], children: vec![vec![1, 2], vec![], vec![]] }, vec![vec![Edit::Disabled(0), Edit::Image(0, Some(gradient(1)))], vec![Edit::Disabled(0), Edit::Image(0, Some(gradient(2)))]], vec![vec![Edit::UpAt(Point { x: 160.0, y: 120.0 })], vec![Edit::Tab(false)]]),
        // be2c49b7
        (SceneData { nodes: vec![NodeSpec { children: 3, width: Length::Auto, height: Length::Px(21.195436), padding: 2.9182625, gap: 2.3778238, direction: Direction::Row, background: Some(Color { r: 115, g: 110, b: 52, a: 5 }), border: 2.408015, border_color: Color { r: 253, g: 52, b: 243, a: 169 }, radius: 10.46908, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Px(78.45324), bottom: Length::Auto, left: Length::Px(35.324383) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Px(0.5017597), height: Length::Auto, padding: 4.827228, gap: 3.8587134, direction: Direction::Row, background: Some(Color { r: 212, g: 40, b: 13, a: 87 }), border: 1.142297, border_color: Color { r: 221, g: 154, b: 72, a: 243 }, radius: 7.001988, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Px(54.66066), bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: false, bind_hidden: true, show: true, text: true, bind_text: false, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Px(107.22907), height: Length::Auto, padding: 4.3450546, gap: 1.8220693, direction: Direction::Row, background: Some(Color { r: 112, g: 39, b: 166, a: 96 }), border: 0.3746236, border_color: Color { r: 154, g: 51, b: 88, a: 229 }, radius: 10.4165325, clip: true, absolute: false, inset: Edges { top: Length::Px(13.932853), right: Length::Px(77.85852), bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: false, bind_radius: false, bind_hidden: true, show: true, text: true, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: true, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 0.7069759, gap: 7.8078136, direction: Direction::Column, background: Some(Color { r: 107, g: 215, b: 242, a: 203 }), border: 2.3430562, border_color: Color { r: 231, g: 83, b: 52, a: 142 }, radius: 3.239581, clip: true, absolute: false, inset: Edges { top: Length::Px(60.770233), right: Length::Auto, bottom: Length::Auto, left: Length::Px(16.558931) }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: true, show: true, text: true, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: true, pressed_variant: true, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 2.0, gap: 2.0, direction: Direction::Row, background: None, border: 0.0, border_color: Color { r: 0, g: 0, b: 0, a: 255 }, radius: 0.0, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true }], children: vec![vec![1, 2, 3, 4], vec![], vec![], vec![], vec![]] }, vec![vec![Edit::Show(1)], vec![Edit::Viewport(Size { width: 160.0, height: 201.38507 }, 1.0), Edit::Image(0, Some(gradient(1))), Edit::Image(0, Some(gradient(2)))]], vec![vec![Edit::Move(Point { x: 160.0, y: 120.0 })], vec![Edit::Tab(false)]]),
        // 18268dae
        (SceneData { nodes: vec![NodeSpec { children: 3, width: Length::Auto, height: Length::Auto, padding: 7.5482373, gap: 3.0879421, direction: Direction::Column, background: None, border: 2.721657, border_color: Color { r: 215, g: 93, b: 10, a: 65 }, radius: 1.2262502, clip: false, absolute: true, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: true, bind_background: true, bind_radius: true, bind_hidden: false, show: false, text: false, bind_text: true, bind_glyph_color: true, hover_variant: false, hover_layout: false, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: false, image: false }, NodeSpec { children: 2, width: Length::Auto, height: Length::Px(34.01728), padding: 2.423748, gap: 6.5032988, direction: Direction::Column, background: Some(Color { r: 29, g: 101, b: 177, a: 117 }), border: 1.7483302, border_color: Color { r: 197, g: 194, b: 101, a: 184 }, radius: 7.54314, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Px(35.905777), left: Length::Px(115.589424) }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: true, show: true, text: true, bind_text: true, bind_glyph_color: true, hover_variant: true, hover_layout: true, pressed_variant: false, focused_variant: false, disabled_variant: false, bind_disabled: true, image: false }, NodeSpec { children: 0, width: Length::Auto, height: Length::Auto, padding: 2.0, gap: 2.0, direction: Direction::Row, background: None, border: 0.0, border_color: Color { r: 0, g: 0, b: 0, a: 255 }, radius: 0.0, clip: false, absolute: false, inset: Edges { top: Length::Auto, right: Length::Auto, bottom: Length::Auto, left: Length::Auto }, bind_width: false, bind_background: false, bind_radius: false, bind_hidden: false, show: false, text: false, bind_text: false, bind_glyph_color: false, hover_variant: true, hover_layout: true, pressed_variant: true, focused_variant: true, disabled_variant: true, bind_disabled: true, image: true }], children: vec![vec![1, 2], vec![], vec![]] }, vec![vec![Edit::Disabled(1), Edit::Image(0, Some(gradient(1))), Edit::Image(0, Some(gradient(2)))], vec![Edit::Scroll(170.74878), Edit::Disabled(1), Edit::Viewport(Size { width: 297.5174, height: 163.38925 }, 1.0)], vec![Edit::Color(0, Color { r: 184, g: 29, b: 19, a: 169 }), Edit::ListAppend(5), Edit::Disabled(1)], vec![Edit::ListReverse, Edit::Disabled(1), Edit::Radius(0, 7.898796), Edit::Image(0, Some(gradient(3)))]], vec![vec![Edit::GlyphColor(0, Color { r: 188, g: 198, b: 22, a: 154 }), Edit::Text(0, String::from("?.AV.k5")), Edit::Text(0, String::from("9?t?G")), Edit::Move(Point { x: 175.16779, y: 140.59702 })], vec![Edit::Color(0, Color { r: 26, g: 74, b: 99, a: 11 }), Edit::Tab(true)]]),
    ]
}

#[test]
fn the_hover_flip_cases_match_a_fresh_mount() {
    for (data, geometry, events) in hover_flip_cases() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            check_scene(scale, &data, &geometry, &events);
        }
    }
}

#[test]
fn the_first_image_strategy_cases_match_a_full_repaint() {
    for (data, geometry, events) in first_image_strategy_cases() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            check_scene(scale, &data, &geometry, &events);
        }
    }
}
