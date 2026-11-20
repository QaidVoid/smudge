//! Tabs, by pointer and by keyboard.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::geometry::Point;
use smudge_core::input::Key;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};
use smudge_widgets::tabs;

/// Returns the projected node named `name` with `role`, if it shows.
fn named(harness: &mut Harness, role: Role, name: &str) -> Option<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == role && node.label.as_deref() == Some(name))
}

/// A window of tabs named One, Two and Three, each panel a label of its
/// name, counting how many times a panel was built.
fn scene(selected: RwSignal<usize>, builds: Rc<Cell<u32>>) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let built = |name: &'static str| {
            let builds = Rc::clone(&builds);
            move || {
                builds.set(builds.get() + 1);
                label(format!("{name} panel")).height(px(40.0))
            }
        };
        panel().align_items(Align::Start).child(
            tabs(selected.clone())
                .tab("One", built("One"))
                .tab("Two", built("Two"))
                .tab("Three", built("Three")),
        )
    });
    harness.frame();
    harness
}

/// Clicks the middle of the tab named `name`.
fn press_tab(harness: &mut Harness, name: &str) {
    let bounds = named(harness, Role::Tab, name)
        .unwrap_or_else(|| panic!("the window shows the {name} tab"))
        .bounds;
    harness.click(Point {
        x: (bounds.x0 + bounds.x1) / 2.0,
        y: (bounds.y0 + bounds.y1) / 2.0,
    });
    harness.frame();
}

#[test]
fn a_press_on_a_tab_shows_its_panel_alone() {
    let selected = RwSignal::new(0);
    let mut harness = scene(selected.clone(), Rc::new(Cell::new(0)));
    assert!(named(&mut harness, Role::TabList, "One").is_none());
    assert_eq!(
        named(&mut harness, Role::Tab, "One").and_then(|tab| tab.selected),
        Some(true)
    );
    assert!(named(&mut harness, Role::TabPanel, "One").is_some());
    assert!(
        named(&mut harness, Role::TabPanel, "Two").is_none(),
        "hidden"
    );

    press_tab(&mut harness, "Two");

    assert_eq!(selected.get(), 1);
    assert_eq!(
        named(&mut harness, Role::Tab, "Two").and_then(|tab| tab.selected),
        Some(true)
    );
    assert_eq!(
        named(&mut harness, Role::Tab, "One").and_then(|tab| tab.selected),
        Some(false)
    );
    assert!(named(&mut harness, Role::TabPanel, "Two").is_some());
    assert!(named(&mut harness, Role::TabPanel, "One").is_none());
    harness.assert_matches_full_repaint();
}

#[test]
fn the_arrows_move_along_the_tabs_and_show_each_panel() {
    let selected = RwSignal::new(0);
    let mut harness = scene(selected.clone(), Rc::new(Cell::new(0)));
    press_tab(&mut harness, "One");

    harness.press_key(Key::ArrowRight);
    assert_eq!(selected.get(), 1);
    harness.press_key(Key::End);
    assert_eq!(selected.get(), 2);
    harness.press_key(Key::ArrowRight);
    assert_eq!(selected.get(), 0, "wrapping to the first");
    harness.frame();
    assert!(
        named(&mut harness, Role::Tab, "One").is_some_and(|tab| tab.focused),
        "the focus moves with the selection"
    );
}

#[test]
fn every_panel_is_built_once_and_kept_while_another_shows() {
    let selected = RwSignal::new(0);
    let builds = Rc::new(Cell::new(0));
    let mut harness = scene(selected, Rc::clone(&builds));
    assert_eq!(builds.get(), 3);

    press_tab(&mut harness, "Three");
    press_tab(&mut harness, "One");

    assert_eq!(builds.get(), 3, "no panel was built again");
}

#[test]
fn tab_stops_once_on_the_row_at_the_shown_tab_and_selects_nothing_passing() {
    TextSystem::bundled_only().install();
    let selected = RwSignal::new(1_usize);
    let bound = selected.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let field = |name: &'static str| {
            move || {
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .role(Role::Button)
                    .accessible_label(name)
                    .focusable()
            }
        };
        panel().align_items(Align::Start).child(
            tabs(bound.clone())
                .tab("One", field("In one"))
                .tab("Two", field("In two"))
                .tab("Three", field("In three")),
        )
    });
    harness.frame();
    let focused = |harness: &mut Harness| {
        harness
            .app()
            .semantics_nodes()
            .into_iter()
            .find(|node| node.focused)
            .and_then(|node| node.label)
    };

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Two"),
        "the shown tab"
    );
    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("In two"),
        "then its panel"
    );
    harness.press_key_with(
        Key::Tab,
        smudge_core::input::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Two"));
    assert_eq!(selected.get(), 1, "passing the tabs selects none");

    harness.press_key(Key::ArrowRight);
    assert_eq!(selected.get(), 2, "the arrows still select");
}
