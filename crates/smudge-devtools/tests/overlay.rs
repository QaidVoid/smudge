//! The devtools panel, driven through the headless harness.

use smudge_core::geometry::{Point, Rect};
use smudge_test::Harness;
use smudge_text::TextSystem;

/// A harness that shows the overlay in a window of 800 by 600.
fn overlay() -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(800.0, 600.0, 1.0, smudge_devtools::overlay);
    harness.frame();
    harness
}

/// The rectangle of the panel, the root's one child.
fn panel_rect(harness: &mut Harness) -> Rect {
    let mut found = None;
    harness.app().visit_nodes(|depth, _, rect| {
        if depth == 1 && found.is_none() {
            found = Some(rect);
        }
    });
    found.expect("the root holds the panel")
}

#[test]
fn a_drag_on_the_title_bar_moves_the_panel_with_the_pointer() {
    let mut harness = overlay();
    let before = panel_rect(&mut harness);
    let grab = Point {
        x: before.x0 + 20.0,
        y: before.y0 + 10.0,
    };
    harness.button_down(grab);
    harness.move_to(Point {
        x: grab.x + 100.0,
        y: grab.y + 60.0,
    });
    harness.move_to(Point {
        x: grab.x + 200.0,
        y: grab.y + 120.0,
    });
    harness.button_up(Point {
        x: grab.x + 200.0,
        y: grab.y + 120.0,
    });
    harness.frame();
    let after = panel_rect(&mut harness);
    assert_eq!(
        (after.x0 - before.x0, after.y0 - before.y0),
        (200.0, 120.0),
        "the point grabbed stays under the pointer"
    );
}

#[test]
fn the_button_collapses_the_panel_to_its_title_bar_and_opens_it_again() {
    let mut harness = overlay();
    let open = panel_rect(&mut harness);
    let button = Point {
        x: open.x1 - 20.0,
        y: open.y0 + 14.0,
    };
    harness.click(button);
    harness.frame();
    let collapsed = panel_rect(&mut harness);
    assert!(
        collapsed.height() < open.height() - 100.0,
        "the body is gone: {collapsed:?} from {open:?}"
    );
    harness.click(button);
    harness.frame();
    assert_eq!(panel_rect(&mut harness), open);
}

#[test]
fn a_drag_past_the_top_or_left_leaves_the_title_bar_in_reach() {
    let mut harness = overlay();
    let before = panel_rect(&mut harness);
    let grab = Point {
        x: before.x0 + 20.0,
        y: before.y0 + 10.0,
    };
    harness.button_down(grab);
    harness.move_to(Point {
        x: -600.0,
        y: -300.0,
    });
    harness.button_up(Point {
        x: -600.0,
        y: -300.0,
    });
    harness.frame();
    let pinned = panel_rect(&mut harness);
    assert_eq!(
        (pinned.x0, pinned.y0),
        (0.0, 0.0),
        "the panel stops at the window's corner"
    );

    let middle = Point { x: 20.0, y: 20.0 };
    harness.button_down(middle);
    harness.move_to(Point { x: 120.0, y: 70.0 });
    harness.button_up(Point { x: 120.0, y: 70.0 });
    harness.frame();
    let moved = panel_rect(&mut harness);
    assert_eq!(
        (moved.x0, moved.y0),
        (100.0, 50.0),
        "the middle of the title bar grabs the panel again"
    );
}
