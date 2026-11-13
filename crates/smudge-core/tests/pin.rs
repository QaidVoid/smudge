//! Whole-frame behavior of `Element::pin_child`, through the builder and
//! its binding.

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::{Element, show};
use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Length, px};
use smudge_core::widget::{Panel, panel};

/// The settings for the app under test.
fn config() -> AppConfig {
    AppConfig {
        viewport: Size {
            width: 200.0,
            height: 200.0,
        },
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint: RepaintMode::Damage,
    }
}

/// The signals a pinned column reads.
#[derive(Clone)]
struct Knobs {
    /// The height of the first row.
    first: RwSignal<f32>,
    /// The pin of the column.
    pin: RwSignal<Option<(usize, f32)>>,
    /// The scroll offset of the view around the column.
    offset: RwSignal<f32>,
    /// Whether the view around the column is hidden.
    hidden: RwSignal<bool>,
}

impl Knobs {
    fn new() -> Knobs {
        Knobs {
            first: RwSignal::new(20.0),
            pin: RwSignal::new(Some((1, 50.0))),
            offset: RwSignal::new(0.0),
            hidden: RwSignal::new(false),
        }
    }
}

/// A view with a scroll offset around a column of three rows that pins one
/// of them.
fn pinned_view(knobs: &Knobs) -> Element<Panel> {
    let (first, pin, offset, hidden) = (
        knobs.first.clone(),
        knobs.pin.clone(),
        knobs.offset.clone(),
        knobs.hidden.clone(),
    );
    panel()
        .height(px(200.0))
        .hidden(move || hidden.get())
        .scroll_offset(move || Point {
            x: 0.0,
            y: offset.get(),
        })
        .child(
            Element::new(Panel)
                .column()
                .pin_child(move || pin.get())
                .child(panel().height(move || Length::px(first.get())))
                .child(panel().height(px(20.0)))
                .child(panel().height(px(20.0))),
        )
}

/// Returns the window rectangles of the visible nodes at `depth`.
fn rects_at(app: &App, depth: usize) -> Vec<Rect> {
    let mut rects = Vec::new();
    app.visit_nodes(|at, _, rect| {
        if at == depth {
            rects.push(rect);
        }
    });
    rects
}

/// The depth of the rows: under the root, the view and the column.
const ROWS: usize = 3;

#[test]
fn a_bound_pin_keeps_its_child_in_place_while_a_row_above_grows() {
    let knobs = Knobs::new();
    let built = knobs.clone();
    let mut app = App::new(config(), move || panel().child(pinned_view(&built)));
    let _ = app.frame();
    assert_eq!(rects_at(&app, ROWS)[1].y0, 50.0);

    knobs.first.set(45.0);
    let _ = app.frame();
    let rows = rects_at(&app, ROWS);
    assert_eq!(rows[1].y0, 50.0, "the pinned row stays put");
    assert_eq!(
        (rows[0].y0, rows[0].y1),
        (5.0, 50.0),
        "the grown row stacks upward"
    );
    assert_eq!(rows[2].y0, 70.0);
}

#[test]
fn a_pin_composes_with_the_scroll_offset_of_a_parent() {
    let knobs = Knobs::new();
    let built = knobs.clone();
    let mut app = App::new(config(), move || panel().child(pinned_view(&built)));
    let _ = app.frame();

    knobs.offset.set(10.0);
    let _ = app.frame();
    assert_eq!(
        rects_at(&app, ROWS)[1].y0,
        40.0,
        "the parent's offset moves the pinned place with the content"
    );
}

#[test]
fn a_new_bound_pin_moves_the_rows_without_a_layout_pass() {
    let knobs = Knobs::new();
    let built = knobs.clone();
    let mut app = App::new(config(), move || panel().child(pinned_view(&built)));
    let _ = app.frame();

    knobs.pin.set(Some((2, 0.0)));
    let _ = app.frame();
    assert!(!app.stats().layout_ran);
    assert_eq!(rects_at(&app, ROWS)[2].y0, 0.0);
}

#[test]
fn a_pin_under_a_hidden_view_lands_when_the_view_shows() {
    let knobs = Knobs::new();
    knobs.hidden.set(true);
    let built = knobs.clone();
    let mut app = App::new(config(), move || panel().child(pinned_view(&built)));
    let _ = app.frame();
    assert!(
        rects_at(&app, ROWS).is_empty(),
        "nothing shows while hidden"
    );

    knobs.first.set(45.0);
    let _ = app.frame();
    knobs.hidden.set(false);
    let _ = app.frame();
    let rows = rects_at(&app, ROWS);
    assert_eq!(
        rows[1].y0, 50.0,
        "the pin applies to the first visible pass"
    );
    assert_eq!(rows[0].y0, 5.0);
}

#[test]
fn a_removed_pinning_node_leaves_the_tree_consistent_and_remounts_pinned() {
    let knobs = Knobs::new();
    let mounted = RwSignal::new(true);
    let built = knobs.clone();
    let visible = mounted.clone();
    let mut app = App::new(config(), move || {
        let built = built.clone();
        let visible = visible.clone();
        panel().child(show(move || visible.get(), move || pinned_view(&built)))
    });
    let _ = app.frame();

    mounted.set(false);
    let _ = app.frame();
    knobs.first.set(45.0);
    let _ = app.frame();

    mounted.set(true);
    let _ = app.frame();
    // Under the root, the slot, the view and the column.
    let rows = rects_at(&app, ROWS + 1);
    assert_eq!(rows[1].y0, 50.0);
    assert_eq!(rows[0].y0, 5.0);
}
