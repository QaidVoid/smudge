use proptest::prelude::*;

use super::*;

fn group(panels: &[&str], active: &str) -> DockNode {
    DockNode::Group {
        panels: panels.iter().map(|key| (*key).to_owned()).collect(),
        active: active.to_owned(),
    }
}

fn split(direction: Direction, ratio: f32, first: DockNode, second: DockNode) -> DockNode {
    DockNode::Split {
        direction,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

#[test]
fn a_center_move_appends_and_activates_the_tab_and_collapses_the_source() -> Result<(), DockError> {
    let mut layout = DockLayout::new(split(
        Direction::Row,
        0.35,
        group(&["editor"], "editor"),
        group(&["files", "search"], "files"),
    ))?;

    layout.move_panel("editor", &[Branch::Second], Zone::Center)?;

    assert_eq!(
        layout.root(),
        Some(&group(&["files", "search", "editor"], "editor"))
    );
    Ok(())
}

#[test]
fn each_edge_splits_the_group_in_the_drop_direction() -> Result<(), DockError> {
    for (zone, direction, new_first) in [
        (Zone::Left, Direction::Row, true),
        (Zone::Right, Direction::Row, false),
        (Zone::Top, Direction::Column, true),
        (Zone::Bottom, Direction::Column, false),
    ] {
        let mut layout = DockLayout::new(group(&["editor", "files", "outline"], "outline"))?;
        layout.move_panel("outline", &[], zone)?;

        let original = group(&["editor", "files"], "files");
        let added = group(&["outline"], "outline");
        let (first, second) = if new_first {
            (added, original)
        } else {
            (original, added)
        };
        assert_eq!(
            layout.root(),
            Some(&split(direction, 0.5, first, second)),
            "{zone:?}"
        );
    }
    Ok(())
}

#[test]
fn moving_to_ones_own_center_or_splitting_a_sole_tab_changes_nothing() -> Result<(), DockError> {
    let mut layout = DockLayout::new(split(
        Direction::Column,
        0.4,
        group(&["files", "search"], "search"),
        group(&["editor"], "editor"),
    ))?;
    let original = layout.clone();

    layout.move_panel("files", &[Branch::First], Zone::Center)?;
    assert_eq!(layout, original);
    layout.move_panel("editor", &[Branch::Second], Zone::Left)?;
    assert_eq!(layout, original);
    Ok(())
}

#[test]
fn moving_between_nested_groups_keeps_the_destination_after_source_collapse()
-> Result<(), DockError> {
    let mut layout = DockLayout::new(split(
        Direction::Row,
        0.45,
        group(&["editor"], "editor"),
        split(
            Direction::Column,
            0.65,
            group(&["files"], "files"),
            group(&["terminal"], "terminal"),
        ),
    ))?;

    layout.move_panel("editor", &[Branch::Second, Branch::First], Zone::Bottom)?;

    assert_eq!(
        layout.root(),
        Some(&split(
            Direction::Column,
            0.65,
            split(
                Direction::Column,
                0.5,
                group(&["files"], "files"),
                group(&["editor"], "editor")
            ),
            group(&["terminal"], "terminal"),
        ))
    );
    Ok(())
}

#[test]
fn closing_the_active_tab_picks_the_right_neighbor_then_the_left() -> Result<(), DockError> {
    let mut layout = DockLayout::new(group(&["files", "editor", "outline"], "editor"))?;

    layout.close("editor")?;
    assert_eq!(
        layout.root(),
        Some(&group(&["files", "outline"], "outline"))
    );
    layout.close("outline")?;
    assert_eq!(layout.root(), Some(&group(&["files"], "files")));
    layout.close("files")?;
    assert!(layout.root().is_none());
    Ok(())
}

#[test]
fn reordering_keeps_the_active_tab_and_clamps_the_final_index() -> Result<(), DockError> {
    let mut layout = DockLayout::default();
    layout.add("files", &[])?;
    layout.add("editor", &[])?;
    layout.add("outline", &[])?;
    assert_eq!(
        layout.root(),
        Some(&group(&["files", "editor", "outline"], "outline"))
    );

    layout.activate("editor")?;
    layout.move_tab("editor", usize::MAX)?;
    assert_eq!(
        layout.root(),
        Some(&group(&["files", "outline", "editor"], "editor"))
    );
    layout.move_tab("outline", 0)?;
    assert_eq!(
        layout.root(),
        Some(&group(&["outline", "files", "editor"], "editor"))
    );
    layout.close("files")?;
    assert_eq!(
        layout.root(),
        Some(&group(&["outline", "editor"], "editor"))
    );
    Ok(())
}

#[test]
fn an_imported_tree_repairs_empty_groups_and_missing_active_tabs() -> Result<(), DockError> {
    let layout = DockLayout::new(split(
        Direction::Column,
        f32::NAN,
        split(
            Direction::Row,
            0.2,
            group(&[], ""),
            group(&["files", "search"], "gone"),
        ),
        group(&["editor"], "editor"),
    ))?;

    assert_eq!(
        layout.root(),
        Some(&split(
            Direction::Column,
            0.5,
            group(&["files", "search"], "files"),
            group(&["editor"], "editor"),
        ))
    );
    Ok(())
}

#[test]
fn a_split_ratio_stays_in_range_for_any_float() -> Result<(), DockError> {
    let first = group(&["files"], "files");
    let second = group(&["editor"], "editor");
    let mut layout = DockLayout::new(split(Direction::Row, 0.5, first.clone(), second.clone()))?;

    for (requested, ratio) in [
        (f32::NEG_INFINITY, 0.0),
        (f32::INFINITY, 1.0),
        (0.25, 0.25),
        (f32::NAN, 0.5),
    ] {
        layout.set_ratio(&[], requested)?;
        assert_eq!(
            layout.root(),
            Some(&split(Direction::Row, ratio, first.clone(), second.clone()))
        );
    }
    Ok(())
}

#[test]
fn stale_requests_and_duplicate_keys_leave_the_layout_untouched() -> Result<(), DockError> {
    let duplicate = split(
        Direction::Row,
        0.5,
        group(&["editor"], "editor"),
        group(&["editor"], "editor"),
    );
    assert_eq!(DockLayout::new(duplicate), Err(DockError::DuplicatePanel));

    let file_path = &[Branch::First];
    let mut layout = DockLayout::new(split(
        Direction::Row,
        0.35,
        group(&["files"], "files"),
        group(&["editor"], "editor"),
    ))?;
    let original = layout.clone();

    assert_eq!(
        layout.move_panel("gone", file_path, Zone::Top),
        Err(DockError::PanelNotFound)
    );
    assert_eq!(
        layout.move_panel("files", &[], Zone::Center),
        Err(DockError::GroupNotFound)
    );
    assert_eq!(
        layout.move_panel("files", &[Branch::Second, Branch::First], Zone::Top),
        Err(DockError::GroupNotFound)
    );
    assert_eq!(layout.move_tab("gone", 0), Err(DockError::PanelNotFound));
    assert_eq!(layout.activate("gone"), Err(DockError::PanelNotFound));
    assert_eq!(layout.close("gone"), Err(DockError::PanelNotFound));
    assert_eq!(
        layout.add("editor", file_path),
        Err(DockError::DuplicatePanel)
    );
    assert_eq!(layout.add("search", &[]), Err(DockError::GroupNotFound));
    assert_eq!(
        layout.set_ratio(file_path, 0.2),
        Err(DockError::SplitNotFound)
    );
    assert_eq!(layout, original);
    Ok(())
}

#[derive(Debug)]
enum Edit {
    Add(u8, usize),
    Move(u8, usize, Zone),
    Close(u8),
    Activate(u8),
    Reorder(u8, usize),
    Ratio(usize, f32),
}

fn edit() -> impl Strategy<Value = Edit> {
    let key = 0u8..12;
    let position = 0usize..16;
    prop_oneof![
        3 => (key.clone(), position.clone()).prop_map(|(key, target)| Edit::Add(key, target)),
        4 => (key.clone(), position.clone(), prop::sample::select(vec![Zone::Center, Zone::Left, Zone::Right, Zone::Top, Zone::Bottom]))
            .prop_map(|(key, target, zone)| Edit::Move(key, target, zone)),
        2 => key.clone().prop_map(Edit::Close),
        1 => key.clone().prop_map(Edit::Activate),
        1 => (key, position.clone()).prop_map(|(key, index)| Edit::Reorder(key, index)),
        2 => (position, any::<f32>()).prop_map(|(target, ratio)| Edit::Ratio(target, ratio)),
    ]
}

#[derive(Default)]
struct TreeView {
    groups: Vec<GroupPath>,
    splits: Vec<GroupPath>,
    keys: HashSet<String>,
}

impl TreeView {
    fn inspect(layout: &DockLayout) -> Result<Self, TestCaseError> {
        let mut view = Self::default();
        if let Some(root) = layout.root() {
            view.visit(root, &mut Vec::new())?;
        }
        Ok(view)
    }

    fn visit(&mut self, node: &DockNode, path: &mut GroupPath) -> Result<(), TestCaseError> {
        match node {
            DockNode::Group { panels, active } => {
                prop_assert!(!panels.is_empty());
                prop_assert!(panels.contains(active));
                for key in panels {
                    prop_assert!(self.keys.insert(key.clone()), "duplicate panel {key}");
                }
                self.groups.push(path.clone());
            }
            DockNode::Split {
                ratio,
                first,
                second,
                ..
            } => {
                prop_assert!((0.0..=1.0).contains(ratio));
                self.splits.push(path.clone());
                for (branch, child) in [(Branch::First, first), (Branch::Second, second)] {
                    path.push(branch);
                    self.visit(child, path)?;
                    path.pop();
                }
            }
        }
        Ok(())
    }
}

fn path_for(paths: &[GroupPath], index: usize) -> &[Branch] {
    paths
        .get(index % (paths.len() + 1))
        .map_or(&[Branch::First; 16], Vec::as_slice)
}

proptest! {
    #[test]
    fn arbitrary_edits_preserve_the_normal_tree_and_the_expected_panel_keys(
        edits in prop::collection::vec(edit(), 1..100),
    ) {
        let mut layout = DockLayout::default();
        let mut expected = HashSet::new();

        for edit in edits {
            let before = layout.clone();
            let view = TreeView::inspect(&layout)?;
            let result = match edit {
                Edit::Add(key, target) => {
                    let key = key.to_string();
                    let target = if view.groups.is_empty() && target % 2 == 0 {
                        &[]
                    } else {
                        path_for(&view.groups, target)
                    };
                    let result = layout.add(&key, target);
                    if result.is_ok() {
                        prop_assert!(expected.insert(key));
                    }
                    result
                }
                Edit::Move(key, target, zone) => layout.move_panel(&key.to_string(), path_for(&view.groups, target), zone),
                Edit::Close(key) => {
                    let key = key.to_string();
                    let result = layout.close(&key);
                    if result.is_ok() {
                        prop_assert!(expected.remove(&key));
                    }
                    result
                }
                Edit::Activate(key) => layout.activate(&key.to_string()),
                Edit::Reorder(key, index) => layout.move_tab(&key.to_string(), index),
                Edit::Ratio(target, ratio) => layout.set_ratio(path_for(&view.splits, target), ratio),
            };

            if result.is_err() {
                prop_assert_eq!(&layout, &before, "{edit:?} changed the layout despite returning {result:?}", edit = edit, result = result);
            }
            prop_assert_eq!(&TreeView::inspect(&layout)?.keys, &expected);
            if let Some(root) = layout.root() {
                let normalized = DockLayout::new(root.clone());
                prop_assert_eq!(normalized.as_ref(), Ok(&layout));
            }
        }
    }
}
