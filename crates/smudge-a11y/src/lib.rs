//! The bridge between the widget tree's accessibility projection and
//! AccessKit.
//!
//! [`Projection`] turns what an [`App`] reports about its
//! widgets into the [`TreeUpdate`]s an `accesskit_winit::Adapter` sends to
//! the platform's accessibility APIs. The platform layer owns the adapter
//! and the window; this crate owns only the mapping, so it needs no window,
//! no event loop and no thread, and everything it does can be tested in
//! process.
//!
//! Accessibility for web and other wasm targets is not supported in this
//! release: browsers expose their own accessibility trees, and projecting
//! into them needs a different transport.

#![forbid(unsafe_code)]

use smudge_core::app::App;
use smudge_core::geometry::Rect;
use smudge_core::semantics::Role;
use smudge_core::semantics::SemanticsNode;
use smudge_core::tree::WidgetId;

use accesskit::{Node, NodeId, Rect as AccessKitRect};
use accesskit::{Role as AccessKitRole, TreeId, TreeInfo, TreeUpdate};

/// The node id of the synthesized window root.
///
/// A widget id encodes a slot-map version and index that both fit in 32
/// bits, with the null key alone using all ones for the index, so an id of
/// all ones can never collide with a widget.
const WINDOW_ROOT: NodeId = NodeId(u64::MAX);

/// Builds the AccessKit tree updates for one app.
///
/// Keep one projection per window. Call [`Projection::update`] once per
/// frame, after the frame has run, and hand the result to the adapter.
pub struct Projection {
    /// The window title, which names the window root.
    title: String,
    /// The surface size the window root's bounds were last sent with.
    surface: (f64, f64),
}

impl Projection {
    /// Creates a projection for a window with `title`.
    pub fn new(title: impl Into<String>) -> Projection {
        Projection {
            title: title.into(),
            surface: (0.0, 0.0),
        }
    }

    /// Builds the update that brings AccessKit up to date with `app`.
    ///
    /// `full` rebuilds every node from scratch, which the first update after
    /// an activation and the first update after a scale change need. The
    /// surface size is in physical pixels and `scale` is the mapping from the
    /// logical pixels the app reports its bounds in, so the projected bounds
    /// land where the widgets are drawn.
    ///
    /// The update always carries the current focus, even when nothing else
    /// changed, and it always names a node AccessKit knows, because a tree
    /// update must name a focused node.
    pub fn update(
        &mut self,
        app: &mut App,
        surface_width: f64,
        surface_height: f64,
        scale: f32,
        full: bool,
    ) -> TreeUpdate {
        let changes = app.take_a11y_changes();
        let surface = (surface_width, surface_height);
        let mut update = if full || changes.reset {
            self.build_full(app, surface, scale)
        } else {
            let mut nodes: Vec<(NodeId, Node)> = changes
                .changed
                .iter()
                .filter_map(|id| app.semantics_node(*id))
                .map(|node| (NodeId(node.id.as_u64()), build_node(&node, scale)))
                .collect();
            if changes.roots_changed || surface != self.surface {
                nodes.push((
                    WINDOW_ROOT,
                    self.window_node(app.semantics_roots(), surface),
                ));
            }
            TreeUpdate {
                nodes,
                tree: None,
                tree_id: TreeId::ROOT,
                focus: WINDOW_ROOT,
            }
        };
        self.surface = surface;
        update.focus = current_focus(app);
        update
    }

    /// Builds an update that carries the window root and every projected
    /// node.
    fn build_full(&self, app: &App, surface: (f64, f64), scale: f32) -> TreeUpdate {
        let projected = app.semantics_nodes();
        let roots = projected
            .iter()
            .filter(|node| node.parent.is_none())
            .map(|node| node.id)
            .collect();
        let mut nodes = Vec::with_capacity(projected.len() + 1);
        nodes.push((WINDOW_ROOT, self.window_node(roots, surface)));
        for node in &projected {
            nodes.push((NodeId(node.id.as_u64()), build_node(node, scale)));
        }
        let mut tree = TreeInfo::new(WINDOW_ROOT);
        tree.toolkit_name = Some("smudge".to_owned());
        TreeUpdate {
            nodes,
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            focus: WINDOW_ROOT,
        }
    }

    /// Builds the window root, which holds `roots` and fills the surface.
    fn window_node(&self, roots: Vec<WidgetId>, (width, height): (f64, f64)) -> Node {
        let mut window = Node::new(AccessKitRole::Window);
        window.set_label(self.title.clone());
        window.set_bounds(AccessKitRect {
            x0: 0.0,
            y0: 0.0,
            x1: width,
            y1: height,
        });
        window.set_children(node_ids(&roots));
        window
    }
}

/// Maps one projected node onto an AccessKit node.
///
/// The bounds are scaled to physical pixels and expanded outward, so a node
/// never reports a box smaller than the one it paints into. The node carries
/// its children, because AccessKit replaces a node it already knows with the
/// one an update carries.
fn build_node(node: &SemanticsNode, scale: f32) -> Node {
    let mut out = Node::new(map_role(node.role));
    if let Some(label) = &node.label {
        out.set_label(label.clone());
    }
    if let Some(value) = &node.value {
        out.set_value(value.clone());
    }
    out.set_bounds(scaled_bounds(node.bounds, scale));
    if !node.children.is_empty() {
        out.set_children(node_ids(&node.children));
    }
    if node.disabled {
        out.set_disabled();
    }
    if node.pressable {
        out.add_action(accesskit::Action::Click);
    }
    if node.focusable {
        out.add_action(accesskit::Action::Focus);
    }
    out
}

/// Maps widget ids onto the node ids the projection gives them.
fn node_ids(ids: &[WidgetId]) -> Vec<NodeId> {
    ids.iter().map(|id| NodeId(id.as_u64())).collect()
}

/// Returns the node id AccessKit should treat as focused.
///
/// The focused widget maps through only when it projects; a focus on a node
/// the projection leaves out falls back to the window root, which is how
/// AccessKit reads "no focus inside the tree".
fn current_focus(app: &App) -> NodeId {
    match app.focused().filter(|id| app.semantics_node(*id).is_some()) {
        Some(id) => NodeId(id.as_u64()),
        None => WINDOW_ROOT,
    }
}

/// Maps a projection role onto the AccessKit role with the same meaning.
fn map_role(role: Role) -> AccessKitRole {
    match role {
        Role::Window => AccessKitRole::Window,
        Role::Group => AccessKitRole::Group,
        Role::Button => AccessKitRole::Button,
        Role::Label => AccessKitRole::Label,
        Role::TextInput => AccessKitRole::TextInput,
        Role::ScrollView => AccessKitRole::ScrollView,
        Role::List => AccessKitRole::List,
        Role::ListItem => AccessKitRole::ListItem,
    }
}

/// Scales logical bounds to physical pixels, expanding outward.
fn scaled_bounds(bounds: Rect, scale: f32) -> AccessKitRect {
    AccessKitRect {
        x0: f64::from((bounds.x0 * scale).floor()),
        y0: f64::from((bounds.y0 * scale).floor()),
        x1: f64::from((bounds.x1 * scale).ceil()),
        y1: f64::from((bounds.y1 * scale).ceil()),
    }
}
