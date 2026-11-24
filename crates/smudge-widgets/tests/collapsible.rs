//! Collapsible sections: toggling by press and by key, the content mounted
//! only while open, and what a screen reader hears.

use smudge_core::geometry::{Point, Size};
use smudge_core::input::Key;
use smudge_core::reactive::RwSignal;
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};
use smudge_widgets::collapsible;

/// Returns the first projected node with `role`.
fn with_role(harness: &mut Harness, role: Role) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == role)
        .unwrap_or_else(|| panic!("the window shows a {role:?}"))
}

/// Returns the projected node that shows `text` as its value.
fn showing(harness: &mut Harness, text: &str) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some(text))
        .unwrap_or_else(|| panic!("the window shows {text:?}"))
}

/// Whether any projected node shows `text` as its value.
fn shows(harness: &mut Harness, text: &str) -> bool {
    harness
        .app()
        .semantics_nodes()
        .iter()
        .any(|node| node.value.as_deref() == Some(text))
}

/// Returns the role of the node that holds the focus.
fn focused_role(harness: &mut Harness) -> Option<Role> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
        .map(|node| node.role)
}

/// Returns the middle of `node`.
fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: (node.bounds.x0 + node.bounds.x1) / 2.0,
        y: (node.bounds.y0 + node.bounds.y1) / 2.0,
    }
}

/// A window of 300 by 220 with one collapsible section that starts as
/// `open` says, framed once.
fn window(open: bool) -> Harness {
    TextSystem::bundled_only().install();
    let open = RwSignal::new(open);
    let mut harness = Harness::new(300.0, 220.0, 1.0, move || {
        panel().align_items(Align::Start).child(collapsible(
            open,
            label("Advanced").padding(px(4.0)),
            || label("Expert settings live here"),
        ))
    });
    harness.frame();
    harness
}

#[test]
fn a_press_on_the_header_toggles_it() {
    let mut harness = window(false);
    assert_eq!(
        with_role(&mut harness, Role::Button).expanded,
        Some(false),
        "a closed section reads as not expanded"
    );
    assert!(
        harness
            .app()
            .semantics_nodes()
            .iter()
            .all(|node| node.value.as_deref() != Some("Expert settings live here")),
        "the content of a closed section is not mounted"
    );

    let header = with_role(&mut harness, Role::Button);
    harness.click(middle(&header));
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::Button).expanded,
        Some(true),
        "a press opens the section"
    );
    showing(&mut harness, "Expert settings live here");

    let header = with_role(&mut harness, Role::Button);
    harness.click(middle(&header));
    harness.frame();
    assert_eq!(with_role(&mut harness, Role::Button).expanded, Some(false));
    assert!(
        harness
            .app()
            .semantics_nodes()
            .iter()
            .all(|node| node.value.as_deref() != Some("Expert settings live here")),
        "a second press closes the section again"
    );
}

#[test]
fn enter_and_space_toggle_the_focused_header() {
    let mut harness = window(false);
    harness.press_key(Key::Tab);
    assert_eq!(
        focused_role(&mut harness),
        Some(Role::Button),
        "the header is a Tab stop"
    );

    harness.press_key(Key::Enter);
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::Button).expanded,
        Some(true),
        "Enter opens"
    );
    showing(&mut harness, "Expert settings live here");

    harness.press_key(Key::Space);
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::Button).expanded,
        Some(false),
        "Space closes"
    );
}

#[test]
fn right_opens_and_left_closes() {
    let mut harness = window(false);
    harness.press_key(Key::Tab);

    harness.press_key(Key::ArrowRight);
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::Button).expanded,
        Some(true),
        "Right opens a closed section"
    );
    showing(&mut harness, "Expert settings live here");

    harness.press_key(Key::ArrowLeft);
    harness.frame();
    assert_eq!(
        with_role(&mut harness, Role::Button).expanded,
        Some(false),
        "Left closes an open section"
    );
}

#[test]
fn a_section_that_starts_open_shows_its_content_until_it_closes() {
    let mut harness = window(true);
    showing(&mut harness, "Expert settings live here");
    assert_eq!(with_role(&mut harness, Role::Button).expanded, Some(true));

    harness.press_key(Key::Tab);
    harness.press_key(Key::Enter);
    harness.frame();
    assert!(
        !shows(&mut harness, "Expert settings live here"),
        "Enter on an open section closes it and unmounts the content"
    );
}

/// A window of 300 by 160 with a closed and an open section, themed.
fn sections_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let closed = RwSignal::new(false);
    let open = RwSignal::new(true);
    let mut harness = Harness::new(300.0, 160.0, 1.0, move || {
        panel()
            .column()
            .padding(px(12.0))
            .gap(Size {
                width: 0.0,
                height: 12.0,
            })
            .align_items(Align::Start)
            .child(collapsible(
                closed,
                label("Advanced").padding(px(4.0)),
                || label("Expert settings live here"),
            ))
            .child(collapsible(open, label("Display").padding(px(4.0)), || {
                label("Scale the interface")
            }))
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness.frame();
    harness
}

#[test]
fn a_closed_and_an_open_section_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", sections_window(Theme::light()).png());
}

#[test]
fn a_closed_and_an_open_section_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", sections_window(Theme::dark()).png());
}
