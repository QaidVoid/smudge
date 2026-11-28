use std::collections::HashSet;
use std::fmt;
use std::mem;

use smudge_core::geometry::Rect;
use smudge_core::style::Direction;

#[cfg(feature = "serde")]
#[path = "persistence.rs"]
mod persistence;

pub(super) const MIN_FLOAT_WIDTH: f32 = 160.0;
pub(super) const MIN_FLOAT_HEIGHT: f32 = 96.0;

/// One step through a split, starting at the layout's root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Branch {
    /// The left pane of a row or the top pane of a column.
    First,
    /// The right pane of a row or the bottom pane of a column.
    Second,
}

/// A path to a group or split, with an empty path naming the root.
/// Recompute paths after a change to the tree's structure.
pub type GroupPath = Vec<Branch>;

/// Where a panel lands relative to a group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Zone {
    /// Append the panel to the group's tabs.
    Center,
    /// Split off a group to the left.
    Left,
    /// Split off a group to the right.
    Right,
    /// Split off a group above.
    Top,
    /// Split off a group below.
    Bottom,
}

/// A split or tab group in a docking layout.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DockNode {
    /// Two panes with a divider between them.
    Split {
        /// A row lays out the panes left to right, and a column top to bottom.
        #[cfg_attr(feature = "serde", serde(with = "persistence::SavedDirection"))]
        direction: Direction,
        /// The first pane's share of the split, from zero to one.
        ratio: f32,
        /// The left or top pane.
        first: Box<DockNode>,
        /// The right or bottom pane.
        second: Box<DockNode>,
    },
    /// A group of panels with one visible at a time.
    Group {
        /// Panel keys in tab order.
        panels: Vec<String>,
        /// The key of the visible panel.
        active: String,
    },
}

/// Why a requested layout edit could not be applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockError {
    /// The panel key is absent from the layout.
    PanelNotFound,
    /// The path does not name a group.
    GroupNotFound,
    /// The path does not name a split.
    SplitNotFound,
    /// A panel key would appear more than once in the layout.
    DuplicatePanel,
    /// The rectangle's coordinates or resulting dimensions are not finite.
    InvalidFloatingRect,
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PanelNotFound => "the panel is absent from the layout",
            Self::GroupNotFound => "the path does not name a dock group",
            Self::SplitNotFound => "the path does not name a dock split",
            Self::DuplicatePanel => "panel keys must be unique in a dock layout",
            Self::InvalidFloatingRect => "a floating panel needs a finite rectangle",
        })
    }
}

impl std::error::Error for DockError {}

/// A floating group with a single tab, named by that panel's key.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingGroup {
    panel: String,
    #[cfg_attr(feature = "serde", serde(with = "persistence::SavedRect"))]
    rect: Rect,
}

impl FloatingGroup {
    /// Returns the key of the tab in this group.
    pub fn panel(&self) -> &str {
        &self.panel
    }

    /// Returns the requested rectangle in window logical pixels. The widget
    /// may shift it to keep its title grip reachable in the current window.
    pub fn rect(&self) -> Rect {
        self.rect
    }
}

/// An application-owned docking tree and its floating groups.
/// The default layout is empty.
///
/// Each panel key appears at most once. Every group has an active panel,
/// and every split has two panes. Edits that return an error leave the
/// layout as it was.
///
/// The `serde` feature lets the app save and restore layouts. Deserialization
/// normalizes the docked tree and floating sizes, and rejects duplicate keys
/// and nonfinite floating geometry. Use [`Self::reconcile`] before mounting
/// a saved layout to account for changes to the app's panel keys.
///
/// ```
/// use smudge_widgets::dock::{DockError, DockLayout, Zone};
///
/// let mut layout = DockLayout::default();
/// layout.add("editor", &[])?;
/// layout.add("outline", &[])?;
/// layout.move_panel("outline", &[], Zone::Right)?;
/// # Ok::<(), DockError>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "persistence::SavedLayout"))]
pub struct DockLayout {
    root: Option<DockNode>,
    floating: Vec<FloatingGroup>,
}

impl DockLayout {
    /// Builds a layout from a tree. Empty groups disappear and splits with
    /// a single surviving pane collapse. An absent active key is replaced
    /// by the first tab, and ratios are normalized as in [`Self::set_ratio`].
    ///
    /// # Errors
    ///
    /// Returns [`DockError::DuplicatePanel`] if a key appears more than once.
    ///
    /// ```
    /// use smudge_widgets::dock::{DockError, DockLayout, DockNode};
    ///
    /// let layout = DockLayout::new(DockNode::Group {
    ///     panels: vec!["files".into(), "search".into()],
    ///     active: "files".into(),
    /// })?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn new(root: DockNode) -> Result<Self, DockError> {
        if !root.unique_panels(&mut HashSet::new()) {
            return Err(DockError::DuplicatePanel);
        }
        let mut layout = Self {
            root: Some(root),
            ..Self::default()
        };
        layout.normalize();
        Ok(layout)
    }

    /// Returns the docked tree, or `None` when there are no docked panels.
    pub fn root(&self) -> Option<&DockNode> {
        self.root.as_ref()
    }

    /// Returns the floating groups in the order they floated. A pointer
    /// press may change their visual stacking without changing the layout.
    pub fn floating(&self) -> &[FloatingGroup] {
        &self.floating
    }

    pub(super) fn floating_rect(&self, panel: &str) -> Option<Rect> {
        self.floating
            .iter()
            .find(|group| group.panel == panel)
            .map(FloatingGroup::rect)
    }

    /// Returns whether `panel` appears in the layout.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("editor", &[])?;
    /// assert!(layout.contains("editor"));
    /// assert!(!layout.contains("outline"));
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn contains(&self, panel: &str) -> bool {
        self.root.as_ref().is_some_and(|root| root.contains(panel))
            || self.floating_rect(panel).is_some()
    }

    /// Moves `panel` to `group`. A center move appends and activates the tab.
    /// An edge move creates an equally sized group on that side. A center
    /// move within the same group and an edge move of that group's sole tab
    /// leave the layout as it was. A floating tab moves back into the
    /// docked tree. If that tree is empty, an empty path creates its first
    /// group regardless of zone.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::PanelNotFound`] if `panel` is absent or
    /// [`DockError::GroupNotFound`] if the destination path names no group.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout, Zone};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("editor", &[])?;
    /// # layout.add("files", &[])?;
    /// layout.move_panel("files", &[], Zone::Left)?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn move_panel(
        &mut self,
        panel: &str,
        group: &[Branch],
        zone: Zone,
    ) -> Result<(), DockError> {
        if self.root.is_none() && group.is_empty() {
            let moved = self.take_panel(panel)?;
            self.root = Some(single_panel(moved));
            return Ok(());
        }
        let Some(DockNode::Group { panels, .. }) = self.node_at_mut(group) else {
            return Err(DockError::GroupNotFound);
        };
        if (zone == Zone::Center || panels.len() == 1) && panels.iter().any(|key| key == panel) {
            return Ok(());
        }

        let moved = self.take_panel(panel)?;
        let Some(target @ DockNode::Group { .. }) = self.node_at_mut(group) else {
            return Err(DockError::GroupNotFound);
        };
        match (zone, target) {
            (Zone::Center, DockNode::Group { panels, active }) => {
                active.clone_from(&moved);
                panels.push(moved);
            }
            (zone, target) => {
                let direction = if matches!(zone, Zone::Left | Zone::Right) {
                    Direction::Row
                } else {
                    Direction::Column
                };
                let original = mem::replace(target, empty_group());
                let added = single_panel(moved);
                let (first, second) = if matches!(zone, Zone::Left | Zone::Top) {
                    (added, original)
                } else {
                    (original, added)
                };
                *target = DockNode::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
            }
        }
        self.normalize();
        Ok(())
    }

    /// Moves `panel` into its own floating group, or sets its rectangle if
    /// already floating. Rectangles smaller than 160 by 96 logical pixels
    /// expand to that minimum, keeping the top left corner.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::PanelNotFound`] for an absent panel or
    /// [`DockError::InvalidFloatingRect`] for a nonfinite rectangle.
    ///
    /// ```
    /// use smudge_core::geometry::Rect;
    /// use smudge_widgets::dock::{DockError, DockLayout};
    /// let mut layout = DockLayout::default();
    /// layout.add("files", &[])?;
    /// layout.float("files", Rect::new(20.0, 40.0, 280.0, 240.0))?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn float(&mut self, panel: &str, rect: Rect) -> Result<(), DockError> {
        if self.floating_rect(panel).is_some() {
            return self.move_floating(panel, rect);
        }
        let rect = floating_rect(rect)?;
        let panel = self.take_panel(panel)?;
        self.floating.push(FloatingGroup { panel, rect });
        self.normalize();
        Ok(())
    }

    /// Changes the rectangle of the floating group named by `panel`.
    /// Rectangles follow the same rules as in [`Self::float`].
    ///
    /// # Errors
    ///
    /// Returns [`DockError::GroupNotFound`] if the panel is not floating or
    /// [`DockError::InvalidFloatingRect`] for a nonfinite rectangle.
    ///
    /// ```
    /// # use smudge_core::geometry::Rect;
    /// # use smudge_widgets::dock::{DockError, DockLayout};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("files", &[])?;
    /// # layout.float("files", Rect::new(20.0, 40.0, 280.0, 240.0))?;
    /// layout.move_floating("files", Rect::new(80.0, 40.0, 340.0, 240.0))?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn move_floating(&mut self, panel: &str, rect: Rect) -> Result<(), DockError> {
        let group = self
            .floating
            .iter_mut()
            .find(|group| group.panel == panel)
            .ok_or(DockError::GroupNotFound)?;
        group.rect = floating_rect(rect)?;
        Ok(())
    }

    /// Moves a tab within its group, keeping the same active panel. `index`
    /// is its zero-based position after the move. An index past the end
    /// moves the tab to the end. A floating panel has only one tab, so
    /// reordering it leaves the layout unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::PanelNotFound`] if `panel` is absent.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("editor", &[])?;
    /// # layout.add("files", &[])?;
    /// layout.move_tab("files", 0)?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn move_tab(&mut self, panel: &str, index: usize) -> Result<(), DockError> {
        if self.floating_rect(panel).is_some() {
            return Ok(());
        }
        let (panels, _, from) = self
            .panel_group_mut(panel)
            .ok_or(DockError::PanelNotFound)?;
        let to = index.min(panels.len() - 1);
        if from != to {
            let panel = panels.remove(from);
            panels.insert(to, panel);
        }
        self.normalize();
        Ok(())
    }

    /// Shows `panel` in its group. A floating panel is already active in
    /// its group, so this does not change its stacking order.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::PanelNotFound`] if `panel` is absent.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("editor", &[])?;
    /// # layout.add("files", &[])?;
    /// layout.activate("editor")?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn activate(&mut self, panel: &str) -> Result<(), DockError> {
        if self.floating_rect(panel).is_some() {
            return Ok(());
        }
        let (panels, active, index) = self
            .panel_group_mut(panel)
            .ok_or(DockError::PanelNotFound)?;
        active.clone_from(&panels[index]);
        self.normalize();
        Ok(())
    }

    /// Closes `panel`. If it was active, the tab to its right takes its
    /// place, or the tab to its left if it was last. Closing the last tab
    /// removes its group from the tree.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::PanelNotFound`] if `panel` is absent.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("files", &[])?;
    /// layout.close("files")?;
    /// assert!(layout.root().is_none());
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn close(&mut self, panel: &str) -> Result<(), DockError> {
        self.take_panel(panel)?;
        self.normalize();
        Ok(())
    }

    /// Appends and activates a new panel in `group`. In an empty layout,
    /// the empty path creates a group at the root.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::DuplicatePanel`] if the key already exists or
    /// [`DockError::GroupNotFound`] if the path names no destination group.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout};
    /// let mut layout = DockLayout::default();
    /// layout.add("terminal", &[])?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn add(&mut self, panel: &str, group: &[Branch]) -> Result<(), DockError> {
        if self.contains(panel) {
            return Err(DockError::DuplicatePanel);
        }
        if self.root.is_none() && group.is_empty() {
            self.root = Some(single_panel(panel.to_owned()));
        } else if let Some(DockNode::Group { panels, active }) = self.node_at_mut(group) {
            let panel = panel.to_owned();
            active.clone_from(&panel);
            panels.push(panel);
        } else {
            return Err(DockError::GroupNotFound);
        }
        self.normalize();
        Ok(())
    }

    /// Sets the first pane's share of a split. Ratios outside `[0, 1]`
    /// are clamped. A `NaN` gives each pane half.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::SplitNotFound`] if the path names no split.
    ///
    /// ```
    /// # use smudge_widgets::dock::{DockError, DockLayout, Zone};
    /// # let mut layout = DockLayout::default();
    /// # layout.add("editor", &[])?;
    /// # layout.add("files", &[])?;
    /// # layout.move_panel("files", &[], Zone::Left)?;
    /// layout.set_ratio(&[], 0.25)?;
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn set_ratio(&mut self, split: &[Branch], ratio: f32) -> Result<(), DockError> {
        let Some(DockNode::Split { ratio: share, .. }) = self.node_at_mut(split) else {
            return Err(DockError::SplitNotFound);
        };
        *share = ratio;
        self.normalize();
        Ok(())
    }

    /// Reconciles a saved layout with the panel keys the app wants open.
    /// Removes keys absent from `known`, including floating panels. Appends
    /// missing keys in the order given, ignoring repeated keys and keeping
    /// any surviving active tab. To keep a panel closed, omit it from `known`.
    ///
    /// `fallback` names a group in the tree before this edit. If it does not
    /// name a group, missing panels go to the first surviving group in tree
    /// order or form a new root group if none survives. Groups left empty
    /// disappear. Floating rectangles stay in window logical pixels. When
    /// the dock mounts, it keeps the floating grip reachable in the current
    /// window without changing the saved rectangle.
    ///
    /// ```
    /// use smudge_widgets::dock::{DockError, DockLayout};
    ///
    /// let mut layout = DockLayout::default();
    /// layout.add("old-panel", &[])?;
    /// layout.reconcile(&["editor", "outline"], &[]);
    /// assert!(!layout.contains("old-panel"));
    /// assert!(layout.contains("outline"));
    /// # Ok::<(), DockError>(())
    /// ```
    pub fn reconcile(&mut self, known: &[&str], fallback: &[Branch]) {
        let mut missing: HashSet<&str> = known.iter().copied().collect();
        if let Some(root) = self.root.as_mut() {
            root.retain_panels(&mut missing);
        }
        self.floating
            .retain(|group| missing.remove(group.panel.as_str()));

        if !missing.is_empty() {
            let panels = match self.node_at_mut(fallback) {
                Some(DockNode::Group { panels, .. }) => panels,
                _ => {
                    self.normalize();
                    self.root
                        .get_or_insert_with(empty_group)
                        .first_group_panels()
                }
            };
            panels.extend(
                known
                    .iter()
                    .copied()
                    .filter(|key| missing.remove(key))
                    .map(str::to_owned),
            );
        }
        self.normalize();
    }

    fn take_panel(&mut self, panel: &str) -> Result<String, DockError> {
        if let Some(at) = self.floating.iter().position(|group| group.panel == panel) {
            return Ok(self.floating.remove(at).panel);
        }
        let (panels, active, index) = self
            .panel_group_mut(panel)
            .ok_or(DockError::PanelNotFound)?;
        Ok(remove_panel(panels, active, index))
    }

    fn node_at_mut(&mut self, path: &[Branch]) -> Option<&mut DockNode> {
        let mut node = self.root.as_mut()?;
        for branch in path {
            let DockNode::Split { first, second, .. } = node else {
                return None;
            };
            node = match branch {
                Branch::First => first,
                Branch::Second => second,
            };
        }
        Some(node)
    }

    fn panel_group_mut(&mut self, panel: &str) -> Option<(&mut Vec<String>, &mut String, usize)> {
        self.root.as_mut()?.panel_group_mut(panel)
    }

    fn normalize(&mut self) {
        if self.root.as_mut().is_some_and(|root| !root.normalize()) {
            self.root = None;
        }
    }
}

impl DockNode {
    fn retain_panels(&mut self, missing: &mut HashSet<&str>) {
        match self {
            Self::Group { panels, .. } => panels.retain(|key| missing.remove(key.as_str())),
            Self::Split { first, second, .. } => {
                first.retain_panels(missing);
                second.retain_panels(missing);
            }
        }
    }

    fn first_group_panels(&mut self) -> &mut Vec<String> {
        match self {
            Self::Group { panels, .. } => panels,
            Self::Split { first, .. } => first.first_group_panels(),
        }
    }

    fn panel_group_mut(&mut self, panel: &str) -> Option<(&mut Vec<String>, &mut String, usize)> {
        match self {
            Self::Group { panels, active } => {
                let index = panels.iter().position(|key| key == panel)?;
                Some((panels, active, index))
            }
            Self::Split { first, second, .. } => first
                .panel_group_mut(panel)
                .or_else(|| second.panel_group_mut(panel)),
        }
    }

    fn contains(&self, panel: &str) -> bool {
        match self {
            Self::Group { panels, .. } => panels.iter().any(|key| key == panel),
            Self::Split { first, second, .. } => first.contains(panel) || second.contains(panel),
        }
    }

    fn unique_panels<'a>(&'a self, seen: &mut HashSet<&'a str>) -> bool {
        match self {
            Self::Group { panels, .. } => panels.iter().all(|key| seen.insert(key)),
            Self::Split { first, second, .. } => {
                first.unique_panels(seen) && second.unique_panels(seen)
            }
        }
    }

    fn normalize(&mut self) -> bool {
        match self {
            Self::Group { panels, active } => {
                let Some(first) = panels.first() else {
                    return false;
                };
                if !panels.contains(active) {
                    active.clone_from(first);
                }
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => match (first.normalize(), second.normalize()) {
                (true, true) => {
                    *ratio = if ratio.is_nan() {
                        0.5
                    } else {
                        ratio.clamp(0.0, 1.0)
                    };
                }
                (true, false) => *self = mem::replace(first.as_mut(), empty_group()),
                (false, true) => *self = mem::replace(second.as_mut(), empty_group()),
                (false, false) => return false,
            },
        }
        true
    }
}

fn floating_rect(rect: Rect) -> Result<Rect, DockError> {
    Ok(Rect::new(
        rect.x0,
        rect.y0,
        min_end(rect.x0, rect.x1, MIN_FLOAT_WIDTH)?,
        min_end(rect.y0, rect.y1, MIN_FLOAT_HEIGHT)?,
    ))
}

fn min_end(start: f32, end: f32, min: f32) -> Result<f32, DockError> {
    if !start.is_finite() || !end.is_finite() {
        return Err(DockError::InvalidFloatingRect);
    }
    let mut end = end.max(start + min);
    if end - start < min {
        end = end.next_up();
    }
    if !(end - start).is_finite() {
        return Err(DockError::InvalidFloatingRect);
    }
    Ok(end)
}

fn empty_group() -> DockNode {
    DockNode::Group {
        panels: Vec::new(),
        active: String::new(),
    }
}

fn single_panel(panel: String) -> DockNode {
    DockNode::Group {
        active: panel.clone(),
        panels: vec![panel],
    }
}

fn remove_panel(panels: &mut Vec<String>, active: &mut String, index: usize) -> String {
    let removed = panels.remove(index);
    if *active == removed
        && let Some(next) = panels.get(index).or_else(|| panels.last())
    {
        active.clone_from(next);
    }
    removed
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
