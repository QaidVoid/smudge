//! Mirrors docs/guide/text-and-input.md: its Rust samples, held as functions.
//!
//! Every ```rust block of the page appears here exactly as the page
//! shows it, reindented one level inside the wrapper each block needs,
//! and names the page leaves undefined are stubbed next to their
//! block. The page is the source of truth: change it here and there
//! together. The allows hold what page text may carry without a
//! compiler's blessing, so page edits trip this check only for real
//! API drift.
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_imports)]

use smudge::prelude::*;


#[test]
fn labels() {
    use smudge::prelude::*;
    
    let heading = label("Account").text_size(22.0);
    let hint = label("Changes save automatically").text_color(ColorToken::TEXT_MUTED);
}

#[test]
fn system_ui() {
    use smudge::prelude::*;
    use smudge::text::{FontFamily, GenericFamily};
    
    fn root() -> Element<Panel> {
        provide_text_style(TextStyle {
            family: FontFamily::from(GenericFamily::SystemUi),
            ..TextStyle::default()
        });
        column().child(label("Drawn in the host's interface font"))
    }
}

#[test]
fn max_lines() {
    use smudge::prelude::*;
    
    let name = label("Long department name").max_lines(Some(2));
    let open = label("Open-ended").max_lines(None);
}

#[test]
fn default_style() {
    provide_text_style(TextStyle {
        size: 15.0,
        line_height: 1.4,
        ..current_text_style()
    });
}

#[test]
fn form() {
    let email = RwSignal::new(String::new());
    let preview = email.clone();
    
    let form = column()
        .gap(Size { width: 0.0, height: 8.0 })
        .child(
            text_input(email)
                .width(px(280.0))
                .height(px(32.0))
                .placeholder("you@example.com")
                .border(1.0)
                .border_color(ColorToken::BORDER),
        )
        .child(label(move || format!("Sending to {}", preview.get())));
}

#[test]
fn area() {
    let draft = RwSignal::new(String::from("Dear reader,\n"));
    
    let field = text_area(draft.clone())
        .width(px(360.0))
        .min_lines(4)
        .placeholder("Write your reply")
        .border(1.0)
        .border_color(ColorToken::BORDER);
}

#[test]
fn canvas() {
    use smudge::core::input::{Handled, Key};
    
    let canvas = panel()
        .size(Size { width: 320.0, height: 200.0 })
        .focusable()
        .on_pointer_down(|at| println!("pressed at {}, {}", at.x, at.y))
        .on_key(|event| {
            if event.pressed && event.key == Key::Escape {
                println!("cancel");
                return Handled::Yes;
            }
            Handled::No
        });
}

#[test]
fn dialog() {
    use smudge::core::input::{Handled, Key};
    
    let dialog = panel()
        .autofocus()
        .trap_focus()
        .on_key(|event| {
            if event.key == Key::Escape {
                return Handled::Yes;
            }
            Handled::No
        });
}
