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

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

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
    /// Every node AccessKit holds, as it was last sent: a digest of the
    /// node, so an incremental update leaves out a node that a frame touched
    /// without changing anything it projects, such as a text field whose
    /// caret moved, and its children, so a node AccessKit dropped is
    /// forgotten and sent again when it returns.
    sent: HashMap<NodeId, Sent>,
    /// The roots the window root was last sent with.
    roots: Vec<NodeId>,
}

/// What the projection last sent for one node.
struct Sent {
    /// The digest of the node.
    digest: u64,
    /// The node's children.
    children: Vec<NodeId>,
}

impl Projection {
    /// Creates a projection for a window with `title`.
    pub fn new(title: impl Into<String>) -> Projection {
        Projection {
            title: title.into(),
            surface: (0.0, 0.0),
            sent: HashMap::new(),
            roots: Vec::new(),
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
    /// update must name a focused node. An incremental update leaves out a
    /// changed node whose projection is the same as the one last sent, so a
    /// large text value is sent again only when the text changed.
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
            let mut nodes: Vec<(NodeId, Node)> = Vec::new();
            for node in changes
                .changed
                .iter()
                .filter_map(|id| app.semantics_node(*id))
            {
                let id = NodeId(node.id.as_u64());
                let digest = digest(&node, scale);
                if self.sent.get(&id).is_some_and(|sent| sent.digest == digest) {
                    continue;
                }
                let children = node_ids(&node.children);
                let previous = self.sent.insert(
                    id,
                    Sent {
                        digest,
                        children: children.clone(),
                    },
                );
                if let Some(previous) = previous {
                    self.forget_dropped(&previous.children, &children);
                }
                nodes.push((id, build_node(&node, scale)));
            }
            if changes.roots_changed || surface != self.surface {
                let roots = app.semantics_roots();
                let roots = node_ids(&roots);
                let previous = std::mem::replace(&mut self.roots, roots.clone());
                self.forget_dropped(&previous, &roots);
                nodes.push((WINDOW_ROOT, self.window_node(roots, surface)));
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
    fn build_full(&mut self, app: &App, surface: (f64, f64), scale: f32) -> TreeUpdate {
        let projected = app.semantics_nodes();
        let roots: Vec<WidgetId> = projected
            .iter()
            .filter(|node| node.parent.is_none())
            .map(|node| node.id)
            .collect();
        let mut nodes = Vec::with_capacity(projected.len() + 1);
        let roots = node_ids(&roots);
        self.roots = roots.clone();
        nodes.push((WINDOW_ROOT, self.window_node(roots, surface)));
        self.sent.clear();
        for node in &projected {
            let id = NodeId(node.id.as_u64());
            let sent = Sent {
                digest: digest(node, scale),
                children: node_ids(&node.children),
            };
            self.sent.insert(id, sent);
            nodes.push((id, build_node(node, scale)));
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

    /// Forgets every node of `previous` that `current` no longer holds,
    /// with the nodes below it, since AccessKit drops a node no parent
    /// holds and it has to be sent whole when it returns.
    fn forget_dropped(&mut self, previous: &[NodeId], current: &[NodeId]) {
        if previous == current {
            return;
        }
        let current: HashSet<NodeId> = current.iter().copied().collect();
        let mut stack: Vec<NodeId> = previous
            .iter()
            .filter(|id| !current.contains(id))
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            if let Some(sent) = self.sent.remove(&id) {
                stack.extend(sent.children);
            }
        }
    }

    /// Builds the window root, which holds `roots` and fills the surface.
    fn window_node(&self, roots: Vec<NodeId>, (width, height): (f64, f64)) -> Node {
        let mut window = Node::new(AccessKitRole::Window);
        window.set_label(self.title.clone());
        window.set_bounds(AccessKitRect {
            x0: 0.0,
            y0: 0.0,
            x1: width,
            y1: height,
        });
        window.set_children(roots);
        window
    }
}

/// Returns a digest of everything [`build_node`] reads from `node` at
/// `scale`, which tells a node that changed from one that did not.
fn digest(node: &SemanticsNode, scale: f32) -> u64 {
    let mut hasher = DefaultHasher::new();
    node.id.as_u64().hash(&mut hasher);
    node.parent.map(WidgetId::as_u64).hash(&mut hasher);
    for child in &node.children {
        child.as_u64().hash(&mut hasher);
    }
    map_role(node.role).hash(&mut hasher);
    node.label.hash(&mut hasher);
    node.value.hash(&mut hasher);
    let bounds = node.bounds;
    for coordinate in [bounds.x0, bounds.y0, bounds.x1, bounds.y1, scale] {
        coordinate.to_bits().hash(&mut hasher);
    }
    (node.focusable, node.disabled, node.focused, node.pressable).hash(&mut hasher);
    hasher.finish()
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
        Role::MultilineTextInput => AccessKitRole::MultilineTextInput,
        Role::ScrollView => AccessKitRole::ScrollView,
        Role::List => AccessKitRole::List,
        Role::ListItem => AccessKitRole::ListItem,
        // A role added after this mapping reports as a plain group.
        _ => AccessKitRole::Group,
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
