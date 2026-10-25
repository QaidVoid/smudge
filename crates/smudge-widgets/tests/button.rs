//! Whole-frame behavior of the button and the containers, driven through the
//! headless harness.
//!
//! Every test installs a bundled-only text system first, because every button
//! measures its label, so the layout never depends on the fonts installed on
//! the machine running the tests.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::input::Key;
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;

use smudge_widgets::{ButtonExt, button, row, spacer, stack};

/// The absolute rectangles of every visible node, in paint order.
fn node_rects(harness: &mut Harness) -> Vec<Rect> {
    let mut rects = Vec::new();
    harness
        .app()
        .visit_nodes(|_depth, _name, rect| rects.push(rect));
    rects
}

/// The center of `rect`.
fn center(rect: Rect) -> Point {
    Point {
        x: (rect.x0 + rect.x1) / 2.0,
        y: (rect.y0 + rect.y1) / 2.0,
    }
}

/// A counting press handler and the counter it writes.
fn counter() -> (Rc<Cell<u32>>, impl FnMut()) {
    let presses = Rc::new(Cell::new(0u32));
    let handler = {
        let presses = Rc::clone(&presses);
        move || presses.set(presses.get() + 1)
    };
    (presses, handler)
}

#[test]
fn a_click_calls_the_handler() {
    TextSystem::bundled_only().install();
    let (presses, on_press) = counter();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().child(button("OK").on_press(on_press))
    });
    harness.frame();
    let button_rect = node_rects(&mut harness)[1];
    assert!(button_rect.width() > 0.0, "the button is laid out");

    harness.click(center(button_rect));

    assert_eq!(presses.get(), 1, "the click fires the handler once");
}

#[test]
fn enter_while_focused_calls_the_handler() {
    TextSystem::bundled_only().install();
    let (presses, on_press) = counter();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().child(button("OK").on_press(on_press))
    });
    harness.frame();
    harness.press_key(Key::Tab);

    harness.press_key(Key::Enter);

    assert_eq!(presses.get(), 1, "Enter fires the handler once");
}

#[test]
fn space_while_focused_calls_the_handler() {
    TextSystem::bundled_only().install();
    let (presses, on_press) = counter();
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        panel().child(button("OK").on_press(on_press))
    });
    harness.frame();
    harness.press_key(Key::Tab);

    harness.press_key(Key::Space);

    assert_eq!(presses.get(), 1, "Space fires the handler once");
}

#[test]
fn hover_changes_the_background_without_layout() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, || panel().child(button("OK")));
    harness.frame();
    let before = harness.png();
    let button_rect = node_rects(&mut harness)[1];

    harness.move_to(center(button_rect));
    let report = harness.frame();
    let after = harness.png();

    assert!(
        !report.layout_ran,
        "a hover patch that changes only the background never lays out"
    );
    assert_eq!(report.nodes_painted, 1, "only the button repaints");
    assert_ne!(before, after, "the hover changes what is on screen");
    harness.assert_matches_full_repaint();
}

#[test]
fn taking_the_focus_by_tab_draws_the_ring_without_layout() {
    TextSystem::bundled_only().install();
    let mut focused = Harness::new(200.0, 100.0, 1.0, || panel().child(button("OK")));
    let mut ringed = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(button("OK").border_color(ColorToken::FOCUS_RING))
    });
    focused.frame();
    ringed.frame();

    focused.press_key(Key::Tab);
    let report = focused.frame();

    assert!(
        !report.layout_ran,
        "the ring's space is reserved, so the focus moves nothing"
    );
    focused.assert_matches(&ringed);
    focused.assert_matches_full_repaint();
}

#[test]
fn a_focused_primary_button_rings_in_the_accent_text_color_in_the_dark_theme() {
    TextSystem::bundled_only().install();
    let mut focused = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(button("Save").primary())
    });
    let mut ringed = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(
            button("Save")
                .primary()
                .border_color(ColorToken::ACCENT_TEXT),
        )
    });
    for harness in [&mut focused, &mut ringed] {
        harness.app().set_theme(Theme::dark());
        harness.frame();
    }

    // The dark theme's focus color is its accent, so a ring in the focus
    // color would vanish into the primary button's fill.
    focused.press_key(Key::Tab);
    focused.frame();

    focused.assert_matches(&ringed);
}

#[test]
fn a_hovered_primary_button_paints_the_accent_hover_color() {
    TextSystem::bundled_only().install();
    let mut hovered = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(button("Save").primary())
    });
    let mut expected = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(
            button("Save")
                .primary()
                .background(ColorToken::ACCENT_HOVER),
        )
    });
    hovered.frame();
    expected.frame();
    let button_rect = node_rects(&mut hovered)[1];

    hovered.move_to(center(button_rect));
    hovered.frame();

    hovered.assert_matches(&expected);
}

#[test]
fn a_primary_button_renders_differently_from_the_default_button() {
    TextSystem::bundled_only().install();
    let mut default = Harness::new(200.0, 100.0, 1.0, || panel().child(button("Save")));
    let mut primary = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(button("Save").primary())
    });
    default.frame();
    primary.frame();

    assert_ne!(
        default.png(),
        primary.png(),
        "the primary variant changes what the button paints"
    );
    primary.assert_matches_full_repaint();
}

#[test]
fn stack_children_overlap_across_the_stack() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        panel().child(
            stack()
                .size(Size {
                    width: 80.0,
                    height: 40.0,
                })
                .child(panel().background(Color::WHITE))
                .child(panel().background(Color::BLACK)),
        )
    });
    harness.frame();

    let rects = node_rects(&mut harness);
    let stack_rect = rects[1];
    assert_eq!(rects[2], stack_rect, "the first layer covers the stack");
    assert_eq!(rects[3], stack_rect, "the second layer covers the stack");
    harness.assert_matches_full_repaint();
}

#[test]
fn layout_dump_of_a_row_of_buttons_and_a_spacer_matches_the_snapshot() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(320.0, 120.0, 1.0, || {
        row()
            .child(button("Save"))
            .child(spacer())
            .child(button("Cancel"))
            .child(button("Help"))
    });
    harness.frame();

    insta::assert_snapshot!(harness.layout_dump());
}
