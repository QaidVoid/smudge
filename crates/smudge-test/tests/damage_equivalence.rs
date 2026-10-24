//! Property test that a surface repainted from damage equals a full repaint
//! and a fresh mount of the same scene.
//!
//! Every case builds a random tree of panels and text, applies a random
//! sequence of edits, and checks the surface after every frame twice. The
//! comparison with a full repaint of the same display list fails when the
//! damage misses pixels. The comparison with a fresh app built from the final
//! value of every signal fails when the display list itself is stale, which a
//! missed invalidation causes and the first comparison cannot see. A difference
//! in either is a stale pixel on screen.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use smudge_core::element::{AnyElement, Element, IntoElement, show};
use smudge_core::geometry::{Color, Size};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{prelude::*, signal_local};
use smudge_core::style::{Direction, Edges, Length, px};
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_test::Harness;
use smudge_text::{TextLayout, TextStyle, TextSystem};

/// The viewport a scene starts in, in logical pixels.
const VIEWPORT: Size = Size {
    width: 320.0,
    height: 240.0,
};

/// The number of cases each property test runs when `PROPTEST_CASES` is unset.
const DEFAULT_CASES: u32 = 64;

/// The deepest level a generated node may sit at, counting the root as one.
const MAX_DEPTH: u32 = 4;

/// The most children one generated node asks for.
const MAX_CHILDREN: u8 = 3;

/// The most frames one case runs, one comparison after each.
const MAX_FRAMES: usize = 8;

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

/// Any node of a generated scene.
fn any_spec() -> impl Strategy<Value = NodeSpec> {
    (
        any_layout(),
        any_paint(),
        any_position(),
        any_bindings(),
        any_text(),
    )
        .prop_map(|(layout, paint, position, bindings, text)| {
            let (children, width, height, padding, gap, direction) = layout;
            let (background, border, border_color, radius, clip) = paint;
            let (absolute, inset) = position;
            let (bind_width, bind_background, bind_radius, bind_hidden, show) = bindings;
            let (text, bind_text, bind_glyph_color) = text;
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
        for mut run in layout.glyph_runs_at(cx.rect.origin()) {
            run.color = self.color;
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

/// Where a scene's bound property values come from.
#[derive(Clone, Copy)]
enum Values<'a> {
    /// Signals at the values their specs start with, which is what a reactive
    /// scene reads.
    Signals,
    /// The values another scene holds now, which is what a fresh mount of it
    /// reads.
    Current(&'a Scene),
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
    texts: Vec<TextTracked>,
    glyph_colors: Vec<Tracked<Color>>,
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

    /// Creates the static scene that shows what `from` shows now: every bound
    /// property holds the value `from`'s property has at this moment, and the
    /// viewport and scale are copied from it.
    ///
    /// Its mount is a fresh app in the state the reactive scene reached.
    fn fresh(data: &SceneData, from: &Scene) -> Scene {
        Scene::build(
            data,
            Values::Current(from),
            from.viewport.get(),
            from.scale.get(),
        )
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
            texts: Vec::new(),
            glyph_colors: Vec::new(),
            viewport: Cell::new(viewport),
            scale: Cell::new(scale),
        };
        let (mut color, mut width, mut radius, mut hidden, mut show, mut text, mut glyph_color) =
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        for (index, spec) in data.nodes.iter().enumerate() {
            let color = if spec.bind_background {
                let value = match values {
                    Values::Signals => {
                        Tracked::reactive(spec.background.unwrap_or(Color::TRANSPARENT))
                    }
                    Values::Current(from) => Tracked::static_value(from.colors[color].value.get()),
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
                    Values::Current(from) => Tracked::static_value(from.widths[width].value.get()),
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
                    Values::Current(from) => Tracked::static_value(from.radii[radius].value.get()),
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
                    Values::Current(from) => {
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
                    Values::Current(from) => Tracked::static_value(from.shows[show].value.get()),
                };
                show += 1;
                let get = value.get.clone();
                scene.shows.push(value);
                Some(get)
            } else {
                None
            };
            let text = if spec.text && spec.bind_text {
                let value = match values {
                    Values::Signals => TextTracked::reactive(initial_text(index)),
                    Values::Current(from) => {
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
                    Values::Current(from) => {
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
            element.corner_radius(move || radius())
        }
        None => element.corner_radius(spec.radius),
    };
    element = match (&node.color, spec.background) {
        (Some(color), _) => {
            let color = Rc::clone(color);
            element.background(move || color())
        }
        (None, Some(color)) => element.background(color),
        (None, None) => element,
    };
    if let Some(hidden) = &node.hidden {
        let hidden = Rc::clone(hidden);
        element = element.hidden(move || hidden());
    }

    append_children(scene, index, element)
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
}

impl Edit {
    /// Applies the edit, so the next frame sees it.
    ///
    /// Every edit but a viewport one writes a property of the scene. A
    /// viewport edit changes the window the harness lays the scene out in,
    /// which is why it needs the harness.
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
    })
}

/// The edits one frame applies: one to four of them, which the frame carries
/// together.
fn any_edit_group(counts: SignalCounts) -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(any_edit(counts), 1..=MAX_EDITS_PER_FRAME)
}

/// A generated case: a scene and the edits applied to it after it is built,
/// one group per frame.
fn any_case() -> impl Strategy<Value = (SceneData, Vec<Vec<Edit>>)> {
    prop::collection::vec(any_spec(), 2..=40)
        .prop_map(SceneData::new)
        .prop_flat_map(|data| {
            let counts = data.signal_counts();
            (
                Just(data),
                prop::collection::vec(any_edit_group(counts), 1..=MAX_FRAMES),
            )
        })
}

/// Requires the surface of `harness` to match a fresh mount of the final state
/// of `data`, byte for byte.
///
/// The fresh mount reads no signal: every bound property is the value the
/// reactive scene holds right now, which is what [`Tracked`] and
/// [`TextTracked`] keep. A difference means the incremental frames left the
/// screen stale, which the full-repaint comparison cannot see because it
/// repaints the same stale list.
fn assert_matches_fresh_mount(harness: &Harness, scene: &Scene, data: &SceneData) {
    let fresh = Rc::new(Scene::fresh(data, scene));
    let built = Rc::clone(&fresh);
    let viewport = scene.viewport.get();
    let scale = scene.scale.get();
    let mut mount = Harness::new(viewport.width, viewport.height, scale, move || {
        build_node(&built, 0)
    });
    mount.frame();
    harness.assert_matches(&mount);
}

/// Builds `data` at `scale`, applies the edits of each frame, and checks the
/// surface after every frame.
///
/// The first frame repaints the whole surface, so the comparison only starts
/// to depend on the damage after the first edit.
fn check_scene(scale: f32, data: &SceneData, frames: &[Vec<Edit>]) {
    TextSystem::bundled_only().install();
    let scene = Rc::new(Scene::reactive(data, scale));
    let built = Rc::clone(&scene);
    let mut harness = Harness::new(VIEWPORT.width, VIEWPORT.height, scale, move || {
        build_node(&built, 0)
    });

    harness.frame();
    harness.assert_matches_full_repaint();
    assert_matches_fresh_mount(&harness, &scene, data);

    for edits in frames {
        for edit in edits {
            edit.apply(&scene, &mut harness);
        }
        harness.frame();
        harness.assert_matches_full_repaint();
        assert_matches_fresh_mount(&harness, &scene, data);
    }
}

/// Declares the damage equivalence property test once per scale factor.
macro_rules! damage_equivalence_tests {
    ($($name:ident at $scale:expr;)*) => {
        proptest! {
            #![proptest_config(config())]
            $(
                #[test]
                fn $name((data, frames) in any_case()) {
                    check_scene($scale, &data, &frames);
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
