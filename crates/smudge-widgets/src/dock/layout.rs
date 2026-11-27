use std::collections::HashSet;
use std::fmt;
use std::mem;

use smudge_core::style::Direction;

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
pub enum DockNode {
    /// Two panes with a divider between them.
    Split {
        /// A row lays out the panes left to right, and a column top to bottom.
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
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PanelNotFound => "the panel is absent from the layout",
            Self::GroupNotFound => "the path does not name a dock group",
            Self::SplitNotFound => "the path does not name a dock split",
            Self::DuplicatePanel => "panel keys must be unique in a dock layout",
        })
    }
}

impl std::error::Error for DockError {}

/// An application-owned docking tree. The default layout is empty.
///
/// Each panel key appears at most once. Every group has an active panel,
/// and every split has two panes. Edits that return an error leave the
/// layout as it was.
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
pub struct DockLayout {
    root: Option<DockNode>,
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
        let mut layout = Self { root: Some(root) };
        layout.normalize();
        Ok(layout)
    }

    /// Returns the root of the tree, or `None` for an empty layout.
    pub fn root(&self) -> Option<&DockNode> {
        self.root.as_ref()
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
    }

    /// Moves `panel` to `group`. A center move appends and activates the tab.
    /// An edge move creates an equally sized group on that side. A center
    /// move within the same group and an edge move of that group's sole tab
    /// leave the layout as it was.
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
        let Some(DockNode::Group { panels, .. }) = self.node_at_mut(group) else {
            return Err(DockError::GroupNotFound);
        };
        if (zone == Zone::Center || panels.len() == 1) && panels.iter().any(|key| key == panel) {
            return Ok(());
        }

        let (panels, active, index) = self
            .panel_group_mut(panel)
            .ok_or(DockError::PanelNotFound)?;
        let moved = remove_panel(panels, active, index);
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

    /// Moves a tab within its group, keeping the same active panel. `index`
    /// is its zero-based position after the move. An index past the end
    /// moves the tab to the end.
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

    /// Shows `panel` in its group.
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
        let (panels, active, index) = self
            .panel_group_mut(panel)
            .ok_or(DockError::PanelNotFound)?;
        remove_panel(panels, active, index);
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
        if self.panel_group_mut(panel).is_some() {
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
