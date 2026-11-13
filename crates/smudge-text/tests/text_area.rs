//! Multi-line editing, through the headless harness and the editor
//! directly.
//!
//! The harness tests cover what a user does with an area: pressing Enter,
//! pasting multi-line text, moving between lines and pages, and typing past
//! the area's height. The editor-level tests cover the vertical movement
//! rules over paragraphs, which only parley's geometry can exercise.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::clipboard::{Clipboard, MemoryClipboard};
use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{Handled, InputEvent, Key, Modifiers};
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::semantics::Role;
use smudge_core::style::px;
use smudge_test::Harness;
use smudge_text::{Editor, TextAreaExt, TextStyle, text_area};

/// The viewport of a plain area harness, in logical pixels.
const VIEWPORT: (f32, f32) = (200.0, 100.0);

/// How long the caret stays visible or hidden in one blink, the same period
/// the area uses.
const BLINK: Duration = Duration::from_millis(530);

/// Mounts an area bound to a fresh signal, and returns the harness and the
/// signal.
fn mounted(initial: &str) -> (Harness, RwSignal<String>) {
    let value = RwSignal::new(String::from(initial));
    let bound = value.clone();
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        text_area(bound.clone())
            .width(px(VIEWPORT.0))
            .height(px(VIEWPORT.1))
    });
    harness.frame();
    (harness, value)
}

/// Clicks in the middle of the area, which focuses it, and settles.
fn focus(harness: &mut Harness) {
    harness.click(Point {
        x: 60.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.settle();
}

/// The absolute rectangle of the area node.
fn area_rect(harness: &mut Harness) -> Rect {
    let mut rect = None;
    harness.app().visit_nodes(|_depth, type_name, node_rect| {
        if type_name.rsplit("::").next() == Some("TextArea") {
            rect = Some(node_rect);
        }
    });
    rect.expect("the area lays out")
}

#[test]
fn enter_inserts_a_line_break_and_does_not_reach_a_parent() {
    let hits = Rc::new(Cell::new(0u32));
    let value = RwSignal::new(String::new());
    let bound = value.clone();
    let counted = Rc::clone(&hits);
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        smudge_core::widget::panel()
            .on_key(move |event| {
                if event.pressed {
                    counted.set(counted.get() + 1);
                }
                Handled::No
            })
            .child(
                text_area(bound.clone())
                    .width(px(VIEWPORT.0))
                    .height(px(VIEWPORT.1)),
            )
    });
    harness.frame();
    focus(&mut harness);

    harness.type_text("first");
    harness.press_key(Key::Enter);
    harness.type_text("second");

    assert_eq!(value.get(), "first\nsecond", "Enter inserts a line break");
    assert_eq!(
        hits.get(),
        0,
        "Enter is handled by the area and never reaches the parent"
    );

    harness.press_key(Key::Tab);
    assert_eq!(
        hits.get(),
        1,
        "a key the area does not take still bubbles, so the parent was \
         listening"
    );
}

#[test]
fn editing_across_lines_edits_the_value() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("ab");
    harness.press_key(Key::Enter);
    harness.type_text("cd");
    assert_eq!(value.get(), "ab\ncd");

    harness.press_key(Key::Backspace);
    assert_eq!(value.get(), "ab\nc", "backspace removes from the last line");

    harness.press_key(Key::Backspace);
    assert_eq!(
        value.get(),
        "ab\n",
        "a second backspace removes the last character of the empty line \
         above"
    );
}

#[test]
fn a_paste_keeps_its_line_breaks() {
    let (mut harness, value) = mounted("");
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    focus(&mut harness);

    clipboard.set(String::from("one\ntwo\r\nthree"));
    let command = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::Character(String::from("v")), command);

    assert_eq!(
        value.get(),
        "one\ntwo\nthree",
        "a multi-line paste keeps every line break it carried, a carriage \
         return and line feed pair as one"
    );
}

#[test]
fn up_moves_to_the_line_above_and_down_bring_the_caret_back() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("aa");
    harness.press_key(Key::Enter);
    harness.type_text("aa");

    harness.press_key(Key::ArrowUp);
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "aaX\naa",
        "Up keeps the column, which the end of the first line holds"
    );

    harness.press_key(Key::ArrowDown);
    harness.type_text("Y");
    assert_eq!(
        value.get(),
        "aaX\naaY",
        "Down keeps the column, which is now the end of the second line"
    );
}

#[test]
fn up_at_the_first_line_and_down_at_the_last_line_stay() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("one");
    for _ in 0..3 {
        harness.press_key(Key::ArrowUp);
    }
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "Xone",
        "Up past the first line lands at the start of the text"
    );

    for _ in 0..3 {
        harness.press_key(Key::ArrowDown);
    }
    harness.type_text("!");
    assert_eq!(
        value.get(),
        "Xone!",
        "Down past the last line lands at the end of the text"
    );
}

#[test]
fn page_down_moves_a_page_of_lines() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    for line in 0..24 {
        harness.type_text(&format!("line{line}"));
        if line + 1 < 24 {
            harness.press_key(Key::Enter);
        }
    }
    harness.press_key_with(Key::Home, command());
    harness.settle();

    harness.press_key(Key::PageDown);
    harness.type_text("X");

    let text = value.get();
    let lines: Vec<&str> = text.split('\n').collect();
    let landing = lines
        .iter()
        .position(|line| line.starts_with('X'))
        .expect("the caret typed its X on one of the lines");
    assert!(
        (2..=6).contains(&landing),
        "Page Down moved the caret a page of lines down, which lands \
         between the second and the seventh line at this height, and it \
         landed on line {landing}"
    );
}

#[test]
fn the_caret_stays_visible_while_typing_past_the_height() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    for line in 0..12 {
        harness.type_text(&format!("row{line}"));
        harness.press_key(Key::Enter);
    }
    harness.settle();
    assert_eq!(
        value.get().matches('\n').count(),
        12,
        "every typed line break is in the value"
    );

    let blink = harness.advance(BLINK);
    assert_eq!(
        blink.damage.len(),
        1,
        "the blink damages one rectangle while the area is scrolled"
    );
    let caret = blink.damage[0];
    assert!(
        caret.y + caret.height <= VIEWPORT.1 as u32,
        "the caret of the last typed line scrolled into the area: {caret:?}"
    );
}

#[test]
fn an_area_measures_its_min_lines_floor() {
    let value = RwSignal::new(String::new());
    let bound = value.clone();
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        smudge_core::widget::panel()
            .column()
            .child(text_area(bound.clone()).width(px(VIEWPORT.0)).min_lines(4))
    });
    harness.frame();
    harness.settle();

    let line = TextStyle::default().size * TextStyle::default().line_height;
    let height = area_rect(&mut harness).height();
    assert!(
        (height - 4.0 * line).abs() < 0.5,
        "the area measures four lines tall: {height} against {}",
        4.0 * line
    );
}

#[test]
fn an_outside_write_replaces_the_text_and_moves_the_caret_to_the_end() {
    let (mut harness, value) = mounted("draft");
    focus(&mut harness);

    value.set(String::from("first\nsecond\n"));
    harness.settle();
    harness.type_text("third");
    assert_eq!(
        value.get(),
        "first\nsecond\nthird",
        "typing after an outside write continues at the new end"
    );
}

#[test]
fn up_and_down_keep_the_column_across_paragraphs() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_text("aaaaaa\nbb\naaaaaa");

    // From the end of the last line, two ups land at the end of the short
    // middle line and then at the end of the first, whose width the caret
    // remembers; two downs bring the caret back to the end of the last.
    editor.move_up();
    editor.move_up();
    editor.insert("X");
    assert_eq!(
        editor.text(),
        "aaaaaaX\nbb\naaaaaa",
        "Up keeps the column the first move remembered"
    );

    editor.move_down();
    editor.move_down();
    editor.insert("Y");
    assert_eq!(
        editor.text(),
        "aaaaaaX\nbb\naaaaaaY",
        "Down returns the caret to the end the column names"
    );
}

#[test]
fn moving_lines_jumps_several_lines_at_once() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_text("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");

    editor.move_to_start();
    editor.move_lines(4);
    editor.insert("X");
    assert_eq!(
        editor.text(),
        "0\n1\n2\n3\nX4\n5\n6\n7\n8\n9",
        "a positive count moves down, keeping the column of the start"
    );

    editor.move_lines(-8);
    editor.insert("Y");
    assert_eq!(
        editor.text(),
        "Y0\n1\n2\n3\nX4\n5\n6\n7\n8\n9",
        "a count past the first line lands at the start of the text"
    );
}

#[test]
fn wrapping_at_a_width_makes_the_text_taller_than_one_line() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_wrap_width(Some(60.0));
    editor.set_text("word word word");

    let line = TextStyle::default().size * TextStyle::default().line_height;
    assert!(
        editor.layout_size().height > 2.0 * line,
        "three words do not fit one line at 60 logical pixels: {:?}",
        editor.layout_size()
    );
}

#[test]
fn shift_up_extends_the_selection_over_lines() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_text("first\nsecond");

    editor.move_to_end();
    editor.extend_up();
    assert_eq!(
        editor.selected_text(),
        Some(String::from("\nsecond")),
        "extending up selects from the caret back over the line break"
    );
}

/// The modifiers of the platform's command key.
fn command() -> Modifiers {
    #[cfg(target_os = "macos")]
    {
        Modifiers {
            meta: true,
            ..Modifiers::default()
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }
}

#[test]
fn home_and_end_move_within_the_line_and_command_reaches_the_text_ends() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("first");
    harness.press_key(Key::Enter);
    harness.type_text("second");

    harness.press_key(Key::Home);
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "first\nXsecond",
        "Home goes to the start of the caret's line"
    );

    harness.press_key(Key::ArrowUp);
    harness.press_key(Key::End);
    harness.type_text("Y");
    assert_eq!(
        value.get(),
        "firstY\nXsecond",
        "End goes to the end of the caret's line"
    );

    harness.press_key_with(Key::End, command());
    harness.type_text("Z");
    assert_eq!(
        value.get(),
        "firstY\nXsecondZ",
        "the command key with End goes to the end of the text"
    );

    harness.press_key_with(Key::Home, command());
    harness.type_text("W");
    assert_eq!(
        value.get(),
        "WfirstY\nXsecondZ",
        "the command key with Home goes to the start of the text"
    );
}

#[test]
fn shift_home_selects_to_the_start_of_the_line() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("one");
    harness.press_key(Key::Enter);
    harness.type_text("two");
    harness.press_key_with(
        Key::Home,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "one\nX",
        "Shift and Home select the caret's line only"
    );
}

#[test]
fn home_and_end_stay_on_the_visual_line_of_a_wrapped_paragraph() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_wrap_width(Some(60.0));
    editor.set_text("word word word");

    // One word per visual line at this width: the caret goes to the second
    // line, then to its start and its end.
    editor.move_to_start();
    editor.move_down();
    editor.move_right();
    editor.move_to_line_start();
    editor.insert("X");
    assert_eq!(
        editor.text(),
        "word Xword word",
        "Home stops at the start of the wrapped line, not the paragraph"
    );

    editor.move_to_line_end();
    editor.insert("Y");
    assert_eq!(
        editor.text(),
        "word Xword Yword",
        "End stops at the end of the wrapped line, not the paragraph"
    );
}

#[test]
fn an_area_without_min_lines_is_one_line_tall() {
    let value = RwSignal::new(String::new());
    let bound = value.clone();
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        smudge_core::widget::panel()
            .column()
            .child(text_area(bound.clone()).width(px(VIEWPORT.0)))
    });
    harness.frame();
    harness.settle();

    let line = TextStyle::default().size * TextStyle::default().line_height;
    let height = area_rect(&mut harness).height();
    assert!(
        (height - line).abs() < 0.5,
        "the area measures one line tall by default: {height} against {line}"
    );
}

#[test]
fn a_carriage_return_line_feed_is_one_line_break() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value.clone());
    editor.set_style(&TextStyle::default());
    editor.set_text("a\r\nb\rc");

    assert_eq!(
        editor.text(),
        "a\nb\nc",
        "a pair and a lone carriage return each become one line feed"
    );
    assert_eq!(value.get(), "a\nb\nc", "the bound value follows the editor");
    let line = TextStyle::default().size * TextStyle::default().line_height;
    let height = editor.layout_size().height;
    assert!(
        (height - 3.0 * line).abs() < 0.5,
        "three lines show, with no empty line after a break: {height}"
    );

    editor.insert("\r\n");
    editor.paste("x\r\ny");
    assert_eq!(editor.text(), "a\nb\nc\nx\ny");
}

#[test]
fn a_paste_of_windows_line_breaks_shows_no_empty_lines() {
    let (mut harness, value) = mounted("");
    let mut clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    focus(&mut harness);

    clipboard.set(String::from("one\r\ntwo"));
    harness.press_key_with(Key::Character(String::from("v")), command());
    assert_eq!(value.get(), "one\ntwo");

    harness.press_key(Key::ArrowUp);
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "oneX\ntwo",
        "Up from the second line reaches the first, with no line between"
    );
}

#[test]
fn the_wheel_scrolls_the_text_without_moving_the_caret() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);
    for line in 0..12 {
        harness.type_text(&format!("row{line}"));
        harness.press_key(Key::Enter);
    }
    harness.settle();

    harness.move_to(Point { x: 60.0, y: 50.0 });
    harness.event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: -1000.0 },
    });
    harness.settle();

    // The first line is back at the top, so a click there lands on it.
    harness.click(Point { x: 100.0, y: 5.0 });
    harness.settle();
    harness.type_text("X");
    assert!(
        value.get().starts_with("row0X\n"),
        "the wheel scrolled back to the first line: {:?}",
        value.get()
    );
}

#[test]
fn enter_with_a_command_modifier_bubbles_and_inserts_nothing() {
    let hits = Rc::new(Cell::new(0u32));
    let value = RwSignal::new(String::new());
    let bound = value.clone();
    let counted = Rc::clone(&hits);
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        smudge_core::widget::panel()
            .on_key(move |event| {
                if event.pressed && event.key == Key::Enter {
                    counted.set(counted.get() + 1);
                }
                Handled::No
            })
            .child(
                text_area(bound.clone())
                    .width(px(VIEWPORT.0))
                    .height(px(VIEWPORT.1)),
            )
    });
    harness.frame();
    focus(&mut harness);
    harness.type_text("note");

    for modifiers in [
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
        Modifiers {
            meta: true,
            ..Modifiers::default()
        },
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
    ] {
        harness.press_key_with(Key::Enter, modifiers);
    }
    assert_eq!(value.get(), "note", "no line break was inserted");
    assert_eq!(hits.get(), 3, "every combination reached the parent");

    harness.press_key_with(
        Key::Enter,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(
        value.get(),
        "note\n",
        "Shift and Enter still break the line"
    );
    assert_eq!(hits.get(), 3);
}

#[test]
fn an_area_reports_a_multi_line_text_role() {
    let (mut harness, _value) = mounted("one\ntwo");
    harness.settle();
    let roles: Vec<Role> = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .map(|node| node.role)
        .collect();
    assert!(
        roles.contains(&Role::MultilineTextInput),
        "the area is announced as a multi-line field: {roles:?}"
    );
}

#[test]
fn a_caret_round_trip_ends_the_undo_group() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    for character in ["h", "e", "l", "l", "o"] {
        editor.insert(character);
    }
    editor.move_left();
    editor.move_right();
    editor.insert("x");

    editor.undo();
    assert_eq!(
        editor.text(),
        "hello",
        "the letter typed after the caret moved is a step of its own"
    );
}

#[test]
fn down_over_soft_wraps_keeps_the_column_across_lines_of_different_lengths() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_text("aaaaaa");
    let word = editor.layout_size().width;
    // Three visual lines of one word each: long, short, long.
    editor.set_wrap_width(Some(word + 2.0));
    editor.set_text("aaaaaa b aaaaaa");
    let line = TextStyle::default().size * TextStyle::default().line_height;
    assert!(
        (editor.layout_size().height - 3.0 * line).abs() < 0.5,
        "the text wraps to three lines at this width"
    );

    editor.move_to_start();
    for _ in 0..4 {
        editor.move_right();
    }
    let column = editor.cursor_geometry(1.0).expect("a caret").x0;

    editor.move_down();
    let short = editor.cursor_geometry(1.0).expect("a caret");
    assert!(
        short.x0 < column - 1.0 && short.y0 >= line - 0.5,
        "the short middle line pulls the caret left: {short:?} from {column}"
    );

    editor.move_down();
    let landed = editor.cursor_geometry(1.0).expect("a caret");
    assert!(
        (landed.x0 - column).abs() < 1.0 && landed.y0 >= 2.0 * line - 0.5,
        "the third line gets the first line's column back: {landed:?} \
         against {column}"
    );
}

#[test]
fn up_over_a_soft_wrap_lands_on_the_wrapped_line() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_wrap_width(Some(60.0));
    editor.set_text("word word word");

    editor.move_to_end();
    editor.move_up();
    editor.move_to_line_start();
    editor.insert("X");
    assert_eq!(
        editor.text(),
        "word Xword word",
        "Up from the last visual line lands on the middle one"
    );
}

#[test]
fn a_selected_empty_line_paints_a_highlight() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_text("a\n\nb");
    editor.select_all();

    let line = TextStyle::default().size * TextStyle::default().line_height;
    let rects = editor.selection_geometry();
    assert!(
        rects
            .iter()
            .any(|rect| rect.y0 >= line - 0.5 && rect.y1 <= 2.0 * line + 0.5 && rect.width() > 0.0),
        "the empty middle line shows as selected: {rects:?}"
    );
}

#[test]
fn a_width_change_wraps_again_to_the_same_lines() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_wrap_width(Some(60.0));
    editor.set_text("word word word\nword");
    let wrapped = editor.layout_size();

    editor.set_wrap_width(None);
    let unwrapped = editor.layout_size();
    assert!(unwrapped.height < wrapped.height);

    editor.set_wrap_width(Some(60.0));
    assert_eq!(
        editor.layout_size(),
        wrapped,
        "wrapping again at the first width gives the first lines back"
    );
}

#[test]
fn the_undo_history_keeps_a_bounded_number_of_bytes() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    let large = "x".repeat(1 << 20);
    editor.set_text(&large);

    for _ in 0..40 {
        editor.paste("y");
    }
    let mut undone = 0;
    loop {
        let before = editor.text();
        editor.undo();
        if editor.has_text(&before) {
            break;
        }
        undone += 1;
    }
    assert!(
        undone < 40,
        "forty steps over a 1 MB text do not all fit the history"
    );
    assert!(
        undone >= 4,
        "a few steps of a large text are kept: {undone}"
    );
}

#[test]
fn an_app_minimum_height_replaces_the_one_line_floor() {
    let value = RwSignal::new(String::new());
    let bound = value.clone();
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        smudge_core::widget::panel().column().child(
            text_area(bound.clone())
                .width(px(VIEWPORT.0))
                .min_height(px(4.0)),
        )
    });
    harness.frame();
    harness.settle();

    let height = area_rect(&mut harness).height();
    assert!(
        height < TextStyle::default().size,
        "the app's minimum replaces the floor the constructor set: {height}"
    );
}
