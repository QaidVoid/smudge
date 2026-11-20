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
fn own_fonts() {
    // The page embeds a font file of the application's own. The bundled
    // mono stands in for it.
    macro_rules! include_bytes {
        ($path:literal) => {
            std::include_bytes!("../../../../crates/smudge-text/assets/fonts/NotoSansMono-Regular.ttf")
        };
    }
    use smudge::prelude::*;
    use smudge::text::{FontFamily, TextSystem};
    
    fn main() -> Result<(), Box<dyn std::error::Error>> {
        TextSystem::with(|system| system.register_font(include_bytes!("../assets/Brand.ttf")))?;
        run(WindowOptions::default(), || {
            provide_text_style(TextStyle {
                family: FontFamily::named("Brand"),
                ..TextStyle::default()
            });
            column().child(label("Drawn in the brand font"))
        })?;
        Ok(())
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
fn rich_text_sample() {
    use std::rc::Rc;
    
    use smudge::prelude::*;
    
    let status = RwSignal::new(String::from("passing"));
    let shown = status.clone();
    let summary = rich_text(move || {
        Rc::from([
            Span::new("The build is ", SpanStyle::default()),
            Span::new(
                shown.get(),
                SpanStyle {
                    weight: Some(FontWeight::BOLD),
                    color: Some(ColorToken::ACCENT.into()),
                    ..SpanStyle::default()
                },
            ),
            Span::new(". Read the ", SpanStyle::default()),
            Span::link("log", || println!("open the log")),
            Span::new(" or run ", SpanStyle::default()),
            Span::new(
                "cargo test",
                SpanStyle {
                    monospace: true,
                    ..SpanStyle::default()
                },
            ),
            Span::new(" again.", SpanStyle::default()),
        ])
    });
}

#[test]
fn selection_sample() {
    use std::rc::Rc;
    
    use smudge::prelude::*;
    
    let lines: Rc<[String]> = Rc::from([
        String::from("The build started."),
        String::from("All 412 tests passed."),
    ]);
    let texts = Rc::clone(&lines);
    let transcript = selection_scope(
        || Rc::from([0usize, 1]),
        move |at: &usize| texts[*at].clone(),
        move || {
            column()
                .child(label(lines[0].clone()).selectable(0usize))
                .child(label(lines[1].clone()).selectable(1usize))
        },
    );
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
                .padding_xy(Size { width: 8.0, height: 6.0 })
                .placeholder("you@example.com")
                .border(1.0)
                .border_color(ColorToken::BORDER),
        )
        .child(label(move || format!("Sending to {}", preview.get())));
}

#[test]
fn password_form() {
    let password = RwSignal::new(String::new());
    let shown = RwSignal::new(false);
    let masked = shown.clone();
    
    let fields = column()
        .gap(Size { width: 0.0, height: 8.0 })
        .child(
            text_input(password)
                .password(move || !masked.get())
                .accessible_label("Password")
                .width(px(280.0))
                .border(1.0)
                .border_color(ColorToken::BORDER),
        )
        .child(smudge::widgets::checkbox("Show password", shown));
}

#[test]
fn invalid_field() {
    /// The reason `text` is not a width, or nothing when it is one.
    fn problem(text: &str) -> String {
        match text.parse::<u32>() {
            Ok(_) => String::new(),
            Err(_) => String::from("Enter a whole number"),
        }
    }
    
    let width = RwSignal::new(String::from("24"));
    let (checked, described, shown) = (width.clone(), width.clone(), width.clone());
    
    let field = column()
        .child(
            text_input(width)
                .accessible_label("Width")
                .border(1.0)
                .border_color(ColorToken::BORDER)
                .invalid(move || checked.with(|text| !problem(text).is_empty()))
                .accessible_description(move || described.with(|text| problem(text))),
        )
        .child(label(move || shown.with(|text| problem(text))).text_color(ColorToken::ERROR));
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
fn middle_drag() {
    use smudge::core::input::{Handled, PointerButton};

    let offset = RwSignal::new(Point::default());
    let (grab, pan) = (RwSignal::new(Point::default()), offset.clone());
    let start = grab.clone();
    let view = panel()
        .size(Size { width: 320.0, height: 200.0 })
        .on_button_down(move |event| {
            if event.button != PointerButton::Middle {
                return Handled::No;
            }
            start.set(event.position);
            Handled::Yes
        })
        .on_button_move(move |event| {
            let from = grab.get();
            pan.update(|offset| {
                offset.x += event.position.x - from.x;
                offset.y += event.position.y - from.y;
            });
            grab.set(event.position);
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
