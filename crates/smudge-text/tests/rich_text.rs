//! Whole-frame behavior of the rich text paragraph, driven through the
//! headless harness with the bundled fonts.

use std::cell::Cell;
use std::rc::Rc;

use insta::assert_binary_snapshot;
use smudge_core::element::Element;
use smudge_core::geometry::{Color, Point};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Align, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_core::window::Cursor;
use smudge_test::Harness;
use smudge_text::{FontWeight, RichText, Span, SpanStyle, TextSystem, rich_text};

/// The window the paragraphs sit in.
const WINDOW: (f32, f32) = (320.0, 120.0);

/// Where the paragraph sits in the window, so its rectangle is not the
/// window's.
const INSET: f32 = 20.0;

/// A plain span of `text`.
fn plain(text: &str) -> Span {
    Span::new(text, SpanStyle::default())
}

/// A link span of `text` whose handler counts its runs in `clicks`.
fn link(text: &str, clicks: &Rc<Cell<u32>>) -> Span {
    let clicks = Rc::clone(clicks);
    Span::new(
        text,
        SpanStyle {
            link: Some(Rc::new(move || clicks.set(clicks.get() + 1))),
            ..SpanStyle::default()
        },
    )
}

/// Builds a harness that shows the spans `spans` returns, sized to their
/// text and inset from the window's top left.
fn harness(spans: impl Fn() -> Rc<[Span]> + 'static) -> Harness {
    shaped_harness(spans, |paragraph| paragraph)
}

/// Builds a harness like [`harness`] whose paragraph `shape` sizes.
fn shaped_harness(
    spans: impl Fn() -> Rc<[Span]> + 'static,
    shape: impl Fn(Element<RichText>) -> Element<RichText> + 'static,
) -> Harness {
    TextSystem::bundled_only().install();
    let spans = Rc::new(spans);
    let shape = Rc::new(shape);
    Harness::new(WINDOW.0, WINDOW.1, 1.0, move || {
        let spans = Rc::clone(&spans);
        panel()
            .column()
            .align_items(Align::Start)
            .padding(px(INSET))
            .child(shape(rich_text(move || spans())))
    })
}

/// The rectangle of the paragraph, as left, top, right and bottom, from
/// the layout dump.
fn paragraph_rect(harness: &Harness) -> (f32, f32, f32, f32) {
    let dump = harness.layout_dump();
    let line = dump
        .lines()
        .find(|line| line.contains("RichText ["))
        .expect("the layout holds the paragraph");
    let start = line
        .find("RichText [")
        .expect("the line names the paragraph")
        + "RichText [".len();
    let values: Vec<f32> = line[start..line.len() - 1]
        .split(", ")
        .map(|value| value.parse().expect("the rectangle is numbers"))
        .collect();
    (values[0], values[1], values[2], values[3])
}

/// A point on the first line of the paragraph, `x` pixels into it.
fn on_first_line(harness: &Harness, x: f32) -> Point {
    let (left, top, _, _) = paragraph_rect(harness);
    Point {
        x: left + x,
        y: top + 8.0,
    }
}

#[test]
fn a_color_only_change_repaints_without_layout_inside_the_paragraph() {
    let color = RwSignal::new(Color::rgb(20, 120, 220));
    let bound = color.clone();
    let mut harness = harness(move || {
        Rc::from([
            plain("Status: "),
            Span::new(
                "running",
                SpanStyle {
                    color: Some(bound.get().into()),
                    ..SpanStyle::default()
                },
            ),
        ])
    });
    harness.frame();

    color.set(Color::rgb(220, 60, 40));
    let report = harness.frame();

    assert!(
        !report.layout_ran,
        "a color change never lays the paragraph out"
    );
    assert!(!report.damage.is_empty(), "the color change shows");
    let (left, top, right, bottom) = paragraph_rect(&harness);
    for rect in &report.damage {
        assert!(
            rect.x as f32 >= left - 1.0
                && rect.y as f32 >= top - 1.0
                && (rect.x + rect.width) as f32 <= right + 1.0
                && (rect.y + rect.height) as f32 <= bottom + 1.0,
            "the damage {rect:?} reaches past the paragraph"
        );
    }
    harness.assert_matches_full_repaint();
}

#[test]
fn a_text_change_lays_the_paragraph_out() {
    let text = RwSignal::new(String::from("short"));
    let bound = text.clone();
    let mut harness = harness(move || Rc::from([plain(&bound.get())]));
    harness.frame();

    text.set(String::from("a much longer text"));
    let report = harness.frame();

    assert!(report.layout_ran, "a text change lays the paragraph out");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_click_on_a_link_runs_its_handler_once_and_a_click_beside_it_none() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = harness(move || Rc::from([link("guide", &shared), plain(" and more words")]));
    harness.frame();

    harness.click(on_first_line(&harness, 6.0));
    harness.frame();
    assert_eq!(
        clicks.get(),
        1,
        "the click on the link ran its handler once"
    );

    let (left, _, right, _) = paragraph_rect(&harness);
    harness.click(on_first_line(&harness, right - left - 4.0));
    harness.frame();
    assert_eq!(
        clicks.get(),
        1,
        "a click on the plain words ran the handler"
    );

    harness.assert_matches_full_repaint();
}

#[test]
fn a_press_that_leaves_the_link_before_its_release_runs_nothing() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = harness(move || Rc::from([link("guide", &shared), plain(" and more words")]));
    harness.frame();

    let (left, _, right, _) = paragraph_rect(&harness);
    harness.button_down(on_first_line(&harness, 6.0));
    harness.button_up(on_first_line(&harness, right - left - 4.0));
    harness.frame();

    assert_eq!(clicks.get(), 0, "a release off the link ran the handler");
}

#[test]
fn the_pointer_shows_the_pointing_cursor_over_a_link_only() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = harness(move || Rc::from([link("guide", &shared), plain(" and more words")]));
    harness.frame();

    harness.move_to(on_first_line(&harness, 6.0));
    harness.frame();
    assert_eq!(harness.app().cursor(), Cursor::Pointer, "over the link");

    let (left, _, right, _) = paragraph_rect(&harness);
    let report = {
        harness.move_to(on_first_line(&harness, right - left - 4.0));
        harness.frame()
    };
    assert_eq!(
        harness.app().cursor(),
        Cursor::Default,
        "over the plain words"
    );
    assert_eq!(report.nodes_painted, 0, "a cursor change paints nothing");
}

#[test]
fn a_paragraph_of_styled_spans_draws_as_designed() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = harness(move || {
        Rc::from([
            plain("Run "),
            Span::new(
                "cargo test",
                SpanStyle {
                    monospace: true,
                    ..SpanStyle::default()
                },
            ),
            plain(", read the "),
            link("guide", &shared),
            plain(", and mind the "),
            Span::new(
                "warnings",
                SpanStyle {
                    weight: Some(FontWeight::BOLD),
                    color: Some(Color::rgb(200, 60, 40).into()),
                    ..SpanStyle::default()
                },
            ),
            plain(" in "),
            Span::new(
                "italic",
                SpanStyle {
                    italic: true,
                    ..SpanStyle::default()
                },
            ),
            plain(" and "),
            Span::new(
                "bold italic",
                SpanStyle {
                    italic: true,
                    weight: Some(FontWeight::BOLD),
                    ..SpanStyle::default()
                },
            ),
            plain(" text that wraps across lines."),
        ])
    });
    harness.frame();

    assert_binary_snapshot!(".png", harness.png());
}

#[test]
fn a_click_inside_the_paragraph_but_off_its_text_runs_nothing() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = shaped_harness(
        move || Rc::from([link("guide", &shared)]),
        |paragraph| paragraph.width(px(260.0)).height(px(80.0)),
    );
    harness.frame();

    // Beside the link on its line, and below it, inside the paragraph's
    // rectangle both times.
    harness.click(on_first_line(&harness, 200.0));
    let (left, top, _, _) = paragraph_rect(&harness);
    let below = Point {
        x: left + 6.0,
        y: top + 60.0,
    };
    harness.click(below);
    harness.frame();
    assert_eq!(clicks.get(), 0, "a click off the text ran the link");

    harness.move_to(below);
    harness.frame();
    assert_eq!(harness.app().cursor(), Cursor::Default, "below the link");
}

#[test]
fn a_link_that_wraps_takes_a_click_on_its_second_line() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = shaped_harness(
        move || {
            Rc::from([link(
                "a link long enough to wrap onto a second line",
                &shared,
            )])
        },
        |paragraph| paragraph.width(px(120.0)),
    );
    harness.frame();

    let (left, top, _, bottom) = paragraph_rect(&harness);
    assert!(bottom - top > 30.0, "the link wraps");
    harness.click(Point {
        x: left + 6.0,
        y: top + 26.0,
    });
    harness.frame();
    assert_eq!(
        clicks.get(),
        1,
        "the second line of the link took the click"
    );
}

#[test]
fn a_press_on_plain_words_reaches_the_row_around_the_paragraph() {
    TextSystem::bundled_only().install();
    let clicks = Rc::new(Cell::new(0));
    let rows = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let row_presses = Rc::clone(&rows);
    let mut harness = Harness::new(WINDOW.0, WINDOW.1, 1.0, move || {
        let shared = Rc::clone(&shared);
        let row_presses = Rc::clone(&row_presses);
        panel()
            .column()
            .align_items(Align::Start)
            .padding(px(INSET))
            .on_press(move || row_presses.set(row_presses.get() + 1))
            .child(rich_text(move || {
                Rc::from([link("guide", &shared), plain(" and more words")])
            }))
    });
    harness.frame();

    let (left, _, right, _) = paragraph_rect(&harness);
    harness.click(on_first_line(&harness, right - left - 4.0));
    assert_eq!(rows.get(), 1, "the row took the press on the plain words");
    assert_eq!(clicks.get(), 0);

    harness.click(on_first_line(&harness, 6.0));
    assert_eq!(clicks.get(), 1, "the link took the press on it");
    assert_eq!(rows.get(), 1, "the row did not take the press on the link");
}

#[test]
fn a_theme_switch_repaints_a_link_in_the_new_accent_without_layout() {
    let clicks = Rc::new(Cell::new(0));
    let shared = Rc::clone(&clicks);
    let mut harness = harness(move || Rc::from([plain("Read the "), link("guide", &shared)]));
    harness.frame();

    harness.app().set_theme(Theme::dark());
    let report = harness.frame();

    assert!(
        !report.layout_ran,
        "a theme switch never lays the paragraph out"
    );
    assert_eq!(report.nodes_painted, 1, "the paragraph repaints");
    harness.assert_matches_full_repaint();
}

#[test]
fn a_link_whose_handler_replaces_the_spans_leaves_plain_text() {
    let opened = RwSignal::new(false);
    let mut harness = harness(move || {
        let opened = opened.clone();
        if opened.get() {
            Rc::from([plain("Opened.")])
        } else {
            Rc::from([Span::link("open", move || opened.set(true))])
        }
    });
    harness.frame();

    let at = on_first_line(&harness, 6.0);
    harness.move_to(at);
    harness.frame();
    assert_eq!(harness.app().cursor(), Cursor::Pointer, "over the link");
    harness.click(at);
    harness.frame();

    assert_eq!(
        harness.app().cursor(),
        Cursor::Default,
        "the link under the pointer is gone"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_link_that_relabels_itself_keeps_the_pointing_cursor_under_a_still_pointer() {
    let clicks = RwSignal::new(0u32);
    let mut harness = harness(move || {
        let clicks = clicks.clone();
        let label = format!("clicked {} times", clicks.get());
        Rc::from([Span::link(label, move || {
            clicks.update(|count| *count += 1)
        })])
    });
    harness.frame();

    let at = on_first_line(&harness, 6.0);
    harness.move_to(at);
    harness.click(at);
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.app().cursor(),
        Cursor::Pointer,
        "the relabelled link is still under the pointer"
    );
}

#[test]
fn a_link_that_moves_under_a_still_pointer_shows_the_pointing_cursor() {
    TextSystem::bundled_only().install();
    let gap = RwSignal::new(40.0f32);
    let bound = gap.clone();
    let mut harness = Harness::new(WINDOW.0, WINDOW.1, 1.0, move || {
        let bound = bound.clone();
        panel()
            .column()
            .align_items(Align::Start)
            .padding(px(INSET))
            .child(panel().height(move || px(bound.get())))
            .child(rich_text(Rc::from([Span::link("guide", || {})])))
    });
    harness.frame();

    // The pointer rests where the link lands once the gap above it closes.
    harness.move_to(Point {
        x: INSET + 6.0,
        y: INSET + 8.0,
    });
    harness.frame();
    assert_eq!(
        harness.app().cursor(),
        Cursor::Default,
        "the gap is under it"
    );

    gap.set(0.0);
    harness.frame();
    assert_eq!(
        harness.app().cursor(),
        Cursor::Pointer,
        "the link moved under the pointer"
    );
}
