//! Two inputs, a plan to pick and a submit button, with a live preview:
//! how a two-way input binding feeds the widgets around it. A separator
//! divides the sections and a hyperlink links to the plans page.
//!
//! Run with `cargo run --example form -p smudge`.

use smudge::prelude::*;
use smudge::widgets::radio_group;

/// The height of one input field, in logical pixels.
const FIELD_HEIGHT: f32 = 32.0;

/// The border color of an input field.
const FIELD_BORDER: Color = Color::rgb(70, 74, 88);

/// The plan the form's radio group picks.
#[derive(Clone, PartialEq)]
enum Plan {
    Free,
    Pro,
}

/// Returns the name `plan` shows.
fn plan_name(plan: &Plan) -> &'static str {
    match plan {
        Plan::Free => "Free",
        Plan::Pro => "Pro",
    }
}

fn main() -> Result<(), PlatformError> {
    let name = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let plan = RwSignal::new(Plan::Free);
    let submitted = RwSignal::new(String::new());

    let preview_name = name.clone();
    let preview_email = email.clone();
    let preview_plan = plan.clone();
    let submit_name = name.clone();
    let submit_email = email.clone();
    let submit_plan = plan.clone();
    let shown_submitted = submitted.clone();

    run_with(
        WindowOptions {
            title: String::from("Smudge form"),
            size: Size {
                width: 380.0,
                height: 410.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
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
                        .padding_xy(Size {
                            width: 8.0,
                            height: 6.0,
                        })
                        .placeholder("Name")
                        .border(1.0)
                        .border_color(FIELD_BORDER),
                )
                .child(
                    text_input(email)
                        .width(pct(100.0))
                        .height(px(FIELD_HEIGHT))
                        .padding_xy(Size {
                            width: 8.0,
                            height: 6.0,
                        })
                        .placeholder("Email")
                        .border(1.0)
                        .border_color(FIELD_BORDER),
                )
                .child(separator())
                .child(
                    radio_group(plan)
                        .option(Plan::Free, "Free")
                        .option(Plan::Pro, "Pro")
                        .accessible_label("Plan"),
                )
                .child(button("Submit").primary().on_press(move || {
                    let chosen = submit_plan.get();
                    submitted.set(format!(
                        "Submitted {} <{}> on the {} plan",
                        submit_name.get(),
                        submit_email.get(),
                        plan_name(&chosen),
                    ));
                }))
                .child(label(move || {
                    let name = preview_name.get();
                    let email = preview_email.get();
                    let chosen = preview_plan.get();
                    if name.is_empty() && email.is_empty() {
                        String::from("Type a name and an email")
                    } else {
                        format!(
                            "Preview: {name} <{email}> on the {} plan",
                            plan_name(&chosen)
                        )
                    }
                }))
                .child(label(move || match shown_submitted.get().as_str() {
                    "" => String::from("Nothing submitted yet"),
                    text => text.to_owned(),
                }))
                .child(hyperlink("Compare the plans", || {
                    // Opening a URL is a service of the platform, which
                    // phones get through the platform layer. On the desktop,
                    // the app asks the system's opener itself, which hands
                    // back the opener process for the app to keep or reap:
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open")
                        .arg("https://smudge.example/plans")
                        .spawn();
                }))
        },
        |app| app.set_theme(Theme::dark()),
    )
}
