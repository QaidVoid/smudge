//! Docking workspaces, layout values and the operations that edit them.

mod floating;
mod keyboard;
mod layout;
mod strip;
mod widget;
mod zone;

pub use layout::{Branch, DockError, DockLayout, DockNode, FloatingGroup, GroupPath, Zone};
pub use widget::{Dock, DockExt, DockPanel, dock};
