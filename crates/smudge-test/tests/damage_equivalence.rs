//! Property test that a surface repainted from damage equals a full repaint.
//!
//! Every case builds a random tree of panels, applies a random sequence of
//! edits, and compares the surface after each frame with a full repaint of the
//! same display list. A difference is a stale pixel on screen.

use std::cell::Cell;
use std::rc::Rc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use smudge_core::element::{Element, show};
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{prelude::*, signal_local};
use smudge_core::style::{Direction, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// The viewport every generated scene is laid out in, in logical pixels.
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

/// The longest edit sequence one case applies.
const MAX_EDITS: usize = 20;

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
    /// Whether the width is read from a signal instead of being fixed.
    bind_width: bool,
    /// Whether the background is read from a signal instead of being fixed.
    bind_background: bool,
    /// Whether the corner radius is read from a signal instead of being fixed.
    bind_radius: bool,
    /// Whether the node is mounted through a visibility signal.
    show: bool,
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

/// Which of a node's properties read a signal.
fn any_bindings() -> impl Strategy<Value = (bool, bool, bool, bool)> {
    (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>())
}

/// Any node of a generated scene.
fn any_spec() -> impl Strategy<Value = NodeSpec> {
    (any_layout(), any_paint(), any_bindings()).prop_map(|(layout, paint, bindings)| {
        let (children, width, height, padding, gap, direction) = layout;
        let (background, border, border_color, radius, clip) = paint;
        let (bind_width, bind_background, bind_radius, show) = bindings;
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
            bind_width,
            bind_background,
            bind_radius,
            show,
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
    /// The result always holds a signal of every kind an edit can target, so
    /// that every generated edit sequence changes something: the root reads
    /// its width, background and corner radius from signals, and the first
    /// child is mounted through a visibility signal. The root itself is never
    /// mounted through one, because the harness builds it directly.
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
        nodes[0].bind_width = true;
        nodes[0].bind_background = true;
        nodes[0].bind_radius = true;
        if !nodes[1..].iter().any(|node| node.show) {
            nodes[1].show = true;
        }

        SceneData { nodes, children }
    }

    /// How many signals of each kind the scene creates.
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
        }
    }
}

/// How many signals of each kind a scene creates.
#[derive(Copy, Clone, Debug)]
struct SignalCounts {
    colors: usize,
    widths: usize,
    radii: usize,
    shows: usize,
}

/// Reads one property of a scene.
type Getter<T> = Rc<dyn Fn() -> T>;

/// Writes one property of a scene.
type Setter<T> = Box<dyn Fn(T)>;

/// Creates the signal of one property and returns the handles that read and
/// write it.
///
/// Signals are created before the app, so they live outside every node's owner
/// and survive a node being removed and mounted again.
fn property<T: Clone + 'static>(value: T) -> (Getter<T>, Setter<T>) {
    let (read, write) = signal_local(value);
    (
        Rc::new(move || read.get()),
        Box::new(move |value| write.set(value)),
    )
}

/// One node of a built scene.
struct SceneNode {
    /// The properties the node's element is built from.
    spec: NodeSpec,
    /// The indices of the node's children.
    children: Vec<usize>,
    /// The background the node paints, when a signal supplies it.
    color: Option<Getter<Color>>,
    /// The width the node lays out with, when a signal supplies it.
    width: Option<Getter<Length>>,
    /// The corner radius the node paints with, when a signal supplies it.
    radius: Option<Getter<f32>>,
    /// The visibility the node is mounted with, when a signal supplies it.
    show: Option<Getter<bool>>,
}

/// The signal that mounts one node, with the value it holds.
///
/// Edits are the only writer, so `visible` always matches the signal, and an
/// edit flips it without reading the signal outside a binding.
struct ShowSignal {
    /// The visibility the signal holds now.
    visible: Cell<bool>,
    /// Writes a new visibility.
    set: Setter<bool>,
}

/// A built scene: its nodes and the signals its bindings read.
///
/// The signal handles stay here so an edit can change one property without
/// rebuilding the scene.
struct Scene {
    nodes: Vec<SceneNode>,
    colors: Vec<Setter<Color>>,
    widths: Vec<Setter<Length>>,
    radii: Vec<Setter<f32>>,
    shows: Vec<ShowSignal>,
}

impl Scene {
    /// Creates the signals the scene's bindings read.
    fn new(data: &SceneData) -> Scene {
        let mut scene = Scene {
            nodes: Vec::with_capacity(data.nodes.len()),
            colors: Vec::new(),
            widths: Vec::new(),
            radii: Vec::new(),
            shows: Vec::new(),
        };
        for (index, spec) in data.nodes.iter().enumerate() {
            let color = if spec.bind_background {
                let (get, set) = property(spec.background.unwrap_or(Color::TRANSPARENT));
                scene.colors.push(set);
                Some(get)
            } else {
                None
            };
            let width = if spec.bind_width {
                let (get, set) = property(spec.width);
                scene.widths.push(set);
                Some(get)
            } else {
                None
            };
            let radius = if spec.bind_radius {
                let (get, set) = property(spec.radius);
                scene.radii.push(set);
                Some(get)
            } else {
                None
            };
            let show = if spec.show {
                let (get, set) = property(true);
                scene.shows.push(ShowSignal {
                    visible: Cell::new(true),
                    set,
                });
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
                show,
            });
        }
        scene
    }
}

/// Builds the element of the node at `index`, with every child of every node
/// built the same way.
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

    for &child in &node.children {
        element = match &scene.nodes[child].show {
            Some(visible) => {
                let scene = Rc::clone(scene);
                let visible = Rc::clone(visible);
                element.child(show(move || visible(), move || build_panel(&scene, child)))
            }
            None => element.child(build_panel(scene, child)),
        };
    }
    element
}

/// One change a case applies to a built scene.
#[derive(Clone, Debug)]
enum Edit {
    /// Sets the background signal at this index to a color.
    Color(usize, Color),
    /// Sets the width signal at this index to a length.
    Width(usize, Length),
    /// Sets the corner radius signal at this index to a radius.
    Radius(usize, f32),
    /// Flips the visibility signal at this index, which removes the node it
    /// mounts or mounts it again.
    Show(usize),
}

impl Edit {
    /// Applies the edit to `scene`, so the next frame sees it.
    fn apply(&self, scene: &Scene) {
        match *self {
            Edit::Color(index, color) => (scene.colors[index])(color),
            Edit::Width(index, width) => (scene.widths[index])(width),
            Edit::Radius(index, radius) => (scene.radii[index])(radius),
            Edit::Show(index) => {
                let show = &scene.shows[index];
                let visible = !show.visible.get();
                show.visible.set(visible);
                (show.set)(visible);
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
}

/// Any edit kind, all four of which every scene has a signal for.
fn any_edit_kind() -> impl Strategy<Value = EditKind> {
    prop_oneof![
        Just(EditKind::Color),
        Just(EditKind::Width),
        Just(EditKind::Radius),
        Just(EditKind::Show),
    ]
}

/// One edit, targeting a signal that `counts` describes.
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
    })
}

/// A generated case: a scene and the edits applied to it after it is built.
fn any_case() -> impl Strategy<Value = (SceneData, Vec<Edit>)> {
    prop::collection::vec(any_spec(), 2..=40)
        .prop_map(SceneData::new)
        .prop_flat_map(|data| {
            let counts = data.signal_counts();
            (
                Just(data),
                prop::collection::vec(any_edit(counts), 1..=MAX_EDITS),
            )
        })
}

/// Builds `data` at `scale`, applies `edits`, and checks the surface after
/// every frame.
///
/// The first frame repaints the whole surface, so the comparison only starts
/// to depend on the damage after the first edit.
fn check_scene(scale: f32, data: &SceneData, edits: &[Edit]) {
    let scene = Rc::new(Scene::new(data));
    let built = Rc::clone(&scene);
    let mut harness = Harness::new(VIEWPORT.width, VIEWPORT.height, scale, move || {
        build_panel(&built, 0)
    });

    harness.frame();
    harness.assert_matches_full_repaint();

    for edit in edits {
        edit.apply(&scene);
        harness.frame();
        harness.assert_matches_full_repaint();
    }
}

/// Declares the damage equivalence property test once per scale factor.
macro_rules! damage_equivalence_tests {
    ($($name:ident at $scale:expr;)*) => {
        proptest! {
            #![proptest_config(config())]
            $(
                #[test]
                fn $name((data, edits) in any_case()) {
                    check_scene($scale, &data, &edits);
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
