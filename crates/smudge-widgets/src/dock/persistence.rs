use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use smudge_core::geometry::Rect;
use smudge_core::style::Direction;

use super::{DockError, DockLayout, DockNode, FloatingGroup, floating_rect};

#[derive(Deserialize)]
#[serde(rename = "DockLayout")]
pub(super) struct SavedLayout {
    root: Option<DockNode>,
    floating: Vec<FloatingGroup>,
}

impl TryFrom<SavedLayout> for DockLayout {
    type Error = DockError;

    fn try_from(saved: SavedLayout) -> Result<Self, Self::Error> {
        let mut layout = Self {
            root: saved.root,
            floating: saved.floating,
        };
        let mut seen = HashSet::new();
        if !layout.root.iter().all(|node| node.unique_panels(&mut seen))
            || !layout
                .floating
                .iter()
                .all(|group| seen.insert(&group.panel))
        {
            return Err(DockError::DuplicatePanel);
        }
        for group in &mut layout.floating {
            group.rect = floating_rect(group.rect)?;
        }
        layout.normalize();
        Ok(layout)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Direction")]
pub(super) enum SavedDirection {
    Row,
    Column,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Rect")]
pub(super) struct SavedRect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}
