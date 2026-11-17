//! Whole-frame behavior of the split, driven through the headless harness.

use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Direction, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_widgets::{SplitExt, split};

/// The window the splits sit in.
const WINDOW: Size = Size {
    width: 400.0,
    height: 200.0,
};

/// The rectangles of the direct children of the outermost split, as left,
/// top, right and bottom: the first pane, the divider and the second pane.
fn parts(harness: &Harness) -> Vec<[f32; 4]> {
    let dump = harness.layout_dump();
    let mut lines = dump.lines();
    let Some(split) = lines
        .by_ref()
        .find(|line| line.trim_start().starts_with("Split ["))
    else {
        panic!("the layout holds a split");
    };
    let depth = split.len() - split.trim_start().len();
    lines
        .take_while(|line| line.len() - line.trim_start().len() > depth)
        .filter(|line| line.len() - line.trim_start().len() == depth + 2)
        .map(|line| {
            let start = line.find('[').expect("a rectangle") + 1;
            let values: Vec<f32> = line[start..line.len() - 1]
                .split(", ")
                .map(|value| value.parse().expect("numbers"))
                .collect();
            [values[0], values[1], values[2], values[3]]
        })
        .collect()
}

/// Builds a row split with `ratio` and the given minimums, filling the
/// window, and settles it.
fn row_split(ratio: RwSignal<f32>, min_first: f32, min_second: f32) -> Harness {
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        split(Direction::Row, panel(), panel())
            .ratio(ratio.clone())
            .min_first(min_first)
            .min_second(min_second)
    });
    harness.frame();
    harness.settle();
    harness
}

/// The middle of the divider, from its rectangle.
fn divider_middle(harness: &Harness) -> Point {
    let divider = parts(harness)[1];
    Point {
        x: (divider[0] + divider[2]) / 2.0,
        y: (divider[1] + divider[3]) / 2.0,
    }
}

#[test]
fn dragging_the_divider_moves_it_and_writes_the_ratio() {
    let ratio = RwSignal::new(0.5);
    let mut harness = row_split(ratio.clone(), 0.0, 0.0);
    assert_eq!(parts(&harness)[0][2], 200.0, "the first pane takes half");

    let start = divider_middle(&harness);
    harness.button_down(start);
    harness.move_to(Point {
        x: start.x + 100.0,
        y: start.y,
    });
    harness.button_up(Point {
        x: start.x + 100.0,
        y: start.y,
    });
    harness.frame();

    assert_eq!(ratio.get(), 0.75, "the first pane took three quarters");
    let parts = parts(&harness);
    assert_eq!(
        parts[0][2], 300.0,
        "the first pane ends where the drag ended"
    );
    assert_eq!(parts[1][0], 300.0, "the divider follows it");
    assert_eq!(parts[2][2], 400.0, "the second pane fills the rest");
    harness.assert_matches_full_repaint();
}

#[test]
fn the_minimums_hold_past_a_drag_and_a_smaller_window() {
    let ratio = RwSignal::new(0.5);
    let mut harness = row_split(ratio.clone(), 100.0, 120.0);

    let start = divider_middle(&harness);
    harness.button_down(start);
    harness.move_to(Point {
        x: 395.0,
        y: start.y,
    });
    harness.frame();
    assert_eq!(
        400.0 - parts(&harness)[2][0],
        120.0,
        "the second pane kept its minimum"
    );
    harness.move_to(Point { x: 5.0, y: start.y });
    harness.button_up(Point { x: 5.0, y: start.y });
    harness.frame();
    assert_eq!(
        parts(&harness)[0][2],
        100.0,
        "the first pane kept its minimum"
    );
    assert_eq!(ratio.get(), 0.25, "the ratio asks for no less");

    // A quarter of the smaller window is less than the first minimum.
    harness.app().set_viewport(
        Size {
            width: 250.0,
            height: 200.0,
        },
        1.0,
    );
    harness.frame();
    let parts_now = parts(&harness);
    assert_eq!(parts_now[2][2], 250.0, "the split took the smaller window");
    assert_eq!(
        parts_now[0][2] - parts_now[0][0],
        100.0,
        "the first pane kept its minimum in layout"
    );
}

#[test]
fn nested_splits_keep_their_own_ratios() {
    let outer = RwSignal::new(0.5);
    let inner = RwSignal::new(0.5);
    let (bound_outer, bound_inner) = (outer.clone(), inner.clone());
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, move || {
        split(
            Direction::Row,
            panel(),
            split(Direction::Column, panel(), panel()).ratio(bound_inner.clone()),
        )
        .ratio(bound_outer.clone())
    });
    harness.frame();
    harness.settle();

    // The inner split's divider is horizontal, across the right half.
    harness.button_down(Point { x: 300.0, y: 100.0 });
    harness.move_to(Point { x: 300.0, y: 150.0 });
    harness.button_up(Point { x: 300.0, y: 150.0 });
    harness.frame();

    assert_eq!(outer.get(), 0.5, "the outer split did not move");
    assert!(
        inner.get() > 0.7,
        "the inner split moved, ratio {}",
        inner.get()
    );
}

#[test]
fn the_arrow_keys_home_and_end_move_a_focused_divider() {
    let ratio = RwSignal::new(0.5);
    let mut harness = row_split(ratio.clone(), 40.0, 60.0);

    harness.press_key(Key::Tab);
    harness.frame();
    harness.press_key(Key::ArrowRight);
    harness.frame();
    assert_eq!(parts(&harness)[0][2], 216.0, "one step is 16 pixels");
    harness.press_key(Key::ArrowLeft);
    harness.press_key(Key::ArrowLeft);
    harness.frame();
    assert_eq!(parts(&harness)[0][2], 184.0);
    harness.press_key(Key::Home);
    harness.frame();
    assert_eq!(
        parts(&harness)[0][2],
        40.0,
        "Home goes to the first minimum"
    );
    harness.press_key(Key::End);
    harness.frame();
    assert_eq!(
        400.0 - parts(&harness)[2][0],
        60.0,
        "End goes to the second minimum"
    );
}

#[test]
fn a_double_click_on_the_divider_resets_the_ratio() {
    let ratio = RwSignal::new(0.4);
    let mut harness = row_split(ratio.clone(), 0.0, 0.0);

    ratio.set(0.7);
    harness.frame();
    let middle = divider_middle(&harness);
    harness.click(middle);
    harness.click(middle);
    harness.frame();

    assert_eq!(ratio.get(), 0.4, "the ratio the split was built with");
    assert_eq!(parts(&harness)[0][2], 160.0);
}

#[test]
fn a_drag_damages_only_the_split() {
    let ratio = RwSignal::new(0.5);
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, {
        let ratio = ratio.clone();
        move || {
            panel().column().child(
                split(Direction::Row, panel(), panel())
                    .ratio(ratio.clone())
                    .height(px(100.0)),
            )
        }
    });
    harness.frame();
    harness.settle();

    let start = divider_middle(&harness);
    harness.button_down(start);
    harness.move_to(Point {
        x: start.x + 40.0,
        y: start.y,
    });
    let report = harness.frame();
    harness.button_up(Point {
        x: start.x + 40.0,
        y: start.y,
    });

    assert!(!report.damage.is_empty(), "the drag repainted");
    for rect in &report.damage {
        assert!(
            rect.y + rect.height <= 100,
            "the damage {rect:?} reaches below the split"
        );
    }
    harness.assert_matches_full_repaint();
}

#[test]
fn a_drag_on_a_padded_split_follows_the_pointer() {
    let ratio = RwSignal::new(0.5);
    let mut harness = Harness::new(WINDOW.width, WINDOW.height, 1.0, {
        let ratio = ratio.clone();
        move || {
            split(Direction::Row, panel(), panel())
                .ratio(ratio.clone())
                .padding(px(20.0))
        }
    });
    harness.frame();
    harness.settle();

    let start = divider_middle(&harness);
    harness.button_down(start);
    harness.move_to(Point {
        x: start.x + 1.0,
        y: start.y,
    });
    harness.button_up(Point {
        x: start.x + 1.0,
        y: start.y,
    });
    harness.frame();

    let moved = divider_middle(&harness).x - start.x;
    assert!(
        (moved - 1.0).abs() < 0.01,
        "the divider moved with the pointer, by {moved}"
    );
}

#[test]
fn a_quick_second_grab_after_a_drag_drags_again() {
    let ratio = RwSignal::new(0.5);
    let mut harness = row_split(ratio.clone(), 0.0, 0.0);

    let start = divider_middle(&harness);
    harness.button_down(start);
    harness.move_to(Point {
        x: start.x + 100.0,
        y: start.y,
    });
    harness.button_up(Point {
        x: start.x + 100.0,
        y: start.y,
    });
    harness.frame();
    let again = divider_middle(&harness);
    harness.button_down(again);
    harness.move_to(Point {
        x: again.x + 50.0,
        y: again.y,
    });
    harness.button_up(Point {
        x: again.x + 50.0,
        y: again.y,
    });
    harness.frame();

    assert_eq!(
        ratio.get(),
        0.875,
        "the second press dragged instead of resetting"
    );
}

#[test]
fn the_divider_is_a_focusable_splitter_named_resize() {
    let mut harness = row_split(RwSignal::new(0.5), 0.0, 0.0);
    let nodes = harness.app().semantics_nodes();
    let divider = nodes
        .iter()
        .find(|node| node.role == Role::Splitter)
        .expect("the divider reaches the accessibility tree");
    assert_eq!(divider.label.as_deref(), Some("Resize"));
    assert!(divider.focusable, "the keyboard reaches it");
}

#[test]
fn a_key_with_a_modifier_passes_the_divider_by() {
    let ratio = RwSignal::new(0.5);
    let mut harness = row_split(ratio.clone(), 0.0, 0.0);

    harness.press_key(Key::Tab);
    harness.frame();
    harness.press_key_with(
        Key::ArrowRight,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    harness.frame();

    assert_eq!(ratio.get(), 0.5, "the shortcut was left to the app");
}

#[test]
fn a_reset_keeps_to_the_minimums() {
    let ratio = RwSignal::new(0.1);
    let mut harness = row_split(ratio.clone(), 100.0, 0.0);

    let middle = divider_middle(&harness);
    harness.click(middle);
    harness.click(middle);
    harness.frame();

    assert_eq!(
        ratio.get(),
        0.25,
        "the reset asks for no less than the minimum"
    );
}
