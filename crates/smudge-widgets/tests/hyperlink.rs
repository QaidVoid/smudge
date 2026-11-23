//! Whole-frame behavior of the hyperlink, driven through the headless
//! harness.
//!
//! Every test installs a bundled-only text system first, because every
//! link measures its text, so the layout never depends on the fonts
//! installed on the machine running the tests.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::geometry::{Point, Rect, Size};
use smudge_core::input::Key;
use smudge_core::semantics::Role;
use smudge_core::style::Align;
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};
use smudge_widgets::{column, hyperlink};

/// The center of `rect`.
fn center(rect: Rect) -> Point {
    Point {
        x: (rect.x0 + rect.x1) / 2.0,
        y: (rect.y0 + rect.y1) / 2.0,
    }
}

/// The absolute rectangles of every visible node, in paint order.
fn node_rects(harness: &mut Harness) -> Vec<Rect> {
    let mut rects = Vec::new();
    harness
        .app()
        .visit_nodes(|_depth, _name, rect| rects.push(rect));
    rects
}

/// A counting press handler and the counter it writes.
fn counter() -> (Rc<Cell<u32>>, impl Fn()) {
    let opens = Rc::new(Cell::new(0u32));
    let handler = {
        let opens = Rc::clone(&opens);
        move || opens.set(opens.get() + 1)
    };
    (opens, handler)
}

/// A window of 200 by 100 whose root panel shows one link.
fn link_window(on_press: impl Fn() + 'static) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel()
            .align_items(Align::Start)
            .child(hyperlink("Read the guide", on_press))
    });
    harness.frame();
    harness
}

#[test]
fn a_click_runs_the_handler() {
    let (opens, on_press) = counter();
    let mut harness = link_window(on_press);
    let rect = node_rects(&mut harness)
        .pop()
        .expect("the window shows a link");

    harness.click(center(rect));

    assert_eq!(opens.get(), 1, "the click fires the handler once");
}

#[test]
fn enter_while_focused_runs_the_handler() {
    let (opens, on_press) = counter();
    let mut harness = link_window(on_press);

    harness.press_key(Key::Tab);
    harness.press_key(Key::Enter);

    assert_eq!(opens.get(), 1, "Enter fires the handler once");
}

#[test]
fn space_while_focused_runs_the_handler() {
    let (opens, on_press) = counter();
    let mut harness = link_window(on_press);

    harness.press_key(Key::Tab);
    harness.press_key(Key::Space);

    assert_eq!(opens.get(), 1, "Space fires the handler once");
}

#[test]
fn hovering_underlines_the_link_without_layout() {
    let mut harness = link_window(|| {});
    let before = harness.png();
    let rect = node_rects(&mut harness)
        .pop()
        .expect("the window shows a link");

    harness.move_to(center(rect));
    let report = harness.frame();
    let after = harness.png();

    assert!(
        !report.layout_ran,
        "the underline is a paint change, so the hover never lays out"
    );
    assert_eq!(report.nodes_painted, 1, "only the link repaints");
    assert_ne!(before, after, "the underline changes what is on screen");
    harness.assert_matches_full_repaint();
}

#[test]
fn focusing_underlines_the_link_the_way_a_forced_underline_looks() {
    TextSystem::bundled_only().install();
    let mut focused = Harness::new(200.0, 100.0, 1.0, || {
        panel()
            .align_items(Align::Start)
            .child(hyperlink("Read the guide", || {}))
    });
    let mut forced = Harness::new(200.0, 100.0, 1.0, || {
        panel().align_items(Align::Start).child(
            label("Read the guide")
                .text_color(ColorToken::LINK)
                .underline(true),
        )
    });
    focused.frame();
    forced.frame();

    focused.press_key(Key::Tab);
    focused.frame();

    assert_eq!(
        focused.png(),
        forced.png(),
        "a keyboard-focused link looks exactly like a link asked to underline"
    );
}

#[test]
fn the_link_reports_its_role_named_by_its_text() {
    let mut harness = link_window(|| {});

    let node = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Link)
        .expect("the window shows a link");
    assert_eq!(
        node.label.as_deref(),
        Some("Read the guide"),
        "the link is named by its text"
    );
}

/// A window that shows a link the keyboard focuses, a link the pointer
/// hovers, and a link at rest, in `theme`.
fn states_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 120.0, 1.0, || {
        panel().align_items(Align::Start).child(
            column()
                .gap(Size {
                    width: 0.0,
                    height: 8.0,
                })
                .child(hyperlink("Smudge home", || {}))
                .child(hyperlink("A hovered link", || {}))
                .child(hyperlink("One more link", || {})),
        )
    });
    harness.app().set_theme(theme);
    harness.frame();
    harness.press_key(Key::Tab);
    let rects = node_rects(&mut harness);
    // The chain is the root panel, the column, then the three links: the
    // second link is the one the pointer hovers.
    harness.move_to(center(rects[3]));
    harness.frame();
    harness
}

#[test]
fn the_states_of_a_hyperlink_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::light()).png());
}

#[test]
fn the_states_of_a_hyperlink_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::dark()).png());
}
