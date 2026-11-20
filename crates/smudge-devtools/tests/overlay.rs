//! The devtools panel, driven through the headless harness.

use smudge_core::geometry::{Point, Rect, Size};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role};
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

/// The rectangle of the panel, the root's last child, after the tab order
/// numbers.
fn panel_rect(harness: &mut Harness) -> Rect {
    let mut found = None;
    harness.app().visit_nodes(|depth, _, rect| {
        if depth == 1 {
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
        .any(|text| text.starts_with("7 nodes painted"));
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
        .any(|text| text.starts_with("11 nodes painted"));
    assert!(followed, "a new frame under the still pointer is described");
}

/// The center of the text node that reads `text`.
fn text_at(harness: &mut Harness, text: &str) -> Option<Point> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some(text))
        .map(|node| Point {
            x: (node.bounds.x0 + node.bounds.x1) / 2.0,
            y: (node.bounds.y0 + node.bounds.y1) / 2.0,
        })
}

/// Whether the switch named `name` reads as on.
fn switch_on(harness: &mut Harness, name: &str) -> bool {
    let node = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Switch && node.label.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the panel shows the {name} switch"));
    node.checked == Some(Checked::True)
}

/// Presses the overlay's text that reads `text`.
fn press(harness: &mut Harness, text: &str) {
    let at = text_at(harness, text).unwrap_or_else(|| panic!("the panel shows {text:?}"));
    harness.click(at);
    harness.frame();
}

#[test]
fn the_panels_switches_turn_on_and_off_and_say_so() {
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
    press(&mut harness, "Overlays");
    for name in ["Damage tint", "Paint flashing", "Layout bounds"] {
        assert!(!switch_on(&mut harness, name), "{name} starts off");
        press(&mut harness, name);
        assert!(switch_on(&mut harness, name), "{name} reads as on");
    }
    press(&mut harness, "Full repaint");
    assert!(switch_on(&mut harness, "Full repaint"));
    let switches = tools.switches();
    assert!(
        switches.damage_tint
            && switches.paint_flashing
            && switches.layout_bounds
            && switches.full_repaint
    );

    press(&mut harness, "Damage tint");
    assert!(!tools.switches().damage_tint);
    assert!(!switch_on(&mut harness, "Damage tint"));
}

#[test]
fn the_live_tab_shows_a_build_failure_and_a_patch_outcome() {
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
    press(&mut harness, "Live");
    tools.record_hot(smudge_devtools::HotEvent::Building);
    tools.record_hot(smudge_devtools::HotEvent::BuildFailed(String::from(
        "error[E0308]: mismatched types",
    )));
    tools.record_hot(smudge_devtools::HotEvent::Building);
    tools.record_hot(smudge_devtools::HotEvent::Applied {
        took: std::time::Duration::from_millis(12),
        remounted: Some(smudge_core::app::Remounted::default()),
    });
    harness.frame();
    let texts: Vec<String> = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .collect();
    let at = |text: &str| texts.iter().position(|shown| shown == text);
    let applied = at("Patch applied in 12.0 ms").expect("the patch shows");
    let failed = at("Build failed").expect("the failure shows");
    assert!(applied < failed, "the newest step comes first: {texts:?}");
    assert!(
        at("error[E0308]: mismatched types").is_some(),
        "the compiler's message shows: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("Remounted, focus not restored")),
        "{texts:?}"
    );
    assert!(
        at("Building a patch").is_none(),
        "a finished build shows how it ended: {texts:?}"
    );
}

#[test]
fn a_watched_theme_file_recolors_the_app_and_its_bad_lines_show() {
    TextSystem::bundled_only().install();
    let dir = std::env::temp_dir().join(format!("smudge-theme-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the directory is made");
    let path = dir.join("theme.txt");
    std::fs::write(&path, "accent = #e07a3a\nglow = #ffffff\n").expect("the file is written");

    let tools = smudge_devtools::Tools::new();
    let theme = RwSignal::new(smudge_core::theme::Theme::dark());
    let (watched, shown) = (theme.clone(), tools.clone());
    let watching = path.clone();
    let mut harness = Harness::new(800.0, 600.0, 1.0, move || {
        std::mem::forget(smudge_devtools::watch_theme(
            watching.clone(),
            watched.clone(),
        ));
        smudge_devtools::overlay(&shown, || Size {
            width: 800.0,
            height: 600.0,
        })
    });
    harness.frame();
    press(&mut harness, "Live");
    harness.advance(std::time::Duration::from_millis(300));
    harness.frame();
    assert_eq!(
        theme.get().color(smudge_core::theme::ColorToken::ACCENT),
        smudge_core::geometry::Color::rgb(0xe0, 0x7a, 0x3a),
        "the file's accent reached the theme"
    );
    let reported = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .any(|text| text == "line 2: the theme has no token `glow`");
    assert!(reported, "the bad line shows in the panel");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Returns the projected node that holds the focus, if any.
fn focused(harness: &mut Harness) -> Option<smudge_core::semantics::SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
}

#[test]
fn the_keyboard_reaches_the_tabs_and_the_switches() {
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

    harness.press_key(smudge_core::input::Key::Tab);
    harness.press_key(smudge_core::input::Key::Tab);
    let tab = focused(&mut harness).expect("Tab reaches the tab bar");
    assert_eq!(
        (tab.role, tab.label.as_deref(), tab.selected),
        (Role::Tab, Some("Frames"), Some(true)),
        "at the shown tab"
    );
    harness.press_key(smudge_core::input::Key::ArrowRight);
    harness.press_key(smudge_core::input::Key::Enter);
    harness.frame();
    assert_eq!(
        focused(&mut harness).and_then(|node| node.label).as_deref(),
        Some("Overlays")
    );

    let switch = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == Role::Switch && node.label.as_deref() == Some("Damage tint"))
        .expect("Enter showed the overlays tab");
    assert!(switch.description.is_some(), "a switch says what it does");
    assert!(harness.app().request_focus(switch.id));
    harness.press_key(smudge_core::input::Key::Space);
    harness.frame();
    assert!(tools.switches().damage_tint, "Space turns a switch on");
}

#[test]
fn a_collapsed_panel_asks_the_window_for_no_tab_data() {
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
    press(&mut harness, "Inspector");
    assert!(tools.is_inspecting());

    press(&mut harness, "Hide");
    assert!(!tools.is_inspecting(), "nothing shows the tree");
    press(&mut harness, "Show");
    assert!(tools.is_inspecting());
}

#[test]
fn a_narrow_window_holds_the_whole_panel() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(320.0, 360.0, 1.0, || {
        smudge_devtools::overlay(&smudge_devtools::Tools::new(), || Size {
            width: 320.0,
            height: 360.0,
        })
    });
    harness.frame();
    let panel = panel_rect(&mut harness);
    assert!(
        panel.x0 >= 0.0 && panel.x1 <= 320.0,
        "the panel fits the window: {panel:?}"
    );
}
