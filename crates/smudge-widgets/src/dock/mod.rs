//! Docking workspaces, layout values and the operations that edit them.

mod layout;
mod strip;
mod widget;

pub use layout::{Branch, DockError, DockLayout, DockNode, GroupPath, Zone};
pub use widget::{Dock, DockExt, DockPanel, dock};
