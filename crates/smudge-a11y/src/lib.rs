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
use smudge_core::semantics::SemanticsNode;
use smudge_core::semantics::{Checked, Politeness, Role, TextOffset, TextRun};
use smudge_core::tree::WidgetId;

use accesskit::{Live, Node, NodeId, Rect as AccessKitRect, Toggled};
use accesskit::{Role as AccessKitRole, TreeId, TreeInfo, TreeUpdate};

/// The node id of the synthesized window root.
///
/// A widget id encodes a slot-map version and index that both fit in 32
/// bits, with the null key alone using all ones for the index, so an id of
/// all ones can never collide with a widget.
const WINDOW_ROOT: NodeId = NodeId(u64::MAX);

/// Returns the node id numbered `serial` among the nodes the projection
/// makes up itself: the announcements and the text runs.
///
/// Its index is all ones, which no widget id has, and its version is
/// `serial`, which stops short of all ones so it never names
/// [`WINDOW_ROOT`].
fn synthetic_id(serial: u32) -> NodeId {
    NodeId((u64::from(serial % u32::MAX) << 32) | u64::from(u32::MAX))
}

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
    /// The announcement nodes the window root holds after its roots, until
    /// the next announcements replace them.
    announced: Vec<NodeId>,
    /// The ids of the text runs of each node that has any, in order, kept
    /// while the node lives so a reader's place in the text stays put.
    runs: HashMap<NodeId, Vec<NodeId>>,
    /// The ids of the links of each node that has any, by the link's index,
    /// kept while the node lives.
    links: HashMap<NodeId, Vec<NodeId>>,
    /// The node and the index of the link each link node stands for, which a
    /// press on it runs.
    link_targets: HashMap<NodeId, (WidgetId, usize)>,
    /// The serial of the next node the projection makes up.
    serial: u32,
}

/// What the projection last sent for one node.
struct Sent {
    /// The digest of the node.
    digest: u64,
    /// The digest of its text runs.
    runs: u64,
    /// The node's children.
    children: Vec<NodeId>,
}

impl Projection {
    /// Names the window root `title` from the next update on, which must be
    /// a full one for the name to reach assistive technology.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// Creates a projection for a window with `title`.
    pub fn new(title: impl Into<String>) -> Projection {
        Projection {
            title: title.into(),
            surface: (0.0, 0.0),
            sent: HashMap::new(),
            roots: Vec::new(),
            announced: Vec::new(),
            runs: HashMap::new(),
            links: HashMap::new(),
            link_targets: HashMap::new(),
            serial: 0,
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
    /// The announcements the app queued become at most two nodes under the
    /// window root, one per politeness, each named by its messages joined
    /// by line feeds in the order they were announced and live with its
    /// politeness, which a screen reader speaks as the node arrives. A
    /// screen reader reports the nodes one update adds in no set order, so
    /// one node per politeness keeps the messages in order. The nodes stay
    /// until the next announcements replace them, so a reader that asks for
    /// a node's name after the event still finds it.
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
        let mut changes = app.take_a11y_changes();
        let announcements = std::mem::take(&mut changes.announcements);
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
                let runs = runs_digest(&node, scale);
                let same = |sent: &Sent| (sent.digest, sent.runs);
                let before = self.sent.get(&id).map(same);
                if before == Some((digest, runs)) {
                    continue;
                }
                let children = node_ids(&node.children);
                let previous = self.sent.insert(
                    id,
                    Sent {
                        digest,
                        runs,
                        children: children.clone(),
                    },
                );
                if let Some(previous) = previous {
                    self.forget_dropped(&previous.children, &children);
                }
                let send_runs = before.is_none_or(|(_, sent)| sent != runs);
                self.push_node(&node, scale, send_runs, &mut nodes);
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
        if full {
            self.announced.clear();
        }
        let window_sent = update.nodes.iter().any(|(id, _)| *id == WINDOW_ROOT);
        let mut fresh = false;
        for politeness in [Politeness::Assertive, Politeness::Polite] {
            let mut messages = announcements
                .iter()
                .filter(|announcement| announcement.politeness == politeness)
                .map(|announcement| announcement.message.as_str());
            let Some(first) = messages.next() else {
                continue;
            };
            if !fresh {
                self.announced.clear();
                fresh = true;
            }
            let mut label = first.to_owned();
            for message in messages {
                label.push('\n');
                label.push_str(message);
            }
            let id = self.next_synthetic();
            let mut node = Node::new(AccessKitRole::Status);
            node.set_label(label);
            node.set_live(match politeness {
                Politeness::Polite => Live::Polite,
                Politeness::Assertive => Live::Assertive,
            });
            self.announced.push(id);
            update.nodes.push((id, node));
        }
        if fresh || (window_sent && !self.announced.is_empty()) {
            let mut children = self.roots.clone();
            children.extend_from_slice(&self.announced);
            update.nodes.retain(|(id, _)| *id != WINDOW_ROOT);
            update
                .nodes
                .push((WINDOW_ROOT, self.window_node(children, surface)));
        }
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
        let live: HashSet<NodeId> = projected
            .iter()
            .map(|node| NodeId(node.id.as_u64()))
            .collect();
        self.runs.retain(|id, _| live.contains(id));
        self.links.retain(|id, _| live.contains(id));
        let links = &self.links;
        self.link_targets.retain(|link, (owner, _)| {
            links
                .get(&NodeId(owner.as_u64()))
                .is_some_and(|ids| ids.contains(link))
        });
        for node in &projected {
            let id = NodeId(node.id.as_u64());
            let sent = Sent {
                digest: digest(node, scale),
                runs: runs_digest(node, scale),
                children: node_ids(&node.children),
            };
            self.sent.insert(id, sent);
            self.push_node(node, scale, true, &mut nodes);
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
            self.runs.remove(&id);
            for link in self.links.remove(&id).unwrap_or_default() {
                self.link_targets.remove(&link);
            }
            if let Some(sent) = self.sent.remove(&id) {
                stack.extend(sent.children);
            }
        }
    }

    /// Returns the node and the index of the link the link node `id` stands
    /// for, which [`App::press_link`] runs when a screen reader presses it,
    /// or `None` when `id` names no link.
    pub fn link_target(&self, id: NodeId) -> Option<(WidgetId, usize)> {
        self.link_targets.get(&id).copied()
    }

    /// Returns the next id of a node the projection makes up.
    fn next_synthetic(&mut self) -> NodeId {
        let id = synthetic_id(self.serial);
        self.serial = self.serial.wrapping_add(1);
        id
    }

    /// Appends the AccessKit node `node` projects to `nodes`, followed by a
    /// node for each of its text runs, which it holds ahead of its children,
    /// when `send_runs` says the runs changed since they were last sent.
    fn push_node(
        &mut self,
        node: &SemanticsNode,
        scale: f32,
        send_runs: bool,
        nodes: &mut Vec<(NodeId, Node)>,
    ) {
        let id = NodeId(node.id.as_u64());
        let mut ids = self.runs.remove(&id).unwrap_or_default();
        ids.truncate(node.text_runs.len());
        while ids.len() < node.text_runs.len() {
            let run = self.next_synthetic();
            ids.push(run);
        }
        let mut link_ids = self.links.remove(&id).unwrap_or_default();
        for gone in link_ids.iter().skip(node.links.len()) {
            self.link_targets.remove(gone);
        }
        link_ids.truncate(node.links.len());
        while link_ids.len() < node.links.len() {
            let link = self.next_synthetic();
            link_ids.push(link);
        }
        for (index, link) in link_ids.iter().enumerate() {
            self.link_targets.insert(*link, (node.id, index));
        }
        // A link holds the runs that overlap its range, which a run meets
        // whole unless the link ends inside a cluster, and takes their place
        // among the node's children.
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); node.links.len()];
        let mut children = Vec::with_capacity(ids.len() + node.children.len());
        for (index, (run, run_id)) in node.text_runs.iter().zip(&ids).enumerate() {
            let end = run.start + run.text.len();
            let link = node
                .links
                .iter()
                .position(|range| run.start < range.end && range.start < end);
            match link {
                Some(link) => {
                    if members[link].is_empty() {
                        children.push(link_ids[link]);
                    }
                    members[link].push(index);
                }
                None => children.push(*run_id),
            }
        }
        children.extend(node_ids(&node.children));
        let mut out = build_node(node, scale);
        if let Some(selection) = node.text_selection
            && !ids.is_empty()
        {
            out.set_text_selection(accesskit::TextSelection {
                anchor: text_position(&node.text_runs, &ids, selection.anchor),
                focus: text_position(&node.text_runs, &ids, selection.focus),
            });
        }
        if !ids.is_empty() {
            out.set_children(children);
        }
        nodes.push((id, out));
        for (link, members) in link_ids.iter().zip(&members) {
            if members.is_empty() {
                continue;
            }
            let mut out = build_link(&node.text_runs, &ids, members, scale);
            if node.disabled {
                out.set_disabled();
            }
            nodes.push((*link, out));
        }
        let sent_runs: &[TextRun] = if send_runs { &node.text_runs } else { &[] };
        for (index, (run, run_id)) in sent_runs.iter().zip(&ids).enumerate() {
            let mut out = build_run(run, scale);
            if run.continues_line && index > 0 {
                out.set_previous_on_line(ids[index - 1]);
            }
            if let Some(next) = node.text_runs.get(index + 1)
                && next.continues_line
            {
                out.set_next_on_line(ids[index + 1]);
            }
            nodes.push((*run_id, out));
        }
        if !ids.is_empty() {
            self.runs.insert(id, ids);
        }
        if !link_ids.is_empty() {
            self.links.insert(id, link_ids);
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
    node.checked.hash(&mut hasher);
    (node.position, node.set_size, node.level).hash(&mut hasher);
    node.text_selection.hash(&mut hasher);
    node.text_runs.len().hash(&mut hasher);
    for link in &node.links {
        (link.start, link.end).hash(&mut hasher);
    }
    if let Some(range) = node.range {
        for number in [range.value, range.min, range.max] {
            number.to_bits().hash(&mut hasher);
        }
    }
    let bounds = node.bounds;
    for coordinate in [bounds.x0, bounds.y0, bounds.x1, bounds.y1, scale] {
        coordinate.to_bits().hash(&mut hasher);
    }
    (node.focusable, node.disabled, node.focused, node.pressable).hash(&mut hasher);
    hasher.finish()
}

/// Returns a digest of the text runs of `node` at `scale`, which tells runs
/// that changed from runs that did not, so a move of the caret alone sends
/// the node without its runs.
fn runs_digest(node: &SemanticsNode, scale: f32) -> u64 {
    let mut hasher = DefaultHasher::new();
    scale.to_bits().hash(&mut hasher);
    for run in &node.text_runs {
        (run.start, run.right_to_left, run.continues_line).hash(&mut hasher);
        run.text.hash(&mut hasher);
        run.character_lengths.hash(&mut hasher);
        run.word_starts.hash(&mut hasher);
        for number in run
            .character_positions
            .iter()
            .chain(&run.character_widths)
            .chain([
                &run.bounds.x0,
                &run.bounds.y0,
                &run.bounds.x1,
                &run.bounds.y1,
            ])
        {
            number.to_bits().hash(&mut hasher);
        }
    }
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
    if let Some(checked) = node.checked {
        out.set_toggled(match checked {
            Checked::True => Toggled::True,
            Checked::False => Toggled::False,
            Checked::Mixed => Toggled::Mixed,
        });
    }
    // A range that is not finite or runs backwards is left out, which reads
    // as a control with no known value, such as a busy progress bar.
    if let Some(range) = node.range
        && range.min.is_finite()
        && range.max.is_finite()
        && range.min <= range.max
        && range.value.is_finite()
    {
        out.set_numeric_value(range.value.clamp(range.min, range.max));
        out.set_min_numeric_value(range.min);
        out.set_max_numeric_value(range.max);
    }
    if let Some(position) = node.position {
        out.set_position_in_set(position);
    }
    if let Some(size) = node.set_size {
        out.set_size_of_set(size);
    }
    // AccessKit counts heading levels from zero, and the projection from 1.
    if let Some(level) = node.level {
        out.set_level(level.saturating_sub(1));
    }
    if matches!(
        node.role,
        Role::CheckBox | Role::Switch | Role::MenuItemCheckBox
    ) && !node.pressable
    {
        out.set_read_only();
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

/// Returns where `offset` into the text `runs` hold lands: on the character
/// that holds its byte, or past the last character of the last run, which
/// is where a caret after all the text sits, in an empty last run when the
/// text ends in a line break. An offset that ends a wrapped line lands past
/// the last character of that line instead of on the first of the next.
/// `ids` names the runs, in the same order, and is not empty.
fn text_position(runs: &[TextRun], ids: &[NodeId], offset: TextOffset) -> accesskit::TextPosition {
    let byte = offset.byte;
    for (index, (run, id)) in runs.iter().zip(ids).enumerate() {
        let mut at = run.start;
        for (character, length) in run.character_lengths.iter().enumerate() {
            let end = at + usize::from(*length);
            if (at..end).contains(&byte) {
                let wrapped = index > 0 && !runs[index - 1].text.ends_with('\n');
                if offset.ends_line && byte == run.start && !run.continues_line && wrapped {
                    return accesskit::TextPosition {
                        node: ids[index - 1],
                        character_index: runs[index - 1].character_lengths.len(),
                    };
                }
                return accesskit::TextPosition {
                    node: *id,
                    character_index: character,
                };
            }
            at = end;
        }
    }
    let last = runs.len().min(ids.len()) - 1;
    accesskit::TextPosition {
        node: ids[last],
        character_index: runs[last].character_lengths.len(),
    }
}

/// Builds the link node that holds the runs of `runs` at `members`, named
/// by their text and pressed with a click. `ids` names the runs.
fn build_link(runs: &[TextRun], ids: &[NodeId], members: &[usize], scale: f32) -> Node {
    let mut out = Node::new(AccessKitRole::Link);
    let text: String = members
        .iter()
        .map(|index| runs[*index].text.as_str())
        .collect();
    out.set_label(text.trim_end_matches('\n').to_owned());
    let mut bounds: Option<Rect> = None;
    for index in members {
        let run = runs[*index].bounds;
        bounds = Some(bounds.map_or(run, |bounds| bounds.union(run)));
    }
    if let Some(bounds) = bounds {
        out.set_bounds(scaled_bounds(bounds, scale));
    }
    out.set_children(members.iter().map(|index| ids[*index]).collect::<Vec<_>>());
    out.add_action(accesskit::Action::Click);
    out
}

/// Maps one text run onto an AccessKit text run node, its positions and
/// widths scaled to physical pixels as its bounds are.
fn build_run(run: &TextRun, scale: f32) -> Node {
    let mut out = Node::new(AccessKitRole::TextRun);
    out.set_value(run.text.clone());
    out.set_character_lengths(run.character_lengths.clone());
    out.set_word_starts(run.word_starts.clone());
    let bounds = scaled_bounds(run.bounds, scale);
    // The positions count from the scaled bounds, which round outward, so
    // the characters land where the glyphs paint.
    let shift = if run.right_to_left {
        (bounds.x1 - f64::from(run.bounds.x1 * scale)) as f32
    } else {
        (f64::from(run.bounds.x0 * scale) - bounds.x0) as f32
    };
    out.set_character_positions(
        run.character_positions
            .iter()
            .map(|position| position * scale + shift)
            .collect::<Vec<f32>>(),
    );
    out.set_character_widths(
        run.character_widths
            .iter()
            .map(|width| width * scale)
            .collect::<Vec<f32>>(),
    );
    out.set_bounds(bounds);
    out.set_text_direction(if run.right_to_left {
        accesskit::TextDirection::RightToLeft
    } else {
        accesskit::TextDirection::LeftToRight
    });
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
    match app.focused().filter(|id| app.is_projected(*id)) {
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
        Role::Splitter => AccessKitRole::Splitter,
        Role::CheckBox => AccessKitRole::CheckBox,
        Role::Switch => AccessKitRole::Switch,
        Role::ProgressBar => AccessKitRole::ProgressIndicator,
        Role::Link => AccessKitRole::Link,
        Role::Heading => AccessKitRole::Heading,
        Role::MenuBar => AccessKitRole::MenuBar,
        Role::Menu => AccessKitRole::Menu,
        Role::MenuItem => AccessKitRole::MenuItem,
        Role::MenuItemCheckBox => AccessKitRole::MenuItemCheckBox,
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
