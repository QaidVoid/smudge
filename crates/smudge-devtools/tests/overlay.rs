//! The devtools panel, driven through the headless harness.

use smudge_core::geometry::{Point, Rect, Size};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_test::Harness;
use smudge_text::TextSystem;

/// A harness that shows the overlay in a window of 800 by 600.
fn overlay() -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(800.0, 600.0, 1.0, || {
        smudge_devtools::overlay(&smudge_devtools::Tools::new(), || Size {
            width: 800.0,
            height: 600.0,
        })
    });
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

#[test]
fn the_panel_stays_inside_the_window_past_its_right_and_bottom_edges() {
    TextSystem::bundled_only().install();
    let viewport = RwSignal::new(Size {
        width: 800.0,
        height: 600.0,
    });
    let read = viewport.clone();
    let mut harness = Harness::new(800.0, 600.0, 1.0, move || {
        let read = read.clone();
        smudge_devtools::overlay(&smudge_devtools::Tools::new(), move || read.get())
    });
    harness.frame();
    let before = panel_rect(&mut harness);
    let grab = Point {
        x: before.x0 + 20.0,
        y: before.y0 + 10.0,
    };
    harness.button_down(grab);
    harness.move_to(Point {
        x: 2000.0,
        y: 2000.0,
    });
    harness.button_up(Point {
        x: 2000.0,
        y: 2000.0,
    });
    harness.frame();
    let pinned = panel_rect(&mut harness);
    assert!(pinned.x1 <= 800.0, "the right edge stays in: {pinned:?}");
    assert!(
        pinned.y0 <= 600.0 - 32.0,
        "the title bar stays above the bottom: {pinned:?}"
    );

    viewport.set(Size {
        width: 500.0,
        height: 300.0,
    });
    harness.frame();
    let shrunk = panel_rect(&mut harness);
    assert!(
        shrunk.x1 <= 500.0,
        "a smaller window pulls the panel in: {shrunk:?}"
    );
    assert!(shrunk.y0 <= 300.0 - 32.0);
}

#[test]
fn hovering_a_bar_describes_its_frame() {
    TextSystem::bundled_only().install();
    let tools = smudge_devtools::Tools::new();
    let shown = tools.clone();
    let mut harness = Harness::new(800.0, 600.0, 1.0, move || {
        smudge_devtools::overlay(&shown, || Size {
            width: 800.0,
            height: 600.0,
        })
    });
    harness.frame();
    for painted in [3, 7] {
        tools.record(smudge_devtools::FrameRecord {
            layout: std::time::Duration::from_millis(4),
            presented: true,
            nodes_painted: painted,
            ..smudge_devtools::FrameRecord::default()
        });
    }
    harness.frame();
    let mut timeline = None;
    harness.app().visit_nodes(|_, type_name, rect| {
        if type_name.ends_with("TimelineView") {
            timeline = Some(rect);
        }
    });
    let timeline = timeline.expect("the panel shows the timeline");
    harness.move_to(Point {
        x: timeline.x1 - 0.5,
        y: timeline.y0 + 20.0,
    });
    harness.frame();
    let described = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .any(|text| text.contains("7 painted"));
    assert!(described, "the newest frame is described");

    tools.record(smudge_devtools::FrameRecord {
        presented: true,
        nodes_painted: 11,
        ..smudge_devtools::FrameRecord::default()
    });
    harness.frame();
    let followed = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .any(|text| text.contains("11 painted"));
    assert!(followed, "a new frame under the still pointer is described");
}
