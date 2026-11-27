//! Dock workspaces, retained content, tab interaction and split resizing.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use smudge_core::geometry::{Point, Size};
use smudge_core::input::{InputEvent, Key};
use smudge_core::reactive::{Owner, RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Direction, pct, px};
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};
use smudge_widgets::dock::{Branch, DockExt, DockLayout, DockNode, DockPanel, Zone, dock};

type Counts = Rc<RefCell<HashMap<String, usize>>>;

#[derive(Clone, Default)]
struct Builds {
    factories: Counts,
    content: Counts,
    cleanup: Counts,
}

impl Builds {
    fn panel(&self, key: &str) -> DockPanel {
        *self
            .factories
            .borrow_mut()
            .entry(key.to_owned())
            .or_default() += 1;
        let key = key.to_owned();
        let title = key.clone();
        let content = Rc::clone(&self.content);
        let cleanup = Rc::clone(&self.cleanup);
        let value = RwSignal::new(0_u32);
        DockPanel::new(title, move || {
            *content.borrow_mut().entry(key.clone()).or_default() += 1;
            let cleaned = key.clone();
            Owner::on_cleanup(move || {
                *cleanup.borrow_mut().entry(cleaned).or_default() += 1;
            });
            let pressed = value.clone();
            panel()
                .background(ColorToken::SURFACE)
                .role(Role::Button)
                .accessible_label(format!("Content {key}"))
                .accessible_value(move || value.get().to_string())
                .focusable()
                .on_press(move || pressed.update(|value| *value += 1))
                .child(label(format!("{key} content")).padding(px(12.0)))
        })
    }

    fn assert_once(&self, keys: &[&str]) {
        for key in keys {
            assert_eq!(self.factories.borrow().get(*key), Some(&1), "factory {key}");
            assert_eq!(self.content.borrow().get(*key), Some(&1), "content {key}");
        }
        assert_eq!(self.factories.borrow().len(), keys.len());
        assert_eq!(self.content.borrow().len(), keys.len());
    }
}

fn group(keys: &[&str], active: &str) -> DockNode {
    DockNode::Group {
        panels: keys.iter().map(|key| (*key).to_owned()).collect(),
        active: active.to_owned(),
    }
}

fn layout(keys: &[&str], active: &str) -> DockLayout {
    DockLayout::new(group(keys, active)).expect("the group has unique keys")
}

fn nested_layout() -> DockLayout {
    DockLayout::new(DockNode::Split {
        direction: Direction::Row,
        ratio: 0.3,
        first: Box::new(group(&["Files"], "Files")),
        second: Box::new(DockNode::Split {
            direction: Direction::Column,
            ratio: 0.6,
            first: Box::new(group(&["Editor", "Search"], "Editor")),
            second: Box::new(group(&["Log"], "Log")),
        }),
    })
    .expect("the nested layout has unique keys")
}

fn window(layout: RwSignal<DockLayout>, builds: Builds) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(480.0, 240.0, 1.0, move || {
        dock(layout, move |key| builds.panel(key))
            .width(pct(100.0))
            .height(pct(100.0))
            .background(ColorToken::SURFACE)
    });
    harness.settle();
    harness
}

fn named(harness: &mut Harness, role: Role, name: &str) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == role && node.label.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the dock shows {role:?} named {name:?}"))
}

fn shows(harness: &mut Harness, role: Role, name: &str) -> bool {
    harness
        .app()
        .semantics_nodes()
        .iter()
        .any(|node| node.role == role && node.label.as_deref() == Some(name))
}

fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: (node.bounds.x0 + node.bounds.x1) * 0.5,
        y: (node.bounds.y0 + node.bounds.y1) * 0.5,
    }
}

fn press_tab(harness: &mut Harness, name: &str) {
    let tab = named(harness, Role::Tab, name);
    harness.click(Point {
        x: tab.bounds.x0 + 8.0,
        y: middle(&tab).y,
    });
    harness.settle();
}

fn first_ratio(layout: &RwSignal<DockLayout>) -> f32 {
    layout.read(|layout| match layout.root() {
        Some(DockNode::Split { ratio, .. }) => *ratio,
        _ => panic!("the layout has a root split"),
    })
}

#[test]
fn nested_splits_show_each_groups_active_panel_and_build_every_key() {
    let builds = Builds::default();
    let mut harness = window(RwSignal::new(nested_layout()), builds.clone());
    let nodes = harness.app().semantics_nodes();
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.role == Role::TabList)
            .count(),
        3
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.role == Role::Splitter)
            .count(),
        2
    );
    for key in ["Files", "Editor", "Log"] {
        let content = named(&mut harness, Role::TabPanel, key);
        assert!(content.bounds.width() > 0.0 && content.bounds.height() > 0.0);
    }
    assert!(!shows(&mut harness, Role::TabPanel, "Search"));
    builds.assert_once(&["Files", "Editor", "Search", "Log"]);
    harness.assert_matches_full_repaint();
}

#[test]
fn panel_titles_name_tabs_groups_and_close_requests_still_use_keys() {
    TextSystem::bundled_only().install();
    let model = RwSignal::new(layout(&["files", "editor"], "files"));
    let edits = model.clone();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&requests);
    let mut harness = Harness::new(480.0, 180.0, 1.0, move || {
        dock(edits, |key| {
            DockPanel::new(
                if key == "files" {
                    "Project files"
                } else {
                    "Source editor"
                },
                panel,
            )
        })
        .on_close(move |key| {
            log.borrow_mut().push(key.to_owned());
            true
        })
        .width(pct(100.0))
        .height(pct(100.0))
    });
    harness.settle();
    assert!(shows(&mut harness, Role::Tab, "Project files"));
    assert!(shows(&mut harness, Role::Tab, "Source editor"));
    assert!(shows(&mut harness, Role::Group, "Project files"));
    assert!(shows(&mut harness, Role::TabPanel, "Project files"));
    assert!(shows(&mut harness, Role::Button, "Close Project files"));
    assert!(!shows(&mut harness, Role::Tab, "files"));
    assert!(!shows(&mut harness, Role::Tab, "editor"));
    press_tab(&mut harness, "Source editor");
    assert!(shows(&mut harness, Role::Group, "Source editor"));
    assert!(shows(&mut harness, Role::TabPanel, "Source editor"));
    let close = named(&mut harness, Role::Button, "Close Source editor");
    harness.click(middle(&close));
    harness.settle();
    assert_eq!(*requests.borrow(), ["editor"]);
    assert_eq!(model.get(), layout(&["files"], "files"));
    assert!(shows(&mut harness, Role::Group, "Project files"));
    harness.assert_matches_full_repaint();
}

#[test]
fn tab_presses_and_arrows_activate_without_rebuilding_or_visiting_close_buttons() {
    let model = RwSignal::new(layout(&["One", "Two", "Three"], "One"));
    let builds = Builds::default();
    let mut harness = window(model.clone(), builds.clone());
    let original = named(&mut harness, Role::Tab, "One").id;
    press_tab(&mut harness, "Two");
    assert!(
        named(&mut harness, Role::Tab, "Two")
            .selected
            .unwrap_or(false)
    );
    assert!(shows(&mut harness, Role::TabPanel, "Two"));
    assert!(!shows(&mut harness, Role::TabPanel, "One"));

    harness.press_key(Key::ArrowRight);
    harness.settle();
    assert!(named(&mut harness, Role::Tab, "Three").focused);
    harness.press_key(Key::Home);
    harness.settle();
    assert!(named(&mut harness, Role::Tab, "One").focused);
    harness.press_key(Key::ArrowLeft);
    harness.settle();
    assert!(named(&mut harness, Role::Tab, "Three").focused);
    harness.press_key(Key::End);
    harness.settle();
    assert!(named(&mut harness, Role::Tab, "Three").focused);
    assert_eq!(named(&mut harness, Role::Tab, "One").id, original);
    model.read(|layout| {
        assert!(matches!(layout.root(), Some(DockNode::Group { active, .. }) if active == "Three"));
    });
    builds.assert_once(&["One", "Two", "Three"]);
    harness.assert_matches_full_repaint();
}

#[test]
fn tab_enters_the_strip_once_at_the_active_tab_then_moves_into_its_content() {
    let mut harness = window(
        RwSignal::new(layout(&["One", "Two"], "Two")),
        Builds::default(),
    );
    harness.press_key(Key::Tab);
    harness.settle();
    assert!(named(&mut harness, Role::Tab, "Two").focused);
    harness.press_key(Key::Tab);
    harness.settle();
    assert!(named(&mut harness, Role::Button, "Content Two").focused);
    harness.press_key_with(
        Key::Tab,
        smudge_core::input::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    harness.settle();
    assert!(named(&mut harness, Role::Tab, "Two").focused);
}

#[test]
fn moves_reorders_and_layout_replacements_keep_panel_state_and_focus() {
    let model = RwSignal::new(layout(&["One", "Two", "Three"], "One"));
    let builds = Builds::default();
    let mut harness = window(model.clone(), builds.clone());
    let content = named(&mut harness, Role::Button, "Content One");
    harness.click(middle(&content));
    harness.settle();
    assert_eq!(
        named(&mut harness, Role::Button, "Content One")
            .value
            .as_deref(),
        Some("1")
    );

    model.update(|layout| {
        layout
            .move_panel("One", &[], Zone::Right)
            .expect("the root is a group")
    });
    harness.settle();
    let moved = named(&mut harness, Role::Button, "Content One");
    assert_eq!(moved.id, content.id);
    assert!(moved.focused);
    assert_eq!(moved.value.as_deref(), Some("1"));
    model.update(|layout| {
        layout
            .move_tab("Three", 0)
            .expect("the third panel is present");
        layout
            .activate("Three")
            .expect("the third panel is present");
        layout.set_ratio(&[], 0.25).expect("the root is a split");
    });
    harness.settle();
    harness.assert_matches_full_repaint();
    model.update(|layout| {
        layout
            .move_panel("One", &[Branch::First], Zone::Center)
            .expect("the first pane is a group")
    });
    harness.settle();
    assert!(matches!(model.get().root(), Some(DockNode::Group { .. })));
    assert_eq!(
        named(&mut harness, Role::Button, "Content One").id,
        content.id
    );
    assert!(named(&mut harness, Role::Button, "Content One").focused);
    harness.assert_matches_full_repaint();

    model.set(
        DockLayout::new(DockNode::Split {
            direction: Direction::Column,
            ratio: 0.45,
            first: Box::new(group(&["One"], "One")),
            second: Box::new(group(&["Two", "Three"], "Two")),
        })
        .expect("all keys are unique"),
    );
    harness.settle();
    let replaced = named(&mut harness, Role::Button, "Content One");
    assert_eq!(replaced.id, content.id);
    assert!(replaced.focused);
    assert_eq!(replaced.value.as_deref(), Some("1"));
    harness.click(middle(&replaced));
    harness.settle();
    assert_eq!(
        named(&mut harness, Role::Button, "Content One")
            .value
            .as_deref(),
        Some("2")
    );
    builds.assert_once(&["One", "Two", "Three"]);
    assert!(builds.cleanup.borrow().is_empty());
    harness.assert_matches_full_repaint();
}

#[test]
fn a_held_divider_drag_and_external_ratio_changes_update_the_same_layout() {
    let model = RwSignal::new(nested_layout());
    let builds = Builds::default();
    let mut harness = window(model.clone(), builds.clone());
    let divider = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Splitter && node.bounds.height() > node.bounds.width())
        .expect("the root has a vertical divider");
    let start = middle(&divider);
    harness.button_down(start);
    harness.move_to(Point {
        x: start.x + 100.0,
        y: start.y,
    });
    harness.settle();
    let after = first_ratio(&model);
    assert!((after - (0.3 + 100.0 / 480.0)).abs() < 0.002);
    assert_eq!(harness.app().focused(), Some(divider.id));
    harness.move_to(Point {
        x: start.x + 140.0,
        y: start.y,
    });
    harness.settle();
    assert!((first_ratio(&model) - (0.3 + 140.0 / 480.0)).abs() < 0.002);
    harness.button_up(Point {
        x: start.x + 140.0,
        y: start.y,
    });
    model.update(|layout| layout.set_ratio(&[], 0.2).expect("the root is a split"));
    harness.settle();
    let updated = harness
        .app()
        .semantics_node(divider.id)
        .expect("the divider was retained");
    assert!((updated.bounds.x0 - 96.0).abs() < 0.1);
    assert_eq!(first_ratio(&model), 0.2);

    let nested = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Splitter && node.bounds.width() > node.bounds.height())
        .expect("the second pane has a horizontal divider");
    let start = middle(&nested);
    harness.button_down(start);
    harness.move_to(Point {
        x: start.x,
        y: start.y - 30.0,
    });
    harness.button_up(Point {
        x: start.x,
        y: start.y - 30.0,
    });
    harness.settle();
    model.read(|layout| {
        let Some(DockNode::Split { second, ratio, .. }) = layout.root() else {
            panic!("the root remains split")
        };
        assert_eq!(*ratio, 0.2);
        assert!(matches!(second.as_ref(), DockNode::Split { ratio, .. } if *ratio < 0.6));
    });
    builds.assert_once(&["Files", "Editor", "Search", "Log"]);
    harness.assert_matches_full_repaint();
}

#[test]
fn close_buttons_respect_veto_and_close_policy_without_activating_an_inactive_tab() {
    TextSystem::bundled_only().install();
    let model = RwSignal::new(layout(&["One", "Two", "Locked"], "One"));
    let edits = model.clone();
    let allowed = Rc::new(Cell::new(false));
    let can_close = Rc::clone(&allowed);
    let requests = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&requests);
    let builds = Builds::default();
    let count = builds.clone();
    let mut harness = Harness::new(480.0, 180.0, 1.0, move || {
        dock(edits, move |key| count.panel(key).closable(key != "Locked"))
            .on_close(move |key| {
                log.borrow_mut().push(key.to_owned());
                can_close.get()
            })
            .width(pct(100.0))
            .height(pct(100.0))
    });
    harness.settle();
    let one = named(&mut harness, Role::Button, "Content One");
    harness.click(middle(&one));
    harness.settle();
    assert!(!shows(&mut harness, Role::Button, "Close Locked"));
    let close = named(&mut harness, Role::Button, "Close Two");
    harness.click(middle(&close));
    harness.settle();
    assert!(shows(&mut harness, Role::Tab, "Two"));
    assert!(named(&mut harness, Role::Button, "Content One").focused);
    assert!(
        named(&mut harness, Role::Tab, "One")
            .selected
            .unwrap_or(false)
    );

    allowed.set(true);
    harness.click(middle(&close));
    harness.settle();
    assert!(!shows(&mut harness, Role::Tab, "Two"));
    assert!(named(&mut harness, Role::Button, "Content One").focused);
    assert_eq!(builds.cleanup.borrow().get("Two"), Some(&1));
    assert_eq!(*requests.borrow(), ["Two", "Two"]);
    model.update(|layout| {
        layout
            .close("Locked")
            .expect("application edits can remove nonclosable panels")
    });
    harness.settle();
    assert_eq!(requests.borrow().len(), 2);
    harness.assert_matches_full_repaint();
}

#[test]
fn closing_the_last_panel_disposes_it_and_reopening_builds_fresh_content() {
    let model = RwSignal::new(layout(&["One"], "One"));
    let builds = Builds::default();
    let mut harness = window(model.clone(), builds.clone());
    let old = named(&mut harness, Role::Button, "Content One").id;
    let close = named(&mut harness, Role::Button, "Close One");
    harness.click(middle(&close));
    harness.settle();
    assert!(model.get().root().is_none());
    assert!(!shows(&mut harness, Role::Tab, "One"));
    assert_eq!(builds.cleanup.borrow().get("One"), Some(&1));
    harness.assert_matches_full_repaint();
    model.update(|layout| layout.add("One", &[]).expect("the layout is empty"));
    harness.settle();
    assert_ne!(named(&mut harness, Role::Button, "Content One").id, old);
    assert_eq!(builds.factories.borrow().get("One"), Some(&2));
    assert_eq!(builds.content.borrow().get("One"), Some(&2));
    assert_eq!(
        named(&mut harness, Role::Button, "Content One")
            .value
            .as_deref(),
        Some("0")
    );
}

#[test]
fn closing_active_tabs_selects_neighbors_then_collapses_the_group_without_rebuilding() {
    let model = RwSignal::new(
        DockLayout::new(DockNode::Split {
            direction: Direction::Row,
            ratio: 0.5,
            first: Box::new(group(&["One", "Two", "Three"], "Two")),
            second: Box::new(group(&["Four"], "Four")),
        })
        .expect("the panel names are unique"),
    );
    let builds = Builds::default();
    let mut harness = window(model.clone(), builds.clone());
    let surviving = named(&mut harness, Role::Button, "Content Four").id;
    for (closed, next) in [("Two", "Three"), ("Three", "One"), ("One", "Four")] {
        let close = named(&mut harness, Role::Button, &format!("Close {closed}"));
        harness.click(middle(&close));
        harness.settle();
        assert!(!shows(&mut harness, Role::Tab, closed));
        assert!(
            named(&mut harness, Role::Tab, next)
                .selected
                .unwrap_or(false)
        );
        assert!(shows(&mut harness, Role::TabPanel, next));
        assert_eq!(builds.cleanup.borrow().get(closed), Some(&1));
        harness.assert_matches_full_repaint();
    }
    assert!(matches!(model.get().root(), Some(DockNode::Group { .. })));
    assert_eq!(
        named(&mut harness, Role::Button, "Content Four").id,
        surviving
    );
    builds.assert_once(&["One", "Two", "Three", "Four"]);
}

#[test]
fn a_dock_created_before_mounting_retains_its_panels_across_group_moves() {
    TextSystem::bundled_only().install();
    let model = RwSignal::new(layout(&["One", "Two"], "One"));
    let builds = Builds::default();
    let count = builds.clone();
    let workspace = dock(model.clone(), move |key| count.panel(key))
        .width(pct(100.0))
        .height(pct(100.0));
    let mut harness = Harness::new(480.0, 180.0, 1.0, move || workspace);
    harness.settle();
    let content = named(&mut harness, Role::Button, "Content One");
    harness.click(middle(&content));
    harness.settle();
    model.update(|layout| {
        layout
            .move_panel("One", &[], Zone::Right)
            .expect("the root is a group")
    });
    harness.settle();
    let moved = named(&mut harness, Role::Button, "Content One");
    assert_eq!(moved.id, content.id);
    assert!(moved.focused);
    harness.click(middle(&moved));
    harness.settle();
    assert_eq!(
        named(&mut harness, Role::Button, "Content One")
            .value
            .as_deref(),
        Some("2")
    );
    builds.assert_once(&["One", "Two"]);
    harness.assert_matches_full_repaint();
}

#[test]
fn custom_empty_content_mounts_only_while_the_layout_is_empty() {
    TextSystem::bundled_only().install();
    let model = RwSignal::new(DockLayout::default());
    let edits = model.clone();
    let empty_builds = Rc::new(Cell::new(0));
    let count = Rc::clone(&empty_builds);
    let mut harness = Harness::new(300.0, 140.0, 1.0, move || {
        dock(edits, |_| DockPanel::new("One", || label("One content")))
            .width(pct(100.0))
            .height(pct(100.0))
            .empty(move || {
                count.set(count.get() + 1);
                panel().role(Role::Group).accessible_label("Open a file")
            })
    });
    harness.settle();
    assert!(shows(&mut harness, Role::Group, "Open a file"));
    assert_eq!(empty_builds.get(), 1);
    model.update(|layout| layout.add("One", &[]).expect("the layout is empty"));
    harness.settle();
    assert!(!shows(&mut harness, Role::Group, "Open a file"));
    model.update(|layout| layout.close("One").expect("the panel is present"));
    harness.settle();
    assert!(shows(&mut harness, Role::Group, "Open a file"));
    assert_eq!(empty_builds.get(), 2);
    harness.assert_matches_full_repaint();
}

fn assert_tab_inside_its_strip(harness: &mut Harness, key: &str) {
    let tab = named(harness, Role::Tab, key);
    let nodes = harness.app().semantics_nodes();
    let mut parent = tab.parent;
    let strip = loop {
        let ancestor = nodes
            .iter()
            .find(|node| Some(node.id) == parent)
            .expect("the tab belongs to a scrollable strip");
        if ancestor.role == Role::ScrollView {
            break ancestor;
        }
        parent = ancestor.parent;
    };
    assert!(
        tab.bounds.x0 >= strip.bounds.x0 - 0.01 && tab.bounds.x1 <= strip.bounds.x1 + 0.01,
        "{key}: {:?}, strip bounds {:?}",
        tab.bounds,
        strip.bounds
    );
}

#[test]
fn initial_and_external_activation_reveal_overflowing_tabs_in_every_group_without_focus() {
    TextSystem::bundled_only().install();
    let keys = |prefix: &str| {
        (0..8)
            .map(|at| format!("{prefix} {at}"))
            .collect::<Vec<_>>()
    };
    let model = RwSignal::new(
        DockLayout::new(DockNode::Split {
            direction: Direction::Row,
            ratio: 0.5,
            first: Box::new(DockNode::Group {
                panels: keys("Left"),
                active: "Left 7".into(),
            }),
            second: Box::new(DockNode::Group {
                panels: keys("Right"),
                active: "Right 7".into(),
            }),
        })
        .expect("the panel names are unique"),
    );
    let edits = model.clone();
    let mut harness = Harness::new(400.0, 180.0, 1.0, move || {
        panel()
            .column()
            .child(
                dock(edits, |key| DockPanel::new(key, panel))
                    .width(pct(100.0))
                    .height(px(120.0))
                    .shrink(0.0),
            )
            .child(
                panel()
                    .height(px(40.0))
                    .focusable()
                    .role(Role::Button)
                    .accessible_label("Outside"),
            )
    });
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "Left 7");
    assert_tab_inside_its_strip(&mut harness, "Right 7");
    let outside = named(&mut harness, Role::Button, "Outside");
    harness.click(middle(&outside));
    harness.settle();
    model.update(|layout| {
        layout.activate("Left 0").expect("the tab exists");
        layout.activate("Right 0").expect("the tab exists");
    });
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "Left 0");
    assert_tab_inside_its_strip(&mut harness, "Right 0");
    assert_eq!(harness.app().focused(), Some(outside.id));
    model.update(|layout| {
        layout.activate("Left 7").expect("the tab exists");
        layout.activate("Right 7").expect("the tab exists");
    });
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "Left 7");
    assert_tab_inside_its_strip(&mut harness, "Right 7");
    harness.app().set_viewport(
        Size {
            width: 320.0,
            height: 180.0,
        },
        1.0,
    );
    harness.frame();
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "Left 7");
    assert_tab_inside_its_strip(&mut harness, "Right 7");
    assert_eq!(harness.app().focused(), Some(outside.id));
    harness.assert_matches_full_repaint();
}

#[test]
fn moving_a_group_and_activating_its_tab_in_one_edit_reveals_the_new_tab() {
    TextSystem::bundled_only().install();
    let model = RwSignal::new(
        DockLayout::new(DockNode::Split {
            direction: Direction::Row,
            ratio: 0.5,
            first: Box::new(group(&["Left"], "Left")),
            second: Box::new(group(
                &[
                    "Right 0", "Right 1", "Right 2", "Right 3", "Right 4", "Right 5", "Right 6",
                    "Right 7",
                ],
                "Right 7",
            )),
        })
        .expect("the panel names are unique"),
    );
    let edits = model.clone();
    let mut harness = Harness::new(400.0, 180.0, 1.0, move || {
        dock(edits, |key| DockPanel::new(key, panel))
            .width(pct(100.0))
            .height(pct(100.0))
    });
    harness.settle();
    for (ratio, active) in [(0.7, "Right 5"), (0.2, "Right 7"), (0.35, "Right 0")] {
        model.update(|layout| {
            layout.set_ratio(&[], ratio).expect("the root is split");
            layout.activate(active).expect("the right tab exists");
        });
        harness.settle();
        assert_tab_inside_its_strip(&mut harness, active);
        assert_eq!(harness.app().focused(), None);
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn a_tab_wider_than_its_strip_reveals_its_leading_edge() {
    let long = "A panel title that is much wider than its narrow tab strip";
    let model = RwSignal::new(layout(&["One", long, "Last"], long));
    TextSystem::bundled_only().install();
    let edits = model.clone();
    let mut harness = Harness::new(160.0, 140.0, 1.0, move || {
        dock(edits, |key| DockPanel::new(key, panel))
            .width(pct(100.0))
            .height(pct(100.0))
    });
    harness.settle();
    let assert_leading_edge = |harness: &mut Harness| {
        let tab = named(harness, Role::Tab, long);
        let strip = harness
            .app()
            .semantics_nodes()
            .into_iter()
            .find(|node| node.role == Role::ScrollView)
            .expect("the group has a scrollable strip");
        assert!(tab.bounds.width() > strip.bounds.width());
        assert!(
            (tab.bounds.x0 - strip.bounds.x0).abs() < 0.01,
            "tab {:?}, strip {:?}",
            tab.bounds,
            strip.bounds
        );
        assert!(tab.bounds.x1 > strip.bounds.x1);
    };
    assert_leading_edge(&mut harness);
    model.update(|layout| layout.activate("Last").expect("the last tab exists"));
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "Last");
    model.update(|layout| layout.activate(long).expect("the long tab exists"));
    harness.settle();
    assert_leading_edge(&mut harness);
    assert_eq!(harness.app().focused(), None);
    harness.assert_matches_full_repaint();
}

#[test]
fn manually_scrolling_a_strip_does_not_keep_snapping_back_to_the_active_tab() {
    let model = RwSignal::new(layout(&["One", "Two", "Three", "Four", "Five"], "Five"));
    let mut harness = window(model.clone(), Builds::default());
    harness.app().set_viewport(
        Size {
            width: 180.0,
            height: 240.0,
        },
        1.0,
    );
    harness.frame();
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "Five");
    let before = named(&mut harness, Role::Tab, "Five").bounds.x1;
    harness.move_to(Point { x: 50.0, y: 15.0 });
    harness.event(InputEvent::Wheel {
        delta: Point { x: -60.0, y: 0.0 },
    });
    harness.settle();
    harness.app().set_theme(Theme::dark());
    harness.frame();
    harness.settle();
    let after = named(&mut harness, Role::Tab, "Five").bounds.x1;
    assert!(after > before + 20.0, "before={before}, after={after}");
    model.update(|layout| layout.activate("One").expect("the first tab exists"));
    harness.settle();
    assert_tab_inside_its_strip(&mut harness, "One");
    harness.assert_matches_full_repaint();
}

fn themed(theme: Theme) -> Harness {
    let mut harness = window(RwSignal::new(nested_layout()), Builds::default());
    harness.app().set_theme(theme);
    harness.settle();
    harness
}

#[test]
fn a_nested_workspace_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", themed(Theme::light()).png());
}

#[test]
fn a_nested_workspace_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", themed(Theme::dark()).png());
}
