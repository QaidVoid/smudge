//! Text input editing, through the headless harness and the editor directly.
//!
//! The harness tests cover what a user does: typing, deleting, selecting and
//! the pixels a blink damages, and a property test drives random keys and
//! outside writes against an editor that applies the same edits directly.
//! The editor-level tests cover the caret in right-to-left text, which is
//! only meaningful in parley's own geometry.

use std::time::Duration;

use proptest::prelude::*;

use smudge_core::geometry::{Color, PixelRect, Point, Rect};
use smudge_core::input::{InputEvent, Key, Modifiers};
use smudge_core::paint::{DisplayItem, PaintOp};
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, Theme};
use smudge_test::Harness;
use smudge_text::{Editor, TextInputExt, TextStyle, text_input};

/// How long the caret stays visible or hidden in one blink, the same period
/// the input uses.
const BLINK: Duration = Duration::from_millis(530);

/// The viewport of a plain input harness, in logical pixels.
const VIEWPORT: (f32, f32) = (200.0, 60.0);

/// Mounts an input bound to a fresh signal, and returns the harness and the
/// signal.
fn mounted(initial: &str) -> (Harness, RwSignal<String>) {
    let value = RwSignal::new(String::from(initial));
    let bound = value.clone();
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        text_input(bound.clone())
            .width(px(VIEWPORT.0))
            .height(px(VIEWPORT.1))
    });
    harness.frame();
    (harness, value)
}

/// Clicks in the middle of the input, which focuses it, and settles.
fn focus(harness: &mut Harness) {
    harness.click(Point {
        x: 60.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.settle();
}

#[test]
fn typing_then_backspace_leaves_the_rest_of_the_text() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("héllo");
    assert_eq!(value.get(), "héllo", "the signal carries every typed rune");

    harness.press_key(Key::Backspace);
    assert_eq!(value.get(), "héll", "backspace removes one grapheme");
}

#[test]
fn control_backspace_deletes_the_word_before_the_caret() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("hello world");
    let modifiers = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::Backspace, modifiers);
    assert_eq!(value.get(), "hello ", "the word goes, the space stays");
}

#[test]
fn shift_home_selects_to_the_start_and_typing_replaces_the_selection() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("hello world");
    harness.press_key_with(
        Key::Home,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    harness.type_text("X");
    assert_eq!(value.get(), "X", "the selection is gone, replaced by X");
}

#[test]
fn command_a_selects_all_and_typing_replaces() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("keep one");
    harness.press_key_with(Key::Character(String::from("a")), command());
    harness.type_text("X");
    assert_eq!(value.get(), "X");
}

#[test]
fn undo_removes_typed_text_a_word_at_a_time_and_redo_brings_it_back() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);
    let z = || Key::Character(String::from("z"));
    let redo = Modifiers {
        shift: true,
        ..command()
    };

    harness.type_text("hello world");
    harness.press_key_with(z(), command());
    assert_eq!(value.get(), "hello", "the last word goes first");
    harness.press_key_with(z(), command());
    assert_eq!(value.get(), "");
    harness.press_key_with(z(), command());
    assert_eq!(value.get(), "", "undo past the first step does nothing");

    harness.press_key_with(z(), redo);
    assert_eq!(value.get(), "hello");
    harness.press_key_with(z(), redo);
    assert_eq!(value.get(), "hello world");

    harness.press_key_with(z(), command());
    harness.type_text("!");
    harness.press_key_with(z(), redo);
    assert_eq!(value.get(), "hello!", "an edit after an undo ends redo");
}

#[test]
fn a_word_delete_is_an_undo_step_of_its_own() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);
    let z = || Key::Character(String::from("z"));

    harness.type_text("hello world");
    harness.press_key_with(
        Key::Backspace,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    harness.press_key(Key::Backspace);
    assert_eq!(value.get(), "hello");
    harness.press_key_with(z(), command());
    assert_eq!(value.get(), "hello ", "the grapheme delete undoes alone");
    harness.press_key_with(z(), command());
    assert_eq!(value.get(), "hello world", "then the word delete");
}

#[test]
fn committed_text_is_an_undo_step_of_its_own() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.event(InputEvent::ImeCommit {
        text: String::from("世界"),
    });
    harness.type_text("x");
    harness.press_key_with(Key::Character(String::from("z")), command());
    assert_eq!(value.get(), "世界", "the typed character undoes alone");
}

#[test]
fn undo_puts_the_selection_back() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("hello");
    harness.press_key_with(Key::Character(String::from("a")), command());
    harness.type_text("X");
    harness.press_key_with(Key::Character(String::from("z")), command());
    assert_eq!(value.get(), "hello");
    harness.type_text("Y");
    assert_eq!(value.get(), "Y", "the whole text was selected again");
}

#[test]
fn an_outside_write_ends_the_undo_history() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("draft");
    value.set(String::from("saved"));
    harness.settle();
    harness.press_key_with(Key::Character(String::from("z")), command());
    assert_eq!(value.get(), "saved", "undo never brings back replaced text");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn control_y_redoes() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("word");
    harness.press_key_with(Key::Character(String::from("z")), command());
    harness.press_key_with(Key::Character(String::from("y")), command());
    assert_eq!(value.get(), "word");
}

/// Returns the modifiers of the platform's command key.
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
fn enter_and_tab_do_not_edit_and_tab_wraps_to_the_input() {
    let (mut harness, value) = mounted("text");
    focus(&mut harness);

    harness.press_key(Key::Enter);
    assert_eq!(value.get(), "text", "Enter inserts nothing");

    harness.press_key(Key::Tab);
    assert_eq!(value.get(), "text", "Tab inserts nothing");
    assert!(
        harness.app().focused().is_some(),
        "Tab wraps to the input itself when it is the only focusable node"
    );
}

#[test]
fn an_ime_commit_inserts_without_a_key_event() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.event(InputEvent::ImeCommit {
        text: String::from("世界"),
    });
    assert_eq!(value.get(), "世界", "the committed text is in the signal");
}

#[test]
fn a_press_at_the_edges_places_the_caret_at_the_matching_end() {
    let (mut harness, value) = mounted("abcdef");
    harness.settle();

    harness.click(Point {
        x: 2.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.settle();
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "Xabcdef",
        "a press near the left edge puts the caret at the start"
    );

    value.set(String::from("abcdef"));
    harness.settle();

    harness.click(Point {
        x: VIEWPORT.0 - 2.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.settle();
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "abcdefX",
        "a press near the right edge puts the caret at the end"
    );
}

#[test]
fn a_drag_selects_and_typing_replaces_the_selection() {
    let (mut harness, value) = mounted("abcdef");
    focus(&mut harness);

    harness.button_down(Point {
        x: 2.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.move_to(Point {
        x: VIEWPORT.0 - 2.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.button_up(Point {
        x: VIEWPORT.0 - 2.0,
        y: VIEWPORT.1 / 2.0,
    });
    harness.type_text("X");
    assert_eq!(value.get(), "X", "the drag selected everything");
}

#[test]
fn a_double_click_selects_a_word() {
    let (mut harness, value) = mounted("foo bar");
    focus(&mut harness);

    // Both clicks land inside "bar", which ends the text and so owns the
    // right side of the field.
    let point = Point {
        x: VIEWPORT.0 - 10.0,
        y: VIEWPORT.1 / 2.0,
    };
    harness.click(point);
    harness.click(point);
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "foo X",
        "the double click selected the word under the pointer"
    );
}

#[test]
fn clicks_too_far_apart_in_time_or_space_are_not_a_double_click() {
    let (mut harness, value) = mounted("foo bar");
    focus(&mut harness);
    let end = Point {
        x: VIEWPORT.0 - 10.0,
        y: VIEWPORT.1 / 2.0,
    };

    harness.click(end);
    harness.advance(Duration::from_millis(401));
    harness.click(end);
    harness.type_text("X");
    assert_eq!(value.get(), "foo barX", "401 ms apart is two single clicks");

    harness.advance(Duration::from_secs(1));
    harness.click(end);
    harness.click(Point {
        x: end.x - 5.0,
        ..end
    });
    harness.type_text("Y");
    assert_eq!(value.get(), "foo barXY", "5 px apart is two single clicks");
}

#[test]
fn a_click_right_after_a_drag_places_the_caret() {
    let (mut harness, value) = mounted("foo bar");
    focus(&mut harness);
    let start = Point {
        x: 2.0,
        y: VIEWPORT.1 / 2.0,
    };

    harness.advance(Duration::from_secs(1));
    harness.button_down(start);
    harness.move_to(Point {
        x: VIEWPORT.0 - 2.0,
        ..start
    });
    harness.button_up(Point {
        x: VIEWPORT.0 - 2.0,
        ..start
    });
    harness.click(start);
    harness.type_text("X");
    assert_eq!(
        value.get(),
        "Xfoo bar",
        "the click after a drag is a single click, not a double click on \
         the drag's start"
    );
}

#[test]
fn a_blink_frame_damages_only_the_caret() {
    let (mut harness, _value) = mounted("");
    focus(&mut harness);
    harness.type_text("hi");
    harness.settle();

    // Everything has settled: a frame with a steady caret repaints nothing.
    let steady = harness.frame();
    assert!(
        steady.damage.is_empty() && steady.nodes_painted == 0,
        "the caret does not move on its own: {steady:?}"
    );

    // One blink later, only the caret node repaints.
    let blink = harness.advance(BLINK);
    assert_eq!(blink.nodes_painted, 1, "only the caret node repaints");
    assert_eq!(blink.damage.len(), 1, "the blink damages one rectangle");
    let caret = blink.damage[0];
    assert!(
        caret.width <= 4,
        "the caret is a pixel wide, not the whole input: {caret:?}"
    );
    assert!(caret.height <= 34, "the caret is one line tall: {caret:?}");
    harness.assert_matches_full_repaint();

    // And the next blink damages the same small area bringing it back.
    let back = harness.advance(BLINK);
    assert_eq!(back.nodes_painted, 1);
    assert_eq!(back.damage.len(), 1);
    assert!(back.damage[0].width <= 4 && back.damage[0].height <= 34);
}

#[test]
fn an_editing_frame_repaints_the_text_and_the_blink_stays_small() {
    let (mut harness, _value) = mounted("");
    focus(&mut harness);

    harness.type_text("a");
    let report = harness.frame();
    assert!(
        !report.damage.is_empty(),
        "typing repaints the text the frame the key arrives"
    );
    harness.assert_matches_full_repaint();

    harness.settle();
    let steady = harness.frame();
    assert!(steady.damage.is_empty() && steady.nodes_painted == 0);
    let blink = harness.advance(BLINK);
    assert_eq!(blink.nodes_painted, 1, "the blink repaints the caret alone");
    assert!(blink.damage.iter().all(is_small_caret));
}

#[test]
fn scrolling_keeps_the_caret_inside_the_input() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    let long = "0123456789".repeat(6);
    harness.type_text(&long);
    harness.settle();
    assert_eq!(value.get(), long);

    // The caret sits past the right edge of the text, which is wider than
    // the input, so the input has to scroll for the blink to stay visible.
    let blink = harness.advance(BLINK);
    assert_eq!(
        blink.damage.len(),
        1,
        "the blink still damages one rectangle"
    );
    let caret = blink.damage[0];
    assert!(is_small_caret(&caret));
    assert!(
        caret.x + caret.width <= VIEWPORT.0 as u32,
        "the caret stays inside the input while the text scrolls: {caret:?}"
    );
}

#[test]
fn trailing_spaces_keep_the_caret_inside_the_input() {
    let (mut harness, _value) = mounted("");
    focus(&mut harness);
    harness.type_text(&"0123456789".repeat(4));
    harness.type_text(&" ".repeat(12));
    harness.settle();

    let (fills, _) = painted(&mut harness);
    let caret = fills.last().expect("the focused input paints its caret");
    assert!(
        caret.x1 <= VIEWPORT.0 && caret.x0 > VIEWPORT.0 - 4.0,
        "spaces past the text's ink still scroll the caret to the right \
         edge: {caret:?}"
    );
}

/// A rectangle small enough to be a caret and nothing else.
fn is_small_caret(rect: &PixelRect) -> bool {
    rect.width <= 4 && rect.height <= 34
}

#[test]
fn the_placeholder_shows_while_the_text_is_empty() {
    let value = RwSignal::new(String::new());
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        text_input(value.clone())
            .width(px(VIEWPORT.0))
            .height(px(VIEWPORT.1))
            .placeholder("Type here")
    });
    harness.frame();
    harness.settle();
    harness.assert_matches_full_repaint();

    harness.type_text("hi");
    harness.settle();
    assert!(
        !harness.png().is_empty(),
        "the input renders with and without its placeholder"
    );
}

#[test]
fn editing_right_to_left_text_edits_the_signal() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    harness.type_text("سلام");
    assert_eq!(value.get(), "سلام");
    harness.press_key(Key::Backspace);
    let remaining = value.get();
    assert_eq!(
        remaining.chars().count(),
        3,
        "backspace removes one grapheme"
    );
    assert!(remaining.chars().eq("سلام".chars().take(3)));
}

#[test]
fn the_caret_moves_visually_in_right_to_left_text_as_parley_reports() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value);
    editor.set_style(&TextStyle::default());
    editor.set_text("سلام");
    let caret_width = 1.0;

    // The caret starts at the end of the text, which in a right-to-left run
    // is its visual left edge.
    let end = editor
        .cursor_geometry(caret_width)
        .expect("the editor has a caret");

    editor.move_right();
    let after_right = editor
        .cursor_geometry(caret_width)
        .expect("the caret is still there");
    assert!(
        after_right.x0 > end.x0,
        "moving right moves the caret visually right, even though it is \
         moving towards the logical start: {} -> {}",
        end.x0,
        after_right.x0
    );

    editor.move_left();
    editor.move_left();
    let after_left = editor
        .cursor_geometry(caret_width)
        .expect("the caret is still there");
    assert!(
        after_left.x0 < after_right.x0,
        "moving left moves the caret visually left: {} -> {}",
        after_right.x0,
        after_left.x0
    );
    assert!(
        (after_left.x0 - end.x0).abs() < 1.0,
        "the caret lands on the boundaries parley reported before: {} vs {}",
        after_left.x0,
        end.x0
    );
}

#[test]
fn an_outside_write_replaces_the_text_and_moves_the_caret_to_the_end() {
    let value = RwSignal::new(String::new());
    let editor = Editor::new(value.clone());
    editor.set_style(&TextStyle::default());

    value.set(String::from("héllo"));
    editor.set_text(&value.get());
    editor.insert("!");
    assert_eq!(
        editor.text(),
        "héllo!",
        "the caret sits at the end after an outside write"
    );

    editor.set_text("short");
    assert_eq!(editor.selection(), 5..5, "the caret is at the new end");
}

/// Returns the rectangles and glyph runs of the current display list.
///
/// The frame that builds the list changes nothing when the harness has
/// settled, so the list is the one the last real frame painted.
fn painted(harness: &mut Harness) -> (Vec<Rect>, Vec<Color>) {
    let output = harness.app().frame();
    let mut fills = Vec::new();
    let mut glyph_colors = Vec::new();
    for item in output.list.items() {
        match item {
            DisplayItem::Op(PaintOp::FillRect { rect, .. }) => fills.push(*rect),
            DisplayItem::Op(PaintOp::Glyphs(run)) => glyph_colors.push(run.color),
            _ => {}
        }
    }
    (fills, glyph_colors)
}

#[test]
fn an_edit_moves_the_caret_in_the_frame_that_shows_the_text() {
    let (mut harness, _value) = mounted("");
    focus(&mut harness);

    harness.type_text("ab");
    harness.frame();
    assert!(
        !harness.woken(),
        "placing the caret in the apply step asks for no second frame"
    );
    let next = harness.frame();
    assert!(
        next.nodes_painted == 0 && next.damage.is_empty(),
        "the frame that painted the text also placed the caret: {next:?}"
    );
}

#[test]
fn a_selection_in_scrolled_text_paints_over_its_glyphs() {
    let (mut harness, _value) = mounted("");
    focus(&mut harness);
    harness.type_text(&"0123456789".repeat(6));
    harness.settle();
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for _ in 0..3 {
        harness.press_key_with(Key::ArrowLeft, shift);
    }
    harness.settle();

    let (fills, _) = painted(&mut harness);
    let selection = fills
        .iter()
        .find(|rect| rect.width() > 4.0)
        .expect("three selected digits paint a selection");
    assert!(
        selection.x0 >= 0.0 && selection.x1 <= VIEWPORT.0,
        "the selection scrolls with the text it covers: {selection:?}"
    );
}

#[test]
fn an_outside_write_repaints_without_running_layout() {
    let (mut harness, value) = mounted("one");
    harness.settle();

    value.set(String::from("two"));
    let report = harness.frame();
    assert!(
        !report.layout_ran,
        "the input's size does not hang on its text"
    );
    assert!(report.nodes_painted > 0, "the new text is painted");
}

#[test]
fn the_placeholder_paints_in_the_muted_text_color() {
    let value = RwSignal::new(String::new());
    let mut harness = Harness::new(VIEWPORT.0, VIEWPORT.1, 1.0, move || {
        text_input(value.clone())
            .width(px(VIEWPORT.0))
            .height(px(VIEWPORT.1))
            .placeholder("Type here")
    });
    harness.settle();

    let (_, colors) = painted(&mut harness);
    let muted = Theme::light().color(ColorToken::TEXT_MUTED);
    assert!(!colors.is_empty(), "the placeholder paints");
    assert!(colors.iter().all(|color| *color == muted));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn alt_gr_types_its_character_and_control_does_not() {
    let (mut harness, value) = mounted("");
    focus(&mut harness);

    let alt_gr = Modifiers {
        control: true,
        alt: true,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::Character(String::from("@")), alt_gr);
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    harness.press_key_with(Key::Character(String::from("x")), control);
    harness.press_key_with(Key::Character(String::from("a")), alt_gr);
    assert_eq!(
        value.get(),
        "@a",
        "AltGr types even the letter Control selects everything with, and a \
         Control shortcut types nothing"
    );
}

/// One thing the property test does to an input.
#[derive(Clone, Debug)]
enum Op {
    /// Types a short string.
    Type(String),
    /// Presses a key, with Shift held when the flag is set.
    Key(Key, bool),
    /// Presses a key with Control held, and Shift when the flag is set.
    WordKey(Key, bool),
    /// Presses Alt and Backspace.
    AltBackspace,
    /// Selects everything with the command modifier.
    SelectAll,
    /// Writes the bound signal from outside.
    Write(String),
    /// Undoes with the command key.
    Undo,
    /// Redoes with the command key and Shift.
    Redo,
}

/// A short piece of text, mixed scripts and whitespace included.
fn any_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::char::any().prop_filter("printable only", |c| !c.is_control()),
        0..4,
    )
    .prop_map(String::from_iter)
}

fn any_key() -> impl Strategy<Value = Key> {
    prop_oneof![
        Just(Key::Backspace),
        Just(Key::Delete),
        Just(Key::ArrowLeft),
        Just(Key::ArrowRight),
        Just(Key::Home),
        Just(Key::End),
    ]
}

fn any_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any_text().prop_map(Op::Type),
        4 => (any_key(), any::<bool>()).prop_map(|(key, shift)| Op::Key(key, shift)),
        2 => (any_key(), any::<bool>()).prop_map(|(key, shift)| Op::WordKey(key, shift)),
        1 => Just(Op::AltBackspace),
        1 => Just(Op::SelectAll),
        1 => any_text().prop_map(Op::Write),
        1 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

/// Applies `op` to the reference editor, the way the input's handlers and
/// its binding do.
fn apply_to_reference(editor: &Editor, op: &Op) {
    match op {
        Op::Type(text) => text.chars().for_each(|c| editor.insert(&c.to_string())),
        Op::Key(key, shift) => match (key, shift) {
            (Key::Backspace, _) => editor.backspace(),
            (Key::Delete, _) => editor.delete(),
            (Key::ArrowLeft, false) => editor.move_left(),
            (Key::ArrowLeft, true) => editor.extend_left(),
            (Key::ArrowRight, false) => editor.move_right(),
            (Key::ArrowRight, true) => editor.extend_right(),
            (Key::Home, false) => editor.move_to_start(),
            (Key::Home, true) => editor.extend_to_start(),
            (Key::End, false) => editor.move_to_end(),
            (Key::End, true) => editor.extend_to_end(),
            _ => unreachable!("the strategy makes no other keys"),
        },
        Op::WordKey(key, shift) => match (key, shift) {
            (Key::Backspace, _) => editor.backspace_word(),
            (Key::Delete, _) => editor.delete_word(),
            (Key::ArrowLeft, false) => editor.move_word_left(),
            (Key::ArrowLeft, true) => editor.extend_word_left(),
            (Key::ArrowRight, false) => editor.move_word_right(),
            (Key::ArrowRight, true) => editor.extend_word_right(),
            (Key::Home, false) => editor.move_to_start(),
            (Key::Home, true) => editor.extend_to_start(),
            (Key::End, false) => editor.move_to_end(),
            (Key::End, true) => editor.extend_to_end(),
            _ => unreachable!("the strategy makes no other keys"),
        },
        Op::AltBackspace => editor.backspace_word(),
        Op::SelectAll => editor.select_all(),
        // The input applies an outside write only when it changes the text.
        Op::Write(text) if !editor.has_text(text) => editor.set_text(text),
        Op::Write(_) => {}
        Op::Undo => editor.undo(),
        Op::Redo => editor.redo(),
    }
}

/// Applies `op` to the input through the harness.
fn apply_to_input(harness: &mut Harness, value: &RwSignal<String>, op: &Op) {
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    match op {
        Op::Type(text) => harness.type_text(text),
        Op::Key(key, shift) => harness.press_key_with(
            key.clone(),
            Modifiers {
                shift: *shift,
                ..Modifiers::default()
            },
        ),
        Op::WordKey(key, shift) => harness.press_key_with(
            key.clone(),
            Modifiers {
                shift: *shift,
                ..control
            },
        ),
        Op::AltBackspace => harness.press_key_with(
            Key::Backspace,
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
        ),
        Op::SelectAll => harness.press_key_with(Key::Character(String::from("a")), command()),
        Op::Write(text) => value.set(text.clone()),
        Op::Undo => harness.press_key_with(Key::Character(String::from("z")), command()),
        Op::Redo => harness.press_key_with(
            Key::Character(String::from("z")),
            Modifiers {
                shift: true,
                ..command()
            },
        ),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_edits_keep_the_signal_in_step(ops in proptest::collection::vec(any_op(), 0..30)) {
        let (mut harness, value) = mounted("");
        focus(&mut harness);
        let reference = Editor::new(RwSignal::new(String::new()));
        reference.set_style(&TextStyle::default());

        for op in &ops {
            apply_to_input(&mut harness, &value, op);
            harness.settle();
            apply_to_reference(&reference, op);
            prop_assert_eq!(
                value.get(),
                reference.text(),
                "the signal holds the text the edits made, after {:?}",
                op
            );
            let raw = reference.raw_text();
            let selection = reference.selection();
            prop_assert!(raw.is_char_boundary(selection.start));
            prop_assert!(raw.is_char_boundary(selection.end));
        }
    }
}

/// An upper bound on the undo steps one operation can make: a typed string
/// has fewer than four characters, and each may start a step.
const STEPS_PER_OP: usize = 4;

/// The most operations the undo property applies, so that even when every
/// one makes [`STEPS_PER_OP`] steps they fit the editor's history of 100.
const MAX_UNDO_OPS: usize = 100 / STEPS_PER_OP;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn undoing_every_step_returns_to_the_last_outside_write_and_redo_returns(
        ops in proptest::collection::vec(any_op(), 0..=MAX_UNDO_OPS)
    ) {
        let reference = Editor::new(RwSignal::new(String::new()));
        reference.set_style(&TextStyle::default());
        let ops: Vec<Op> = ops
            .into_iter()
            .filter(|op| !matches!(op, Op::Undo | Op::Redo))
            .collect();
        let mut baseline = String::new();
        for op in &ops {
            if let Op::Write(text) = op
                && !reference.has_text(text)
            {
                baseline = text.clone();
            }
            apply_to_reference(&reference, op);
        }
        let end = reference.text();

        for _ in 0..=STEPS_PER_OP * ops.len() {
            reference.undo();
        }
        prop_assert_eq!(reference.text(), baseline, "undo walks back to the last outside write");
        for _ in 0..=STEPS_PER_OP * ops.len() {
            reference.redo();
        }
        prop_assert_eq!(reference.text(), end, "redo walks forward to where the edits ended");
    }
}
