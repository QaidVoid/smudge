//! Two inputs, a submit button and a live preview: how a two-way input
//! binding feeds the widgets around it.
//!
//! Run with `cargo run --example form -p smudge`.

use smudge::prelude::*;

/// The height of one input field, in logical pixels.
const FIELD_HEIGHT: f32 = 32.0;

/// The border color of an input field.
const FIELD_BORDER: Color = Color::rgb(70, 74, 88);

fn main() -> Result<(), PlatformError> {
    let name = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let submitted = RwSignal::new(String::new());

    let preview_name = name.clone();
    let preview_email = email.clone();
    let submit_name = name.clone();
    let submit_email = email.clone();
    let shown_submitted = submitted.clone();

    run(
        WindowOptions {
            title: String::from("Smudge form"),
            size: Size {
                width: 380.0,
                height: 300.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            repaint: Default::default(),
            show_damage: false,
            renderer: RendererChoice::default(),
            stats: None,
            on_frame: None,
            on_event: None,
            frame_limit: None,
        },
        move || {
            column()
                .padding(px(24.0))
                .gap(Size {
                    width: 0.0,
                    height: 12.0,
                })
                .align_items(Align::Start)
                .width(pct(100.0))
                .child(
                    text_input(name)
                        .width(pct(100.0))
                        .height(px(FIELD_HEIGHT))
                        .placeholder("Name")
                        .border(1.0)
                        .border_color(FIELD_BORDER),
                )
                .child(
                    text_input(email)
                        .width(pct(100.0))
                        .height(px(FIELD_HEIGHT))
                        .placeholder("Email")
                        .border(1.0)
                        .border_color(FIELD_BORDER),
                )
                .child(button("Submit").primary().on_press(move || {
                    submitted.set(format!(
                        "Submitted {} <{}>",
                        submit_name.get(),
                        submit_email.get()
                    ));
                }))
                .child(label(move || {
                    let name = preview_name.get();
                    let email = preview_email.get();
                    if name.is_empty() && email.is_empty() {
                        String::from("Type a name and an email")
                    } else {
                        format!("Preview: {name} <{email}>")
                    }
                }))
                .child(label(move || match shown_submitted.get().as_str() {
                    "" => String::from("Nothing submitted yet"),
                    text => text.to_owned(),
                }))
        },
    )
}
