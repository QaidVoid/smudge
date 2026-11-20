//! The states a field shows besides its text: invalid, and masked.

use smudge_core::geometry::{Color, Point};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextInputExt, TextSystem, text_input};

/// The color a pixel of `color` takes, as the harness reads it.
fn rgba(color: Color) -> Option<[u8; 4]> {
    Some([color.r, color.g, color.b, color.a])
}

/// A window of 200 by 60 with a bordered field at 10, 10, 160 wide, that
/// shows `invalid`, turns its border the accent color under the pointer,
/// and is disabled while `disabled` reads `true`.
fn field(invalid: RwSignal<bool>, disabled: RwSignal<bool>) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 60.0, 1.0, move || {
        let (invalid, disabled) = (invalid.clone(), disabled.clone());
        panel().padding(px(10.0)).child(
            text_input(RwSignal::new(String::from("12x")))
                .width(px(160.0))
                .border(2.0)
                .border_color(ColorToken::BORDER)
                .hover(|p| p.border_color(ColorToken::ACCENT))
                .disabled_style(|p| p.border_color(ColorToken::TEXT_MUTED))
                .invalid(move || invalid.get())
                .disabled(move || disabled.get()),
        )
    });
    harness.settle();
    harness
}

#[test]
fn an_invalid_field_turns_its_border_the_error_color_with_no_layout_pass() {
    let (invalid, disabled) = (RwSignal::new(false), RwSignal::new(false));
    let mut harness = field(invalid.clone(), disabled);
    let theme = Theme::light();
    assert_eq!(harness.pixel(11, 20), rgba(theme.color(ColorToken::BORDER)));

    invalid.set(true);
    let report = harness.frame();
    assert!(!report.layout_ran, "only the border's color changed");
    assert_eq!(harness.pixel(11, 20), rgba(theme.color(ColorToken::ERROR)));
    harness.assert_matches_full_repaint();

    invalid.set(false);
    harness.frame();
    assert_eq!(harness.pixel(11, 20), rgba(theme.color(ColorToken::BORDER)));
}

#[test]
fn a_hovered_invalid_field_keeps_its_error_look_and_a_disabled_one_looks_disabled() {
    let (invalid, disabled) = (RwSignal::new(true), RwSignal::new(false));
    let mut harness = field(invalid.clone(), disabled.clone());
    let theme = Theme::light();

    harness.move_to(Point { x: 60.0, y: 20.0 });
    harness.settle();
    assert_eq!(
        harness.pixel(11, 20),
        rgba(theme.color(ColorToken::ERROR)),
        "the pointer over it does not hide the error"
    );
    invalid.set(false);
    harness.settle();
    assert_eq!(
        harness.pixel(11, 20),
        rgba(theme.color(ColorToken::ACCENT)),
        "where a valid field shows its hover look"
    );
    invalid.set(true);
    harness.settle();

    disabled.set(true);
    harness.settle();
    assert_eq!(
        harness.pixel(11, 20),
        rgba(theme.color(ColorToken::TEXT_MUTED)),
        "a disabled field looks disabled, whatever it holds"
    );
}

/// Draws a valid and an invalid field in `theme`.
fn valid_and_invalid(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(220.0, 90.0, 1.0, || {
        panel()
            .column()
            .padding(px(10.0))
            .gap(smudge_core::geometry::Size {
                width: 0.0,
                height: 10.0,
            })
            .background(ColorToken::SURFACE)
            .child(
                text_input(RwSignal::new(String::from("24")))
                    .width(px(200.0))
                    .padding(px(4.0))
                    .border(1.0)
                    .border_color(ColorToken::BORDER),
            )
            .child(
                text_input(RwSignal::new(String::from("24px")))
                    .width(px(200.0))
                    .padding(px(4.0))
                    .border(1.0)
                    .border_color(ColorToken::BORDER)
                    .invalid(true),
            )
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness
}

#[test]
fn a_valid_and_an_invalid_field_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", valid_and_invalid(Theme::light()).png());
}

#[test]
fn a_valid_and_an_invalid_field_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", valid_and_invalid(Theme::dark()).png());
}

/// The modifier of the copy, cut and paste shortcuts on this platform.
fn command() -> smudge_core::input::Modifiers {
    smudge_core::input::Modifiers {
        control: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..smudge_core::input::Modifiers::default()
    }
}

/// A window with a password field bound to a fresh signal, masked while
/// `masked` reads `true`, focused, with a memory clipboard installed that
/// holds "clip".
fn password(
    masked: RwSignal<bool>,
) -> (
    Harness,
    RwSignal<String>,
    smudge_core::clipboard::MemoryClipboard,
) {
    TextSystem::bundled_only().install();
    let value = RwSignal::new(String::new());
    let bound = value.clone();
    let mut harness = Harness::new(240.0, 60.0, 1.0, move || {
        let masked = masked.clone();
        text_input(bound.clone())
            .password(move || masked.get())
            .accessible_label("Password")
            .width(px(240.0))
            .autofocus()
    });
    let clipboard = smudge_core::clipboard::MemoryClipboard::default();
    {
        use smudge_core::clipboard::Clipboard;
        clipboard.clone().set(String::from("clip"));
    }
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.settle();
    (harness, value, clipboard)
}

/// The field as a screen reader gets it.
fn field_node(harness: &mut Harness) -> smudge_core::semantics::SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some("Password"))
        .expect("the field shows")
}

/// Returns whether `text` is one bullet per character of `count`.
fn bullets(text: &str, count: usize) -> bool {
    text.chars().count() == count && text.chars().all(|c| c == '\u{2022}')
}

#[test]
fn typing_and_editing_a_masked_field_edits_its_text_and_shows_bullets_alone() {
    use smudge_core::input::Key;
    let (mut harness, value, _) = password(RwSignal::new(true));
    harness.type_text("hunter2");
    harness.press_key(Key::Backspace);
    harness.press_key(Key::ArrowLeft);
    harness.press_key(Key::ArrowLeft);
    harness.type_text("X");
    harness.press_key(Key::End);
    harness.press_key_with(
        Key::ArrowLeft,
        smudge_core::input::Modifiers {
            shift: true,
            ..smudge_core::input::Modifiers::default()
        },
    );
    harness.type_text("y");
    harness.settle();

    assert_eq!(value.get(), "huntXey", "the signal holds the text");
    let node = field_node(&mut harness);
    assert_eq!(node.role, smudge_core::semantics::Role::PasswordInput);
    assert!(
        bullets(node.value.as_deref().unwrap_or_default(), 7),
        "{:?}",
        node.value
    );
    let runs: String = node.text_runs.iter().map(|run| run.text.as_str()).collect();
    assert!(
        bullets(&runs, 7),
        "the runs a reader walks hold bullets: {runs:?}"
    );
}

#[test]
fn copy_and_cut_of_a_masked_field_do_nothing_and_paste_works() {
    use smudge_core::clipboard::Clipboard;
    use smudge_core::input::Key;
    let (mut harness, value, mut clipboard) = password(RwSignal::new(true));
    harness.type_text("secret");
    harness.press_key_with(Key::Character(String::from("a")), command());
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.press_key_with(Key::Character(String::from("x")), command());
    harness.settle();
    assert_eq!(
        clipboard.get().as_deref(),
        Some("clip"),
        "the clipboard holds what it held"
    );
    assert_eq!(value.get(), "secret", "and a cut deleted nothing");

    harness.press_key(Key::End);
    harness.press_key_with(Key::Character(String::from("v")), command());
    harness.settle();
    assert_eq!(value.get(), "secretclip");
}

#[test]
fn word_deletion_takes_all_on_its_side_of_the_caret_and_a_double_click_all() {
    use smudge_core::input::Key;
    let word = smudge_core::input::Modifiers {
        control: !cfg!(target_os = "macos"),
        alt: cfg!(target_os = "macos"),
        ..smudge_core::input::Modifiers::default()
    };
    let (mut harness, value, _) = password(RwSignal::new(true));
    harness.type_text("two words");
    for _ in 0..3 {
        harness.press_key(Key::ArrowLeft);
    }
    harness.press_key_with(Key::Backspace, word);
    harness.settle();
    assert_eq!(value.get(), "rds", "everything before the caret went");
    harness.press_key_with(Key::Delete, word);
    harness.settle();
    assert_eq!(value.get(), "", "and then everything after it");

    harness.type_text("two words");
    harness.click(Point { x: 20.0, y: 20.0 });
    harness.click(Point { x: 20.0, y: 20.0 });
    harness.type_text("z");
    harness.settle();
    assert_eq!(value.get(), "z", "a double click selected it all");
}

#[test]
fn showing_the_text_keeps_the_caret_and_masking_it_again_hides_it() {
    use smudge_core::input::Key;
    let masked = RwSignal::new(true);
    let (mut harness, value, _) = password(masked.clone());
    harness.type_text("ae\u{301}io");
    harness.press_key(Key::ArrowLeft);
    harness.press_key(Key::ArrowLeft);

    masked.set(false);
    harness.settle();
    let shown = field_node(&mut harness);
    assert_eq!(shown.role, smudge_core::semantics::Role::TextInput);
    assert_eq!(shown.value.as_deref(), Some("ae\u{301}io"));
    harness.type_text("-");
    assert_eq!(
        value.get(),
        "ae\u{301}-io",
        "the caret stayed after the accented grapheme"
    );

    masked.set(true);
    harness.settle();
    assert!(bullets(
        field_node(&mut harness)
            .value
            .as_deref()
            .unwrap_or_default(),
        5
    ));
    harness.type_text("+");
    assert_eq!(value.get(), "ae\u{301}-+io");
}

#[test]
fn a_composing_preedit_shows_as_bullets_and_reaches_the_text_when_it_commits() {
    let (mut harness, value, _) = password(RwSignal::new(true));
    harness.type_text("a");
    harness.event(smudge_core::input::InputEvent::ImePreedit {
        text: String::from("ka"),
        cursor: Some((2, 2)),
    });
    harness.settle();
    assert_eq!(value.get(), "a", "the preedit is no text yet");
    let runs: String = field_node(&mut harness)
        .text_runs
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert!(
        bullets(&runs, 3),
        "a reader meets the preedit as bullets: {runs:?}"
    );
    harness.event(smudge_core::input::InputEvent::ImeCommit {
        text: String::from("\u{304b}"),
    });
    harness.settle();
    assert_eq!(value.get(), "a\u{304b}");
    assert!(bullets(
        field_node(&mut harness)
            .value
            .as_deref()
            .unwrap_or_default(),
        2
    ));
}

#[test]
fn a_focused_password_field_asks_the_input_method_for_a_password() {
    let masked = RwSignal::new(true);
    let (mut harness, _, _) = password(masked.clone());
    assert_eq!(
        harness.app().ime_purpose(),
        smudge_core::input::ImePurpose::Password
    );
    masked.set(false);
    harness.settle();
    assert_eq!(
        harness.app().ime_purpose(),
        smudge_core::input::ImePurpose::Normal
    );
}
