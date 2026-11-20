//! The canvas: what it draws, where, and when it draws again.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::geometry::{Color, Corners, Point, Rect};
use smudge_core::path::{Path, Stroke};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Length, px};
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;
use smudge_widgets::{canvas, stack};

/// The color the tests draw with.
const INK: Color = Color::rgb(200, 30, 30);

/// Returns whether the pixel at `x`, `y` is the ink color.
fn inked(harness: &Harness, x: u32, y: u32) -> bool {
    harness.pixel(x, y) == Some([INK.r, INK.g, INK.b, 255])
}

#[test]
fn a_canvas_draws_in_its_own_coordinates_and_knows_its_size() {
    let mut harness = Harness::new(100.0, 80.0, 1.0, || {
        panel().padding(px(10.0)).child(
            canvas(|painter| {
                let size = painter.size();
                painter.fill_rect(
                    Rect::new(
                        size.width - 10.0,
                        size.height - 10.0,
                        size.width,
                        size.height,
                    ),
                    Corners::ZERO,
                    INK,
                );
            })
            .width(px(40.0))
            .height(px(30.0)),
        )
    });
    harness.frame();

    assert!(
        inked(&harness, 45, 35),
        "the bottom right corner of the canvas"
    );
    assert!(!inked(&harness, 39, 29), "and nothing above or left of it");
    assert!(!inked(&harness, 50, 40), "nor past the canvas");
}

#[test]
fn a_canvas_draws_again_when_a_signal_it_read_changes() {
    let at = RwSignal::new(20.0_f32);
    let runs = Rc::new(Cell::new(0u32));
    let (moved, counted) = (at.clone(), Rc::clone(&runs));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let (moved, counted) = (moved.clone(), Rc::clone(&counted));
        panel().child(
            canvas(move |painter| {
                counted.set(counted.get() + 1);
                painter.fill(
                    &Path::circle(
                        Point {
                            x: moved.get(),
                            y: 30.0,
                        },
                        8.0,
                    ),
                    INK,
                );
            })
            .width(Length::Percent(100.0))
            .height(px(60.0)),
        )
    });
    harness.frame();
    assert!(inked(&harness, 20, 30), "the first frame draws it");
    harness.settle();
    let settled = runs.get();
    harness.frame();
    assert_eq!(runs.get(), settled, "an idle canvas draws nothing again");

    at.set(70.0);
    harness.frame();
    assert_eq!(runs.get(), settled + 1, "a change draws once");
    assert!(inked(&harness, 70, 30), "the circle moved");
    assert!(!inked(&harness, 20, 30), "and left nothing behind");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_moving_stroke_repaints_exactly_what_a_full_repaint_draws() {
    TextSystem::bundled_only().install();
    let angle = RwSignal::new(0.0_f32);
    let turned = angle.clone();
    let mut harness = Harness::new(120.0, 120.0, 1.5, move || {
        let turned = turned.clone();
        panel().child(
            canvas(move |painter| {
                let center = Point { x: 60.0, y: 60.0 };
                let angle = turned.get();
                let tip = Point {
                    x: center.x + 50.0 * angle.cos(),
                    y: center.y + 50.0 * angle.sin(),
                };
                painter.stroke(
                    &Path::line(center, tip),
                    Stroke::new(5.0, INK).with_cap(smudge_core::path::LineCap::Round),
                );
                painter.stroke(&Path::circle(center, 30.0), Stroke::new(1.5, Color::BLACK));
                painter.text("dial", Point { x: 4.0, y: 4.0 }, 12.0, Color::BLACK);
            })
            .width(px(120.0))
            .height(px(120.0)),
        )
    });
    harness.frame();
    for step in 1..12 {
        angle.set(step as f32 * 0.55);
        harness.frame();
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn a_path_repainted_in_part_matches_a_full_repaint() {
    let shown = RwSignal::new(false);
    let toggled = shown.clone();
    let mut harness = Harness::new(160.0, 120.0, 1.25, move || {
        let toggled = toggled.clone();
        stack()
            .width(px(160.0))
            .height(px(120.0))
            .child(canvas(|painter| {
                let size = painter.size();
                painter.stroke(
                    &Path::new().move_to(Point { x: 3.3, y: 7.1 }).cubic_to(
                        Point { x: 40.7, y: 140.0 },
                        Point { x: 120.2, y: -30.0 },
                        Point {
                            x: size.width - 2.9,
                            y: size.height - 5.4,
                        },
                    ),
                    Stroke::new(7.3, INK),
                );
                painter.fill(
                    &Path::circle(Point { x: 81.7, y: 59.3 }, 37.9),
                    Color::rgba(20, 90, 200, 140),
                );
            }))
            .child(
                panel()
                    .width(px(13.0))
                    .height(px(11.0))
                    .margin(px(51.0))
                    .background(move || {
                        smudge_core::theme::ColorValue::from(if toggled.get() {
                            Color::rgba(250, 200, 40, 120)
                        } else {
                            Color::TRANSPARENT
                        })
                    }),
            )
    });
    harness.frame();
    for _ in 0..4 {
        shown.update(|shown| *shown = !*shown);
        harness.frame();
        harness.assert_matches_full_repaint();
    }
}

#[test]
fn text_on_a_canvas_reports_its_size_and_draws_ink() {
    TextSystem::bundled_only().install();
    let measured = Rc::new(Cell::new(0.0_f32));
    let size = Rc::clone(&measured);
    let mut harness = Harness::new(120.0, 40.0, 1.0, move || {
        let size = Rc::clone(&size);
        panel().child(
            canvas(move |painter| {
                let drawn = painter.text("Grid", Point { x: 10.0, y: 6.0 }, 16.0, Color::BLACK);
                size.set(drawn.width);
            })
            .width(px(120.0))
            .height(px(40.0)),
        )
    });
    harness.frame();

    assert!(measured.get() > 20.0, "{}", measured.get());
    let ink = harness.ink_bounds().expect("the text drew");
    assert!(ink.x0 >= 10.0 && ink.y0 >= 6.0, "{ink:?}");
}

#[test]
fn a_canvas_draws_once_as_it_shows_and_once_per_resize() {
    let width = RwSignal::new(40.0_f32);
    let runs = Rc::new(Cell::new(0u32));
    let (sized, counted) = (width.clone(), Rc::clone(&runs));
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let counted = Rc::clone(&counted);
        let sized = sized.clone();
        panel().child(
            canvas(move |painter| {
                counted.set(counted.get() + 1);
                let size = painter.size();
                painter.fill_rect(
                    Rect::new(size.width - 4.0, 0.0, size.width, 4.0),
                    Corners::ZERO,
                    INK,
                );
            })
            .width(move || px(sized.get()))
            .height(px(20.0)),
        )
    });
    harness.settle();
    assert_eq!(runs.get(), 1, "showing draws once");
    assert!(inked(&harness, 38, 2));

    width.set(80.0);
    harness.frame();
    assert_eq!(
        harness.frame().nodes_painted,
        0,
        "the drawing a resize made is not painted twice"
    );
    harness.settle();
    assert_eq!(runs.get(), 2, "a resize draws once");
    assert!(inked(&harness, 78, 2), "at the new size");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_canvas_draws_again_in_the_colors_of_a_new_theme() {
    let mut harness = Harness::new(60.0, 40.0, 1.0, || {
        panel().child(
            canvas(|painter| {
                let color = painter.theme().color(ColorToken::ACCENT);
                painter.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Corners::ZERO, color);
            })
            .width(px(20.0))
            .height(px(20.0)),
        )
    });
    harness.settle();
    let accent = |theme: &Theme| {
        let color = theme.color(ColorToken::ACCENT);
        [color.r, color.g, color.b, 255]
    };
    assert_eq!(harness.pixel(10, 10), Some(accent(&Theme::light())));

    harness.app().set_theme(Theme::dark());
    harness.settle();
    assert_eq!(harness.pixel(10, 10), Some(accent(&Theme::dark())));
    harness.assert_matches_full_repaint();
}

#[test]
fn a_canvas_that_asks_for_the_theme_only_later_draws_in_the_current_one() {
    let fancy = RwSignal::new(false);
    let read = fancy.clone();
    let mut harness = Harness::new(60.0, 40.0, 1.0, move || {
        let read = read.clone();
        panel().child(
            canvas(move |painter| {
                let color = if read.get() {
                    painter.theme().color(ColorToken::ACCENT)
                } else {
                    INK
                };
                painter.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Corners::ZERO, color);
            })
            .width(px(20.0))
            .height(px(20.0)),
        )
    });
    harness.settle();
    harness.app().set_theme(Theme::dark());
    harness.settle();

    fancy.set(true);
    harness.settle();
    let accent = Theme::dark().color(ColorToken::ACCENT);
    assert_eq!(
        harness.pixel(10, 10),
        Some([accent.r, accent.g, accent.b, 255])
    );
}

#[test]
fn a_drawing_past_the_canvas_stays_inside_it() {
    let width = RwSignal::new(60.0_f32);
    let sized = width.clone();
    let mut harness = Harness::new(100.0, 60.0, 1.0, move || {
        let sized = sized.clone();
        panel().child(
            canvas(|painter| {
                painter.fill(&Path::circle(Point { x: 50.0, y: 20.0 }, 8.0), INK);
            })
            .width(move || px(sized.get()))
            .height(px(40.0)),
        )
    });
    harness.settle();
    assert!(
        inked(&harness, 50, 20),
        "the mark shows inside a wide canvas"
    );

    width.set(40.0);
    harness.settle();
    assert!(!inked(&harness, 50, 20), "and is cut off past a narrow one");
    assert!(!inked(&harness, 50, 45), "nothing spills below it either");
    harness.assert_matches_full_repaint();
}
