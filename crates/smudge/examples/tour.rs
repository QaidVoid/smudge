//! A window to try the panes, the log and the text widgets by hand.
//!
//! Run with `cargo run --example tour -p smudge`, then:
//!
//! - drag the dividers, use the arrow keys, Home and End on a focused one,
//!   and double-click one to put it back where it started;
//! - watch the log grow at its end, scroll it up to stop following, and
//!   press "Follow" to jump back;
//! - drag across the paragraphs on the right, or click one and extend the
//!   selection with Shift and the arrow keys, then copy it;
//! - look at the monospace spans, whose bold and italic have no face of
//!   their own and are drawn heavier and slanted;
//! - type several lines in the notes at the bottom.

use std::rc::Rc;
use std::time::Duration;

use smudge::core::timer::every;
use smudge::prelude::*;
use smudge::widgets::{ScrollExt, virtual_list_keyed};

/// One span of a paragraph: its text, its weight, and whether it is italic
/// and monospace.
type Part = (&'static str, Option<FontWeight>, bool, bool);

/// The paragraphs on the right, as spans.
const PARAGRAPHS: &[&[Part]] = &[
    &[
        ("Panes. ", Some(FontWeight::BOLD), false, false),
        (
            "Each divider moves by drag or by the arrow keys once it has the focus, ",
            None,
            false,
            false,
        ),
        ("and a double click resets it.", None, true, false),
    ],
    &[
        ("Selection. ", Some(FontWeight::BOLD), false, false),
        (
            "A drag that starts in one paragraph and ends in another selects across both. ",
            None,
            false,
            false,
        ),
        (
            "Shift with the arrow keys moves the end of the selection, and Ctrl+C, or Cmd+C \
             on macOS, copies it.",
            None,
            false,
            false,
        ),
    ],
    &[
        ("Synthesis. ", Some(FontWeight::BOLD), false, false),
        ("The monospace face comes in ", None, false, false),
        ("regular", None, false, true),
        (" only, so ", None, false, false),
        ("bold", Some(FontWeight::BOLD), false, true),
        (", ", None, false, false),
        ("italic", None, true, true),
        (" and ", None, false, false),
        ("bold italic", Some(FontWeight::BOLD), true, true),
        (" are drawn from it.", None, false, false),
    ],
    &[
        ("Wrapping. ", Some(FontWeight::BOLD), false, false),
        (
            "Narrow this pane with the divider and every paragraph wraps again to the \
             width it is given, while the selection stays on the same characters.",
            None,
            false,
            false,
        ),
    ],
];

/// How often the log gains a line.
const LOG_PERIOD: Duration = Duration::from_millis(400);

fn main() -> Result<(), PlatformError> {
    run_with(
        WindowOptions {
            title: String::from("Smudge tour"),
            size: Size {
                width: 980.0,
                height: 640.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        || {
            split(Direction::Row, log_pane(), document_pane())
                .min_first(220.0)
                .min_second(320.0)
                .width(pct(100.0))
                .height(pct(100.0))
        },
        |app| app.set_theme(Theme::dark()),
    )
}

/// A log that gains a line on a timer and follows its end until scrolled up.
fn log_pane() -> Element<Panel> {
    let entries: RwSignal<Rc<[u32]>> = RwSignal::new(Rc::from([]));
    let following = RwSignal::new(true);
    let grow = entries.clone();
    every(LOG_PERIOD, move || {
        grow.update(|entries| {
            let next = entries.len() as u32;
            *entries = entries.iter().copied().chain([next]).collect();
        });
    })
    .detach();

    let counted = entries.clone();
    let jump = following.clone();
    let shown = following.clone();
    column()
        .padding(px(12.0))
        .gap(Size {
            width: 0.0,
            height: 8.0,
        })
        .child(
            row()
                .gap(Size {
                    width: 8.0,
                    height: 0.0,
                })
                .align_items(Align::Center)
                .child(label(move || {
                    let state = if shown.get() { "following" } else { "paused" };
                    format!("{} lines, {state}", counted.with(|entries| entries.len()))
                }))
                .child(button("Follow").on_press(move || jump.set(true))),
        )
        .child(
            virtual_list_keyed(
                move || entries.get(),
                |entry| *entry,
                24.0,
                |entry| log_line(*entry),
            )
            .follow_end(following)
            .width(pct(100.0))
            .grow(1.0)
            .border(1.0)
            .border_color(ColorToken::BORDER),
        )
}

/// One line of the log. Every fifth line carries a second line, so rows
/// differ in height.
fn log_line(entry: u32) -> Element<Panel> {
    let text = if entry % 5 == 4 {
        format!("{entry:05}  checkpoint\n{} lines so far", entry + 1)
    } else {
        format!("{entry:05}  tick")
    };
    panel()
        .padding_xy(Size {
            width: 8.0,
            height: 4.0,
        })
        .child(label(text))
}

/// The paragraphs in a selection scope above a notes area.
fn document_pane() -> Element<Split> {
    let keys: Rc<[usize]> = (0..PARAGRAPHS.len()).collect();
    let paragraphs = selection_scope(
        move || Rc::clone(&keys),
        |at: &usize| PARAGRAPHS[*at].iter().map(|part| part.0).collect(),
        || {
            PARAGRAPHS
                .iter()
                .enumerate()
                .fold(column(), |page, (at, parts)| {
                    page.child(paragraph(parts).selectable(at))
                })
                .padding(px(16.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
        },
    );
    let notes = RwSignal::new(String::new());
    split(
        Direction::Column,
        scroll(paragraphs),
        column().padding(px(12.0)).child(
            text_area(notes)
                .placeholder("Notes")
                .width(pct(100.0))
                .grow(1.0)
                .padding(px(8.0))
                .border(1.0)
                .border_color(ColorToken::BORDER),
        ),
    )
    .min_first(160.0)
    .min_second(100.0)
}

/// A paragraph of `parts`.
fn paragraph(parts: &'static [Part]) -> Element<RichText> {
    let spans: Rc<[Span]> = parts
        .iter()
        .map(|&(text, weight, italic, monospace)| {
            Span::new(
                text,
                SpanStyle {
                    weight,
                    italic,
                    monospace,
                    ..SpanStyle::default()
                },
            )
        })
        .collect();
    rich_text(spans)
}
