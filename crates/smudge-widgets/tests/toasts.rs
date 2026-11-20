//! Toasts, driven by the harness clock.

use std::time::Duration;

use smudge_core::geometry::Point;
use smudge_core::reactive::RwSignal;
use smudge_core::semantics::Politeness;
use smudge_test::Harness;
use smudge_text::{TextSystem, text_input};
use smudge_widgets::{MAX_TOASTS, Toasts, column, toast_host};

/// A window of 400 by 300 with a field that holds the focus and the toast
/// host, and the handle that shows toasts in it.
fn scene() -> (Harness, Toasts) {
    TextSystem::bundled_only().install();
    let toasts = Toasts::new();
    let host = toasts.clone();
    let mut harness = Harness::new(400.0, 300.0, 1.0, move || {
        column()
            .child(
                text_input(RwSignal::new(String::new()))
                    .accessible_label("Name")
                    .autofocus(),
            )
            .child(toast_host(&host))
    });
    harness.settle();
    (harness, toasts)
}

/// The messages of the toasts that show, top to bottom.
fn shown(harness: &mut Harness) -> Vec<String> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.value)
        .filter(|value| value.starts_with("Toast"))
        .collect()
}

/// Lets `millis` of the harness clock pass, a frame at a time.
fn wait(harness: &mut Harness, millis: u64) {
    harness.advance(Duration::from_millis(millis));
    harness.settle();
}

#[test]
fn a_toast_leaves_after_its_duration_and_not_before() {
    let (mut harness, toasts) = scene();
    toasts.show("Toast saved");
    harness.settle();
    assert_eq!(shown(&mut harness), ["Toast saved"]);

    wait(&mut harness, 3_900);
    assert_eq!(
        shown(&mut harness),
        ["Toast saved"],
        "still there before four seconds"
    );
    wait(&mut harness, 200);
    assert!(shown(&mut harness).is_empty(), "gone after them");

    toasts.show_for("Toast brief", Duration::from_millis(500));
    harness.settle();
    wait(&mut harness, 600);
    assert!(
        shown(&mut harness).is_empty(),
        "a toast of its own duration leaves after it"
    );
}

#[test]
fn the_pointer_over_a_toast_pauses_it_until_it_leaves() {
    let (mut harness, toasts) = scene();
    toasts.show("Toast paused");
    harness.settle();
    wait(&mut harness, 2_000);

    let toast = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some("Toast paused"))
        .expect("the toast shows")
        .bounds;
    harness.move_to(Point {
        x: (toast.x0 + toast.x1) / 2.0,
        y: (toast.y0 + toast.y1) / 2.0,
    });
    harness.settle();
    wait(&mut harness, 5_000);
    assert_eq!(
        shown(&mut harness),
        ["Toast paused"],
        "the pointer holds it"
    );

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.settle();
    wait(&mut harness, 1_900);
    assert_eq!(
        shown(&mut harness),
        ["Toast paused"],
        "the two seconds it had left run on"
    );
    wait(&mut harness, 200);
    assert!(shown(&mut harness).is_empty());
}

#[test]
fn a_press_closes_a_toast_and_a_fifth_closes_the_oldest() {
    let (mut harness, toasts) = scene();
    for index in 0..=MAX_TOASTS {
        toasts.show(format!("Toast {index}"));
    }
    harness.settle();
    assert_eq!(
        shown(&mut harness),
        ["Toast 1", "Toast 2", "Toast 3", "Toast 4"],
        "the newest at the bottom, the oldest gone"
    );

    let newest = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.value.as_deref() == Some("Toast 4"))
        .expect("the newest shows")
        .bounds;
    assert!(
        newest.x0 > 200.0 && newest.y1 > 250.0,
        "in the bottom right corner: {newest:?}"
    );
    harness.click(Point {
        x: (newest.x0 + newest.x1) / 2.0,
        y: (newest.y0 + newest.y1) / 2.0,
    });
    harness.settle();
    assert_eq!(shown(&mut harness), ["Toast 1", "Toast 2", "Toast 3"]);
    harness.assert_matches_full_repaint();
}

#[test]
fn each_toast_is_announced_once_and_none_takes_the_focus() {
    let (mut harness, toasts) = scene();
    harness.app().take_a11y_changes();
    let focused = |harness: &mut Harness| {
        harness
            .app()
            .semantics_nodes()
            .into_iter()
            .find(|node| node.focused)
            .and_then(|node| node.label)
    };
    assert_eq!(focused(&mut harness).as_deref(), Some("Name"));

    toasts.show("Toast one");
    toasts.show_urgent("Toast two");
    harness.settle();
    wait(&mut harness, 100);

    let spoken: Vec<(String, Politeness)> = harness
        .app()
        .take_a11y_changes()
        .announcements
        .into_iter()
        .map(|announcement| (announcement.message, announcement.politeness))
        .collect();
    assert_eq!(
        spoken,
        [
            (String::from("Toast one"), Politeness::Polite),
            (String::from("Toast two"), Politeness::Assertive),
        ]
    );
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Name"),
        "the field keeps the focus"
    );
}
