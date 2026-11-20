//! Menus, menu bars and context menus, by pointer and by keyboard.

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::geometry::Point;
use smudge_core::input::{InputEvent, Key, Modifiers, PointerButton};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;
use smudge_widgets::{
    ContextMenuExt, menu, menu_bar, menu_check_item, menu_item, menu_separator, submenu,
};

/// A log of the items chosen.
type Log = Rc<RefCell<Vec<&'static str>>>;

/// Returns the projected node named `name`, if the window shows one.
fn node(harness: &mut Harness, name: &str) -> Option<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some(name))
}

/// Returns the center of the node named `name`.
fn center(harness: &mut Harness, name: &str) -> Point {
    let bounds = node(harness, name)
        .unwrap_or_else(|| panic!("the window shows {name}"))
        .bounds;
    Point {
        x: (bounds.x0 + bounds.x1) / 2.0,
        y: (bounds.y0 + bounds.y1) / 2.0,
    }
}

/// Clicks the middle of the node named `name`.
fn click_on(harness: &mut Harness, name: &str) {
    let at = center(harness, name);
    harness.click(at);
}

/// Moves the pointer to the middle of the node named `name`.
fn hover_on(harness: &mut Harness, name: &str) {
    let at = center(harness, name);
    harness.move_to(at);
}

/// Returns whether the node named `name` holds the focus.
fn focused(harness: &mut Harness, name: &str) -> bool {
    node(harness, name).is_some_and(|node| node.focused)
}

/// Returns an item named `name` that logs its name to `log` when chosen.
fn logging(name: &'static str, log: &Log) -> smudge_widgets::MenuItem {
    let log = Rc::clone(log);
    menu_item(name, move || log.borrow_mut().push(name))
}

/// A window of 300 by 200 with an Edit menu button at its top left: Cut,
/// Copy, a separator, a Wrap check item, and a More submenu of Paste and
/// Clear.
fn edit_menu(log: Log, wrap: RwSignal<bool>) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 200.0, 1.0, move || {
        let (log, wrap) = (Rc::clone(&log), wrap.clone());
        panel()
            .align_items(Align::Start)
            .child(menu("Edit", move || {
                let more = Rc::clone(&log);
                vec![
                    logging("Cut", &log),
                    logging("Copy", &log).shortcut("Ctrl+C"),
                    menu_separator(),
                    menu_check_item("Wrap", wrap.clone()),
                    submenu("More", move || {
                        vec![logging("Paste", &more), logging("Clear", &more)]
                    }),
                ]
            }))
    });
    harness.frame();
    harness
}

#[test]
fn a_menu_opens_below_its_button_and_runs_the_item_chosen_then_closes() {
    let log = Log::default();
    let mut harness = edit_menu(Rc::clone(&log), RwSignal::new(false));
    let title = node(&mut harness, "Edit").expect("the button shows").bounds;

    click_on(&mut harness, "Edit");
    harness.frame();
    let cut = node(&mut harness, "Cut").expect("the menu opened").bounds;
    assert!(
        cut.y0 >= title.y1,
        "below the button: {cut:?} under {title:?}"
    );

    click_on(&mut harness, "Copy");
    harness.frame();
    assert_eq!(*log.borrow(), ["Copy"]);
    assert!(node(&mut harness, "Cut").is_none(), "the menu closed");
    harness.assert_matches_full_repaint();
}

#[test]
fn the_keyboard_opens_walks_and_chooses() {
    let log = Log::default();
    let wrap = RwSignal::new(false);
    let mut harness = edit_menu(Rc::clone(&log), wrap.clone());
    click_on(&mut harness, "Edit");
    harness.frame();
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(node(&mut harness, "Cut").is_none(), "Escape closed it");
    assert!(focused(&mut harness, "Edit"), "and gave the focus back");

    harness.press_key(Key::ArrowDown);
    harness.frame();
    assert!(
        focused(&mut harness, "Cut"),
        "the first item takes the focus"
    );
    harness.press_key(Key::ArrowDown);
    harness.press_key(Key::ArrowDown);
    assert!(focused(&mut harness, "Wrap"), "past the separator");
    harness.press_key(Key::Enter);
    harness.frame();
    assert!(wrap.get(), "the check item toggled");
    assert!(node(&mut harness, "Cut").is_none(), "and the menu closed");

    harness.press_key(Key::Enter);
    harness.frame();
    harness.press_key(Key::Character(String::from("c")));
    assert!(focused(&mut harness, "Copy"), "c goes on from Cut to Copy");
    harness.press_key(Key::Space);
    harness.frame();
    assert_eq!(*log.borrow(), ["Copy"]);
    assert!(
        node(&mut harness, "Wrap").is_none(),
        "the menu closed again"
    );
}

#[test]
fn right_opens_a_submenu_left_closes_it_and_a_choice_closes_every_menu() {
    let log = Log::default();
    let mut harness = edit_menu(Rc::clone(&log), RwSignal::new(false));
    click_on(&mut harness, "Edit");
    harness.frame();
    harness.press_key(Key::End);
    assert!(focused(&mut harness, "More"));

    harness.press_key(Key::ArrowRight);
    harness.frame();
    let more = node(&mut harness, "More").expect("the item shows").bounds;
    let paste = node(&mut harness, "Paste")
        .expect("the submenu opened")
        .bounds;
    assert!(
        paste.x0 >= more.x1 - 1.0,
        "beside the item: {paste:?} by {more:?}"
    );
    assert!(focused(&mut harness, "Paste"));

    harness.press_key(Key::ArrowLeft);
    harness.frame();
    assert!(
        node(&mut harness, "Paste").is_none(),
        "Left closed the submenu"
    );
    assert!(focused(&mut harness, "More"), "and gave the focus back");

    harness.press_key(Key::ArrowRight);
    harness.frame();
    harness.press_key(Key::ArrowDown);
    harness.press_key(Key::Enter);
    harness.frame();
    assert_eq!(*log.borrow(), ["Clear"]);
    assert!(node(&mut harness, "Cut").is_none(), "every menu closed");
    harness.assert_matches_full_repaint();
}

#[test]
fn hovering_an_item_opens_its_submenu_and_hovering_another_closes_it() {
    let log = Log::default();
    let mut harness = edit_menu(log, RwSignal::new(false));
    click_on(&mut harness, "Edit");
    harness.frame();

    hover_on(&mut harness, "More");
    harness.frame();
    assert!(node(&mut harness, "Paste").is_some());
    hover_on(&mut harness, "Cut");
    harness.frame();
    assert!(node(&mut harness, "Paste").is_none());
}

#[test]
fn a_submenu_at_the_right_edge_opens_on_the_left() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 200.0, 1.0, || {
        panel()
            .align_items(Align::Start)
            .child(panel().width(px(330.0)).shrink(0.0))
            .child(menu("Tools", || {
                vec![submenu("Deep", || vec![menu_item("Inner", || {})])]
            }))
    });
    harness.frame();
    click_on(&mut harness, "Tools");
    harness.frame();
    harness.press_key(Key::ArrowRight);
    harness.frame();
    let deep = node(&mut harness, "Deep").expect("the item shows").bounds;
    let inner = node(&mut harness, "Inner")
        .expect("the submenu opened")
        .bounds;
    assert!(deep.x1 > 390.0, "the menu sits at the right edge: {deep:?}");
    assert!(
        inner.x1 <= deep.x0 + 6.0,
        "left of the item: {inner:?} {deep:?}"
    );
}

#[test]
fn a_right_press_opens_a_context_menu_at_the_pointer_and_the_menu_key_at_the_focus() {
    TextSystem::bundled_only().install();
    let log = Log::default();
    let chosen = Rc::clone(&log);
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let chosen = Rc::clone(&chosen);
        panel().align_items(Align::Start).child(
            panel()
                .width(px(200.0))
                .height(px(150.0))
                .focusable()
                .context_menu(move || vec![logging("Rename", &chosen), logging("Delete", &chosen)]),
        )
    });
    harness.frame();
    let at = Point { x: 60.0, y: 40.0 };
    harness.event(InputEvent::PointerDown {
        position: at,
        button: PointerButton::Secondary,
    });
    harness.event(InputEvent::PointerUp {
        position: at,
        button: PointerButton::Secondary,
    });
    harness.frame();
    let rename = node(&mut harness, "Rename")
        .expect("the menu opened")
        .bounds;
    assert!(
        (rename.x0 - at.x).abs() <= 8.0 && (rename.y0 - at.y).abs() <= 8.0,
        "at the pointer: {rename:?}"
    );
    click_on(&mut harness, "Delete");
    harness.frame();
    assert_eq!(*log.borrow(), ["Delete"]);

    harness.click(Point { x: 150.0, y: 120.0 });
    harness.press_key_with(
        Key::Function(10),
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    harness.frame();
    let rename = node(&mut harness, "Rename")
        .expect("the menu key opened it")
        .bounds;
    assert!(
        rename.x0 <= 10.0 && rename.y0 <= 10.0,
        "at the node's corner: {rename:?}"
    );
}

#[test]
fn a_menu_bar_moves_between_its_menus_and_reports_its_roles() {
    TextSystem::bundled_only().install();
    let grid = RwSignal::new(true);
    let shown = grid.clone();
    let mut harness = Harness::new(400.0, 200.0, 1.0, move || {
        let shown = shown.clone();
        panel().align_items(Align::Start).child(
            menu_bar()
                .menu("File", || vec![menu_item("New", || {})])
                .menu("View", move || vec![menu_check_item("Grid", shown.clone())]),
        )
    });
    harness.frame();
    let roles: Vec<Role> = harness
        .app()
        .semantics_nodes()
        .iter()
        .map(|node| node.role)
        .collect();
    assert!(roles.contains(&Role::MenuBar), "{roles:?}");

    click_on(&mut harness, "File");
    harness.frame();
    assert!(node(&mut harness, "New").is_some());
    assert_eq!(
        node(&mut harness, "New").map(|node| node.role),
        Some(Role::MenuItem)
    );
    harness.press_key(Key::ArrowRight);
    harness.frame();
    assert!(node(&mut harness, "New").is_none(), "File closed");
    let item = node(&mut harness, "Grid").expect("View opened");
    assert_eq!(item.role, Role::MenuItemCheckBox);
    assert_eq!(item.checked, Some(Checked::True));
    assert!(item.focused);
    assert!(
        harness
            .app()
            .semantics_nodes()
            .iter()
            .any(|node| node.role == Role::Menu),
        "the open menu reports its role"
    );

    harness.move_to(Point { x: 390.0, y: 190.0 });
    hover_on(&mut harness, "File");
    harness.frame();
    assert!(
        node(&mut harness, "New").is_some(),
        "hovering File opens it instead"
    );
    assert!(node(&mut harness, "Grid").is_none());
}

#[test]
fn a_menu_that_starts_with_a_disabled_item_focuses_the_first_it_can_and_never_chooses_it() {
    TextSystem::bundled_only().install();
    let log = Log::default();
    let chosen = Rc::clone(&log);
    let mut harness = Harness::new(400.0, 200.0, 1.0, move || {
        let chosen = Rc::clone(&chosen);
        panel()
            .align_items(Align::Start)
            .child(menu("Edit", move || {
                vec![
                    logging("Undo", &chosen).disabled(true),
                    menu_separator(),
                    logging("Cut", &chosen),
                ]
            }))
    });
    harness.frame();
    click_on(&mut harness, "Edit");
    harness.frame();
    assert!(focused(&mut harness, "Cut"), "the first item it can focus");

    click_on(&mut harness, "Undo");
    harness.frame();
    harness.press_key(Key::Home);
    assert!(
        focused(&mut harness, "Cut"),
        "Home passes over the disabled item"
    );
    assert!(log.borrow().is_empty(), "a disabled item is never chosen");
}

#[test]
fn an_action_creates_what_outlives_the_menu() {
    TextSystem::bundled_only().install();
    let later = RwSignal::new(false);
    let seen = later.clone();
    let mut harness = Harness::new(400.0, 200.0, 1.0, move || {
        let seen = seen.clone();
        panel()
            .align_items(Align::Start)
            .child(menu("File", move || {
                let seen = seen.clone();
                vec![menu_item("Save", move || {
                    let seen = seen.clone();
                    smudge_core::timer::after(std::time::Duration::from_millis(50), move || {
                        seen.set(true);
                    });
                })]
            }))
    });
    harness.frame();
    click_on(&mut harness, "File");
    harness.frame();
    click_on(&mut harness, "Save");
    harness.frame();
    assert!(node(&mut harness, "Save").is_none(), "the menu closed");

    harness.advance(std::time::Duration::from_millis(100));
    harness.frame();
    assert!(later.get(), "the timer the action set still fired");
}

#[test]
fn escape_after_moving_across_a_menu_bar_returns_to_the_title_of_the_open_menu() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 200.0, 1.0, || {
        panel().align_items(Align::Start).child(
            menu_bar()
                .menu("File", || vec![menu_item("New", || {})])
                .menu("View", || vec![menu_item("Zoom", || {})]),
        )
    });
    harness.frame();
    click_on(&mut harness, "File");
    harness.frame();
    harness.press_key(Key::ArrowRight);
    harness.frame();
    assert!(focused(&mut harness, "Zoom"));

    harness.press_key(Key::Escape);
    harness.frame();

    assert!(
        focused(&mut harness, "View"),
        "the focus is on the title of the menu that closed"
    );
}

#[test]
fn an_open_menu_keeps_its_keys_from_the_shortcuts_around_it() {
    TextSystem::bundled_only().install();
    let stolen = Rc::new(std::cell::Cell::new(0u32));
    let count = Rc::clone(&stolen);
    let mut harness = Harness::new(400.0, 200.0, 1.0, move || {
        let count = Rc::clone(&count);
        panel()
            .align_items(Align::Start)
            .on_key(move |event| {
                if event.pressed && matches!(event.key, Key::ArrowDown | Key::Character(_)) {
                    count.set(count.get() + 1);
                    return smudge_core::input::Handled::Yes;
                }
                smudge_core::input::Handled::No
            })
            .child(menu("Edit", || {
                vec![menu_item("Cut", || {}), menu_item("Copy", || {})]
            }))
    });
    harness.frame();
    click_on(&mut harness, "Edit");
    harness.frame();

    harness.press_key(Key::ArrowDown);
    assert!(focused(&mut harness, "Copy"));
    harness.press_key(Key::Character(String::from("c")));
    assert!(focused(&mut harness, "Cut"));
    assert_eq!(stolen.get(), 0, "the shortcuts around the menu heard none");
}

#[test]
fn a_menu_button_reports_a_button_that_opens_and_closes_its_menu() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(400.0, 200.0, 1.0, || {
        panel()
            .align_items(Align::Start)
            .child(menu("Edit", || vec![menu_item("Cut", || {})]))
    });
    harness.frame();
    let button = node(&mut harness, "Edit").expect("the button shows");
    assert_eq!(button.role, Role::Button);
    assert_eq!(button.expanded, Some(false));

    click_on(&mut harness, "Edit");
    harness.frame();
    assert_eq!(
        node(&mut harness, "Edit").and_then(|node| node.expanded),
        Some(true)
    );
}
