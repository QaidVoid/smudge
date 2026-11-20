//! The panel's look: its colors and text sizes, and the small pieces the
//! tabs are built from.
//!
//! The panel paints in colors of its own rather than the app's theme, so it
//! reads the same over every app.

use smudge_core::element::{Element, Slot, for_each, show};
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role};
use smudge_core::style::{Align, Justify, pct, px};
use smudge_core::theme::ColorValue;
use smudge_core::widget::Panel;
use smudge_text::{FontFamily, FontWeight, GenericFamily, Label, LabelExt, TextStyle, label};
use smudge_widgets::{button, column, row, scroll};

/// The panel's background.
pub(crate) const BACKGROUND: Color = Color::rgb(24, 24, 27);
/// The title bar's background.
pub(crate) const BAR: Color = Color::rgb(32, 32, 37);
/// Below the panel's background, as the tab bar's track and the timeline.
pub(crate) const SUNKEN: Color = Color::rgb(16, 16, 19);
/// A card or a button at rest.
pub(crate) const RAISED: Color = Color::rgb(39, 39, 45);
/// A row or a button under the pointer.
pub(crate) const HOVER: Color = Color::rgb(51, 51, 58);
/// A row or a button while a press is held.
pub(crate) const PRESSED: Color = Color::rgb(62, 62, 70);
/// Borders and dividers.
pub(crate) const LINE: Color = Color::rgb(52, 52, 59);
/// The track of a switch that is off, which stands out from a hovered row.
pub(crate) const TRACK: Color = Color::rgb(86, 86, 96);
/// Text.
pub(crate) const TEXT: Color = Color::rgb(236, 236, 240);
/// Text that explains or labels.
pub(crate) const MUTED: Color = Color::rgb(161, 161, 170);
/// Text that matters least, and hidden nodes, still readable on every
/// background of the panel.
pub(crate) const FAINT: Color = Color::rgb(142, 142, 152);
/// What is on or chosen.
pub(crate) const ACCENT: Color = Color::rgb(129, 140, 248);
/// The background of a chosen row.
pub(crate) const ACCENT_SOFT: Color = Color::rgba(129, 140, 248, 48);
/// Text on [`ACCENT`].
pub(crate) const ON_ACCENT: Color = Color::rgb(17, 17, 32);
/// What went well.
pub(crate) const GOOD: Color = Color::rgb(74, 222, 128);
/// What needs a look.
pub(crate) const WARN: Color = Color::rgb(251, 191, 36);
/// What failed.
pub(crate) const BAD: Color = Color::rgb(248, 113, 113);

/// The size of most text, in logical pixels.
pub(crate) const BODY: f32 = 12.0;
/// The size of explanations and secondary text, in logical pixels.
pub(crate) const SMALL: f32 = 11.0;

/// The text style every label of the panel starts from.
pub(crate) fn text_style() -> TextStyle {
    TextStyle {
        size: BODY,
        line_height: 1.4,
        color: ColorValue::Literal(TEXT),
        ..TextStyle::default()
    }
}

/// Builds a gap of `size` logical pixels both ways.
pub(crate) fn gap(size: f32) -> Size {
    Size {
        width: size,
        height: size,
    }
}

/// Builds text that explains, in the muted color and the small size.
#[track_caller]
pub(crate) fn note(text: impl Into<smudge_core::element::Value<String>>) -> Element<Label> {
    label(text).text_size(SMALL).text_color(MUTED)
}

/// Builds text in the monospace family, for places in the source, numbers
/// and styles.
#[track_caller]
pub(crate) fn mono(text: impl Into<smudge_core::element::Value<String>>) -> Element<Label> {
    label(text)
        .family(FontFamily::from(GenericFamily::Monospace))
        .text_size(SMALL)
}

/// Builds a bold heading.
#[track_caller]
pub(crate) fn heading(text: impl Into<smudge_core::element::Value<String>>) -> Element<Label> {
    label(text).weight(FontWeight::BOLD)
}

/// Builds a section: a heading, a line that says what the section is for,
/// and below them the children the caller adds.
#[track_caller]
pub(crate) fn section(title: &'static str, about: &'static str) -> Element<Panel> {
    column().gap(gap(6.0)).child(
        column()
            .gap(gap(1.0))
            .child(heading(title))
            .child(note(about)),
    )
}

/// Builds the column a tab's content goes in, with the panel's padding at
/// its sides and its bottom.
#[track_caller]
pub(crate) fn tab_content() -> Element<Panel> {
    column()
        .width(pct(100.0))
        .padding_xy(Size {
            width: crate::PAD,
            height: 6.0,
        })
        .gap(gap(16.0))
}

/// Builds a tab that scrolls `content` within the height the panel leaves
/// it.
#[track_caller]
pub(crate) fn scrolling(content: Element<Panel>) -> Element<Panel> {
    column()
        .min_height(px(0.0))
        .child(scroll(content).grow(1.0).min_height(px(0.0)))
}

/// Builds a card: a raised, rounded column.
#[track_caller]
pub(crate) fn card() -> Element<Panel> {
    column()
        .padding(px(10.0))
        .gap(gap(6.0))
        .background(RAISED)
        .corner_radius(8.0)
}

/// Builds a button in the panel's colors, showing `text`.
#[track_caller]
pub(crate) fn action(text: impl Into<smudge_core::element::Value<String>>) -> Element<Panel> {
    button(text)
        .shrink(0.0)
        .padding_xy(Size {
            width: 10.0,
            height: 3.0,
        })
        .corner_radius(6.0)
        .background(RAISED)
        .hover(|p| p.background(HOVER))
        .pressed(|p| p.background(PRESSED))
        .focused(|p| p.border_color(ACCENT))
        .style_children(|style| style.paint.text_color = Some(TEXT.into()))
}

/// Builds a small dot of `color`.
#[track_caller]
pub(crate) fn dot(color: impl Into<smudge_core::element::Value<ColorValue>>) -> Element<Panel> {
    smudge_core::widget::panel()
        .width(px(8.0))
        .height(px(8.0))
        .shrink(0.0)
        .corner_radius(4.0)
        .background(color)
}

/// Builds a small rounded tag that reads `text`, in `color` over a faint
/// wash of it.
#[track_caller]
pub(crate) fn chip(
    text: impl Into<smudge_core::element::Value<String>>,
    color: Color,
) -> Element<Panel> {
    row()
        .padding_xy(Size {
            width: 6.0,
            height: 0.0,
        })
        .corner_radius(4.0)
        .background(Color::rgba(color.r, color.g, color.b, 36))
        .child(label(text).text_size(SMALL).text_color(color))
}

/// Builds a row that turns a setting on and off: a switch, the setting's
/// `name`, and a line that says what it does. A `swatch` shows the color
/// the setting draws in, when it draws.
///
/// The whole row takes the press, and Enter and Space while it holds the
/// focus.
#[track_caller]
pub(crate) fn switch_row(
    name: &'static str,
    about: &'static str,
    swatch: Option<Color>,
    on: impl Fn() -> bool + Clone + 'static,
    toggle: impl FnMut() + 'static,
) -> Element<Panel> {
    let (checked, track, knob) = (on.clone(), on.clone(), on);
    let switch = row()
        .width(px(30.0))
        .height(px(18.0))
        .shrink(0.0)
        .padding(px(3.0))
        .corner_radius(9.0)
        .align_items(Align::Center)
        .justify_content(move || if knob() { Justify::End } else { Justify::Start })
        .background(move || ColorValue::from(if track() { ACCENT } else { TRACK }))
        .child(
            smudge_core::widget::panel()
                .width(px(12.0))
                .height(px(12.0))
                .corner_radius(6.0)
                .background(TEXT),
        );
    let words = column()
        .grow(1.0)
        .shrink(1.0)
        .min_width(px(0.0))
        .child(label(name))
        .child(note(about));
    let mut built = row()
        .padding_xy(Size {
            width: 8.0,
            height: 7.0,
        })
        .gap(gap(10.0))
        .align_items(Align::Center)
        .corner_radius(6.0)
        .border(1.0)
        .border_color(Color::TRANSPARENT)
        .hover(|p| p.background(HOVER))
        .pressed(|p| p.background(PRESSED))
        .focused(|p| p.border_color(ACCENT))
        .role(Role::Switch)
        .accessible_label(name)
        .accessible_description(about)
        .accessible_checked(move || {
            if checked() {
                Checked::True
            } else {
                Checked::False
            }
        })
        .focusable()
        .activate_on_keys()
        .on_press(toggle)
        .child(switch)
        .child(words);
    if let Some(color) = swatch {
        built = built.child(
            smudge_core::widget::panel()
                .width(px(14.0))
                .height(px(14.0))
                .shrink(0.0)
                .corner_radius(3.0)
                .border(2.0)
                .border_color(Color::rgb(color.r, color.g, color.b))
                .background(Color::rgba(color.r, color.g, color.b, 60)),
        );
    }
    built
}

/// Builds a row of `name` at the left and `value` at the right, for the
/// facts of a node.
#[track_caller]
pub(crate) fn fact(
    name: &'static str,
    value: impl Into<smudge_core::element::Value<String>>,
) -> Element<Panel> {
    row()
        .gap(gap(8.0))
        .child(note(name).width(px(64.0)).shrink(0.0))
        .child(label(value).grow(1.0).shrink(1.0).min_width(px(0.0)))
}

/// Shows what `build` builds while `shown` holds, as a column that takes no
/// room, not even a gap, while it does not.
#[track_caller]
pub(crate) fn when<E: smudge_core::element::IntoElement>(
    shown: impl Fn() -> bool + Clone + 'static,
    build: impl Fn() -> E + 'static,
) -> Element<Slot> {
    let hidden = shown.clone();
    show(shown, build).column().hidden(move || !hidden())
}

/// Builds a list of `count` rows, built by `build` from their index, which
/// rebuilds only the rows that come and go as the count changes. Each row
/// reads what it shows by its index, so it follows the data on its own.
#[track_caller]
pub(crate) fn rows<E: smudge_core::element::IntoElement>(
    count: impl Fn() -> usize + 'static,
    build: impl Fn(usize) -> E + 'static,
) -> Element<Slot> {
    for_each(
        move || (0..count()).collect(),
        |index: &usize| *index,
        build,
    )
    .column()
}

/// The deepest level a tree's indent shows, past which deeper nodes line
/// up, so their names stay in the panel.
const INDENT_LEVELS: usize = 12;

/// Builds a gap as wide as `depth` levels of a tree's indent, at most
/// [`INDENT_LEVELS`].
#[track_caller]
pub(crate) fn indent(depth: impl Fn() -> usize + 'static) -> Element<Panel> {
    smudge_core::widget::panel()
        .shrink(0.0)
        .width(move || px(depth().min(INDENT_LEVELS) as f32 * 12.0))
}

/// Writes `count` with `one` or `many` after it, as `1 node` or `3 nodes`.
pub(crate) fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Returns what reads `pick` of the value `signal` holds, or the default
/// while it holds none, for the bindings of a tab that shows data the window
/// hands it.
pub(crate) fn read<S: 'static, T: Default + 'static>(
    signal: &RwSignal<Option<std::rc::Rc<S>>>,
    pick: impl Fn(&S) -> T + 'static,
) -> impl Fn() -> T + Clone + 'static {
    let signal = signal.clone();
    let pick = std::rc::Rc::new(pick);
    move || {
        signal.with(|value| {
            value
                .as_deref()
                .map(|value| pick(value))
                .unwrap_or_default()
        })
    }
}

/// Writes `value` with no decimals when it is whole and one otherwise, as
/// the panel writes logical pixels.
pub(crate) fn number(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value}")
    } else {
        format!("{value:.1}")
    }
}

/// Writes a rectangle's size as `width × height`.
pub(crate) fn size_text(rect: smudge_core::geometry::Rect) -> String {
    format!("{} \u{d7} {}", number(rect.width()), number(rect.height()))
}
