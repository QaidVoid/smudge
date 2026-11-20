//! Modal dialogs: over the whole window, holding the pointer and the keys,
//! and giving the focus back.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::show;
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{Handled, Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::px;
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label, text_input};
use smudge_widgets::{button, column, modal, scroll};

/// The color the dialog's box draws in.
const BOX: Color = Color::rgb(240, 240, 250);

/// Returns the projected node named `name`, if a reader reaches it.
fn named(harness: &mut Harness, name: &str) -> Option<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some(name))
}

/// Returns the name of the node that holds the focus.
fn focused(harness: &mut Harness) -> Option<String> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
        .and_then(|node| node.label)
}

/// A dialog box of two buttons, Keep and Discard, 120 by 60, that closes
/// on Discard.
fn dialog_box(open: RwSignal<bool>) -> smudge_core::element::Element<smudge_core::widget::Panel> {
    column()
        .width(px(120.0))
        .height(px(60.0))
        .background(BOX)
        .accessible_label("Unsaved changes")
        .child(button("Keep"))
        .child(button("Discard").on_press(move || open.set(false)))
}

/// What the scene's window counts.
struct Counts {
    /// Presses of the button under the dialog.
    under: Rc<Cell<u32>>,
    /// Keys the root's handler heard.
    keys: Rc<Cell<u32>>,
}

/// A window of 300 by 200 with a button that opens the dialog, inside a
/// scroll view 100 tall that clips it, and a button under where the dialog
/// shows.
fn scene(open: RwSignal<bool>) -> (Harness, Counts) {
    TextSystem::bundled_only().install();
    let counts = Counts {
        under: Rc::new(Cell::new(0)),
        keys: Rc::new(Cell::new(0)),
    };
    let (under, keys) = (Rc::clone(&counts.under), Rc::clone(&counts.keys));
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let (opener, dialog, under, keys) = (
            open.clone(),
            open.clone(),
            Rc::clone(&under),
            Rc::clone(&keys),
        );
        column()
            .on_key(move |event| {
                if event.pressed {
                    keys.set(keys.get() + 1);
                }
                Handled::No
            })
            .child(
                scroll(
                    column()
                        .child(button("Open").on_press(move || opener.set(true)))
                        .child(modal(dialog.clone(), move || dialog_box(dialog.clone()))),
                )
                .height(px(100.0)),
            )
            .child(
                button("Under")
                    .width(px(300.0))
                    .on_press(move || under.set(under.get() + 1)),
            )
    });
    harness.settle();
    (harness, counts)
}

/// Opens the dialog through the Open button, as a user does.
fn open_it(harness: &mut Harness) {
    let opener = named(harness, "Open").expect("the opener shows").bounds;
    harness.click(Point {
        x: opener.x0 + 4.0,
        y: opener.y0 + 4.0,
    });
    harness.settle();
}

#[test]
fn a_dialog_opened_inside_a_clipping_view_covers_the_whole_window() {
    let open = RwSignal::new(false);
    let (mut harness, _) = scene(open.clone());
    let before = harness.pixel(295, 195);
    open_it(&mut harness);

    let dialog = named(&mut harness, "Unsaved changes").expect("the dialog shows");
    assert_eq!(dialog.role, Role::Dialog);
    assert_eq!(
        (dialog.bounds.x0, dialog.bounds.y0),
        (90.0, 70.0),
        "centered in the window, not in the view"
    );
    assert_ne!(
        harness.pixel(295, 195),
        before,
        "the backdrop dims the corner"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn the_backdrop_takes_presses_and_the_dialog_holds_the_keys() {
    let open = RwSignal::new(false);
    let (mut harness, counts) = scene(open.clone());
    open_it(&mut harness);
    let keys_before = counts.keys.get();

    harness.click(Point { x: 10.0, y: 190.0 });
    harness.settle();
    assert_eq!(
        counts.under.get(),
        0,
        "the button under the backdrop heard nothing"
    );
    assert!(open.get(), "and the press did not close the dialog");

    harness.press_key(Key::Character(String::from("x")));
    assert_eq!(
        counts.keys.get(),
        keys_before,
        "a key the dialog leaves alone never reaches the view behind it"
    );
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Keep"),
        "the dialog has the focus"
    );
}

#[test]
fn tab_stays_in_the_dialog_escape_closes_it_and_the_focus_goes_back_to_the_opener() {
    let open = RwSignal::new(false);
    let (mut harness, _) = scene(open.clone());
    open_it(&mut harness);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Keep"),
        "the first button takes the focus"
    );

    harness.press_key(Key::Tab);
    assert_eq!(focused(&mut harness).as_deref(), Some("Discard"));
    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Keep"),
        "Tab wraps inside the dialog"
    );
    harness.press_key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Discard"));

    harness.press_key(Key::Escape);
    harness.settle();
    assert!(!open.get(), "Escape closes it");
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Open"),
        "the opener has the focus back"
    );
}

#[test]
fn a_screen_reader_reaches_the_dialog_and_nothing_behind_it() {
    let open = RwSignal::new(false);
    let (mut harness, _) = scene(open.clone());
    open_it(&mut harness);

    assert!(named(&mut harness, "Unsaved changes").is_some());
    assert!(named(&mut harness, "Keep").is_some());
    assert!(
        named(&mut harness, "Under").is_none(),
        "the tree behind is out of reach"
    );

    open.set(false);
    harness.settle();
    assert!(
        named(&mut harness, "Under").is_some(),
        "and back once it closes"
    );
}

#[test]
fn two_dialogs_stack_and_escape_closes_the_top_one() {
    TextSystem::bundled_only().install();
    let (first, second) = (RwSignal::new(true), RwSignal::new(false));
    let (outer, inner) = (first.clone(), second.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let (outer, inner) = (outer.clone(), inner.clone());
        panel().child(modal(outer, move || {
            let inner = inner.clone();
            column()
                .accessible_label("First")
                .child(button("More").on_press({
                    let inner = inner.clone();
                    move || inner.set(true)
                }))
                .child(modal(inner.clone(), || {
                    column().accessible_label("Second").child(button("Done"))
                }))
        }))
    });
    harness.settle();
    second.set(true);
    harness.settle();
    assert!(
        named(&mut harness, "First").is_none(),
        "the second hides the first from a reader"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Done"));

    harness.press_key(Key::Escape);
    harness.settle();
    assert!(
        !second.get() && first.get(),
        "Escape closed the top one only"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("More"));
}

#[test]
fn a_dialog_whose_opener_went_away_leaves_the_focus_nowhere() {
    TextSystem::bundled_only().install();
    let (open, shown) = (RwSignal::new(false), RwSignal::new(true));
    let (dialog, opener_shown) = (open.clone(), shown.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let (dialog, opener_shown, opener) = (dialog.clone(), opener_shown.clone(), dialog.clone());
        column()
            .child(show(
                move || opener_shown.get(),
                move || {
                    let opener = opener.clone();
                    button("Open").on_press(move || opener.set(true))
                },
            ))
            .child(modal(dialog.clone(), move || dialog_box(dialog.clone())))
    });
    harness.settle();
    open_it(&mut harness);
    shown.set(false);
    harness.settle();
    harness.press_key(Key::Escape);
    harness.settle();

    assert!(!open.get());
    assert_eq!(focused(&mut harness), None, "nothing holds the focus");
}

#[test]
fn an_autofocused_field_in_a_dialog_keeps_the_focus_over_the_first_button() {
    TextSystem::bundled_only().install();
    let open = RwSignal::new(true);
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(modal(open.clone(), || {
            column()
                .accessible_label("Rename")
                .child(button("Cancel"))
                .child(
                    text_input(RwSignal::new(String::new()))
                        .accessible_label("Name")
                        .autofocus(),
                )
        }))
    });
    harness.settle();
    assert_eq!(focused(&mut harness).as_deref(), Some("Name"));
}

/// Draws a small form under an open dialog in `theme`.
fn dialog_over_form(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(240.0, 160.0, 1.0, || {
        let open = RwSignal::new(true);
        column()
            .padding(px(12.0))
            .background(ColorToken::SURFACE)
            .child(label("Name"))
            .child(text_input(RwSignal::new(String::from("Harbor"))))
            .child(button("Save"))
            .child(modal(open, || {
                column()
                    .padding(px(12.0))
                    .background(ColorToken::SURFACE)
                    .border(1.0)
                    .border_color(ColorToken::BORDER)
                    .accessible_label("Saved")
                    .child(label("Saved"))
                    .child(button("Close"))
            }))
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness
}

#[test]
fn a_dialog_over_a_form_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", dialog_over_form(Theme::light()).png());
}

#[test]
fn a_dialog_over_a_form_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", dialog_over_form(Theme::dark()).png());
}

#[test]
fn a_dialog_reopened_while_it_fades_out_comes_back_on_top_with_the_focus() {
    TextSystem::bundled_only().install();
    let (first, second) = (RwSignal::new(false), RwSignal::new(false));
    let (a, b) = (first.clone(), second.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let fading = |open: RwSignal<bool>, name: &'static str, button_name: &'static str| {
            modal(open, move || {
                column()
                    .width(px(120.0))
                    .accessible_label(name)
                    .child(button(button_name))
            })
            .fade(smudge_core::animation::Spring::SMOOTH)
        };
        column()
            .child(fading(a.clone(), "First", "Keep"))
            .child(fading(b.clone(), "Second", "Yes"))
    });
    harness.settle();
    first.set(true);
    harness.settle();
    second.set(true);
    harness.settle();
    assert_eq!(focused(&mut harness).as_deref(), Some("Yes"));

    first.set(false);
    harness.frame();
    first.set(true);
    harness.frame();
    harness.frame();

    assert!(
        named(&mut harness, "First").is_some() && named(&mut harness, "Second").is_none(),
        "the first dialog opened last, so it is the one on top"
    );
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Keep"),
        "and it takes the focus as a fresh open does"
    );
}

#[test]
fn a_popup_of_the_view_opened_under_a_dialog_never_takes_its_keys() {
    TextSystem::bundled_only().install();
    let (dialog, popup) = (RwSignal::new(true), RwSignal::new(false));
    let (shown, opened) = (dialog.clone(), popup.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        column()
            .child(panel().width(px(10.0)).height(px(10.0)).overlay(
                opened.clone(),
                smudge_core::element::Placement::Below,
                || {
                    column()
                        .trap_focus()
                        .child(button("Hidden").autofocus())
                        .child(button("Also hidden"))
                },
            ))
            .child(modal(shown.clone(), || {
                column()
                    .accessible_label("Front")
                    .child(button("Keep"))
                    .child(button("Discard"))
            }))
    });
    harness.settle();
    assert_eq!(focused(&mut harness).as_deref(), Some("Keep"));

    popup.set(true);
    harness.settle();
    harness.press_key(Key::Tab);
    harness.settle();
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Discard"),
        "Tab moves on inside the dialog, which still holds the keyboard"
    );
}

#[test]
fn a_dialog_whose_content_turns_disabled_keeps_the_keys_and_the_focus_until_it_closes() {
    TextSystem::bundled_only().install();
    let (open, busy) = (RwSignal::new(false), RwSignal::new(false));
    let deleted = Rc::new(Cell::new(0));
    let (dialog, disabled, presses) = (open.clone(), busy.clone(), Rc::clone(&deleted));
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let (opener, disabled, presses) = (dialog.clone(), disabled.clone(), Rc::clone(&presses));
        column()
            .child(button("Delete").on_press(move || {
                presses.set(presses.get() + 1);
                opener.set(true);
            }))
            .child(modal(dialog.clone(), move || {
                let (disabled, status) = (disabled.clone(), disabled.clone());
                column()
                    .accessible_label("Saving")
                    .disabled(move || disabled.get())
                    .child(button("Stop"))
                    .child(label(move || {
                        String::from(if status.get() {
                            "Saving the map"
                        } else {
                            "Ready"
                        })
                    }))
            }))
    });
    harness.settle();
    let delete = named(&mut harness, "Delete").expect("the button shows");
    harness.click(Point {
        x: delete.bounds.x0 + 4.0,
        y: delete.bounds.y0 + 4.0,
    });
    harness.settle();
    assert_eq!(focused(&mut harness).as_deref(), Some("Stop"));

    busy.set(true);
    harness.settle();
    harness.press_key(Key::Enter);
    harness.press_key(Key::Tab);
    harness.press_key(Key::Enter);
    harness.settle();
    assert_eq!(
        deleted.get(),
        1,
        "the button behind the dialog heard no key"
    );
    assert_ne!(
        harness.app().focused(),
        Some(delete.id),
        "nor took the focus back"
    );

    harness.press_key(Key::Escape);
    harness.settle();
    assert!(!open.get());
    assert_eq!(
        harness.app().focused(),
        Some(delete.id),
        "closing the dialog gives the focus back to its opener"
    );
}

#[test]
fn a_dialog_reopened_while_it_fades_out_focuses_its_autofocused_field_again() {
    TextSystem::bundled_only().install();
    let open = RwSignal::new(true);
    let shown = open.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        panel().child(
            modal(shown.clone(), || {
                column()
                    .accessible_label("Rename")
                    .child(button("Cancel"))
                    .child(
                        text_input(RwSignal::new(String::new()))
                            .accessible_label("Name")
                            .autofocus(),
                    )
            })
            .fade(smudge_core::animation::Spring::SMOOTH),
        )
    });
    harness.settle();
    assert_eq!(focused(&mut harness).as_deref(), Some("Name"));

    open.set(false);
    harness.frame();
    open.set(true);
    harness.frame();
    harness.frame();
    assert_eq!(focused(&mut harness).as_deref(), Some("Name"));
}

#[test]
fn a_dialog_disabled_when_it_opens_gives_the_focus_back_to_its_opener() {
    TextSystem::bundled_only().install();
    let (open, busy) = (RwSignal::new(false), RwSignal::new(true));
    let (dialog, disabled) = (open.clone(), busy.clone());
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let (opener, disabled) = (dialog.clone(), disabled.clone());
        column()
            .child(button("Delete").on_press(move || opener.set(true)))
            .child(modal(dialog.clone(), move || {
                let disabled = disabled.clone();
                column()
                    .accessible_label("Saving")
                    .disabled(move || disabled.get())
                    .child(button("Stop"))
            }))
    });
    harness.settle();
    let delete = named(&mut harness, "Delete").expect("the button shows");
    harness.click(Point {
        x: delete.bounds.x0 + 4.0,
        y: delete.bounds.y0 + 4.0,
    });
    harness.settle();
    busy.set(false);
    harness.settle();
    harness.press_key(Key::Escape);
    harness.settle();

    assert!(!open.get());
    assert_eq!(focused(&mut harness).as_deref(), Some("Delete"));
}
