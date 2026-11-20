//! The live tab: hot reload's last steps and the theme file.

use smudge_core::element::Element;
use smudge_core::reactive::prelude::*;
use smudge_core::style::{Align, px};
use smudge_core::widget::Panel;
use smudge_text::label;
use smudge_widgets::{column, row};

use crate::look::{self, action, dot, gap, mono, note, rows, section};
use crate::{Switches, Tools, theme_file};

/// Builds the live tab.
pub(crate) fn live_tab(tools: &Tools) -> Element<Panel> {
    look::scrolling(
        look::tab_content()
            .child(hot_reload(tools))
            .child(theme(tools)),
    )
}

/// Builds the hot reload section: the switch that keeps the tree, and the
/// last few steps, newest first.
fn hot_reload(tools: &Tools) -> Element<Panel> {
    let (count, empty) = (tools.hot.clone(), tools.hot.clone());
    let events = tools.hot.clone();
    section(
        "Hot reload",
        "Run the app with cargo smudge dev, and each saved edit patches it while it runs.",
    )
    .child(crate::switch(
        tools,
        "Keep tree",
        "Apply patches without remounting, so all state stays. An edit to what the \
         build creates shows at the next remount, and a closure created after the \
         first patch keeps that patch's code.",
        None,
        |switches: &mut Switches| &mut switches.keep_tree,
    ))
    .child(look::when(
        move || empty.with(|events| events.is_empty()),
        || note("No patches yet.").text_color(look::FAINT),
    ))
    .child(column().gap(gap(4.0)).child(rows(
        move || count.with(|events| events.len()),
        move |index| event(&events, index),
    )))
}

/// Builds the row of the hot reload event at `index`, newest first: a dot
/// in its color, what happened, and its detail.
fn event(
    events: &smudge_core::reactive::RwSignal<
        std::rc::Rc<std::collections::VecDeque<crate::HotEvent>>,
    >,
    index: usize,
) -> Element<Panel> {
    let parts = {
        let events = events.clone();
        move || events.with(|events| events.get(index).map(crate::HotEvent::parts))
    };
    let (color, title, detail) = (parts.clone(), parts.clone(), parts);
    look::card()
        .gap(gap(2.0))
        .child(
            row()
                .gap(gap(8.0))
                .align_items(Align::Center)
                .child(dot(move || {
                    smudge_core::theme::ColorValue::from(
                        color().map_or(look::FAINT, |(color, _, _)| color),
                    )
                }))
                .child(label(move || {
                    title().map_or_else(String::new, |(_, title, _)| title)
                })),
        )
        .child(look::when(
            {
                let detail = detail.clone();
                move || detail().is_some_and(|(_, _, detail)| !detail.is_empty())
            },
            move || {
                let detail = detail.clone();
                mono(move || detail().map_or_else(String::new, |(_, _, detail)| detail))
                    .text_color(look::MUTED)
            },
        ))
}

/// Builds the theme file section: which file is watched, the button that
/// writes the theme as a file to start from, and the lines the last read
/// skipped.
fn theme(tools: &Tools) -> Element<Panel> {
    let watching = theme_file::watched().map_or_else(
        || {
            String::from(
                "Call smudge::devtools::watch_theme in the app to watch a file. Each line \
                 sets a token, such as accent = #e07a3a.",
            )
        },
        |(path, _)| format!("Watching {}. Saving it recolors the app.", path.display()),
    );
    let written = tools.theme_written.clone();
    let shown = tools.theme_written.clone();
    let (count, skipped, any) = (
        tools.theme_skipped.clone(),
        tools.theme_skipped.clone(),
        tools.theme_skipped.clone(),
    );
    section(
        "Theme file",
        "Tune the theme's colors and radii from a text file while the app runs.",
    )
    .child(note(watching))
    .child(
        row()
            .gap(gap(10.0))
            .align_items(Align::Center)
            .child(action("Write theme").on_press(move || written.set(Some(crate::write_theme()))))
            .child(
                note("Saves the theme to target/smudge-devtools/theme.txt as a start.")
                    .shrink(1.0)
                    .min_width(px(0.0)),
            ),
    )
    .child(mono(move || shown.get().unwrap_or_default()).text_color(look::MUTED))
    .child(look::when(
        move || any.with(|skipped| !skipped.is_empty()),
        || label("Lines the last read skipped").text_color(look::WARN),
    ))
    .child(column().child(rows(
        move || count.with(Vec::len),
        move |index| {
            let skipped = skipped.clone();
            mono(move || skipped.with(|skipped| skipped.get(index).cloned().unwrap_or_default()))
                .text_color(look::WARN)
        },
    )))
}
