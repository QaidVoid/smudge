//! Tooltips with text, and the dropdown, a field that picks one choice from
//! a list.

use std::rc::Rc;

use smudge_core::element::{Element, Placement};
use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Handled, Key};
use smudge_core::paint::PaintOp;
use smudge_core::path::{LineCap, LineJoin, Path, PathPaint, Stroke};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Align, Direction, px};
use smudge_core::theme::{ColorToken, ColorValue, RadiusToken};
use smudge_core::widget::{PaintCx, Panel, Widget};
use smudge_text::{LabelExt, label};

use crate::containers::{column, row};

/// The builder of a tooltip with text.
pub trait TooltipExt: Sized {
    /// Shows `text` in a tooltip below the node once the pointer rests on
    /// it, as [`Element::tooltip`] describes, and gives the node `text` as
    /// its accessible description, which a screen reader announces.
    fn tooltip_text(self, text: impl Into<String>) -> Self;
}

impl<W: Widget + 'static> TooltipExt for Element<W> {
    fn tooltip_text(self, text: impl Into<String>) -> Self {
        let text = text.into();
        let shown = text.clone();
        self.accessible_description(text).tooltip(move || {
            row()
                .role(Role::Tooltip)
                .padding_xy(Size {
                    width: 8.0,
                    height: 4.0,
                })
                .margin(px(4.0))
                .background(ColorToken::SURFACE)
                .border(1.0)
                .border_color(ColorToken::BORDER)
                .corner_radius(RadiusToken::SMALL)
                .child(label(shown.clone()).text_size(12.0))
        })
    }
}

/// The arrow at the right of a dropdown, in the field's text color.
struct Chevron;

impl Widget for Chevron {
    /// Strokes a downward chevron across the content box.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let color = cx
            .style
            .paint
            .text_color
            .unwrap_or(ColorValue::Token(ColorToken::TEXT_MUTED))
            .resolve(cx.theme);
        let area = cx.content;
        let at = |x: f32, y: f32| Point {
            x: area.x0 + x * area.width(),
            y: area.y0 + y * area.height(),
        };
        let chevron = Path::polyline([at(0.2, 0.35), at(0.5, 0.65), at(0.8, 0.35)]);
        out.push(PaintOp::Path(Rc::new(PathPaint::stroked(
            chevron,
            Stroke::new(1.5, color)
                .with_cap(LineCap::Round)
                .with_join(LineJoin::Round),
        ))));
    }
}

/// The key of the binding that closes a dropdown's list when its choices
/// change.
struct Watch;

/// Creates a field that shows the choice `selected` picks out of the list
/// `choices` returns, and opens the list below it to pick another.
///
/// A press, Enter, Space or Down opens the list, with the picked choice
/// focused. Up and Down walk the list, a letter goes to the next choice
/// that starts with it, Enter or a press picks one, which writes its index
/// to `selected` and closes the list, and Escape or a press outside closes
/// it unchanged. `choices` runs whenever the field shows, so the list
/// follows the app's state, and a list that changes while open closes. An
/// index past the list shows nothing, and an empty list does not open.
///
/// Screen readers hear a combo box whose value is the picked choice, and a
/// list box of options, the picked one selected. Give the field a name with
/// [`accessible_label`](Element::accessible_label).
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::dropdown;
///
/// let size = RwSignal::new(1_usize);
/// let field = dropdown(|| vec![String::from("Small"), String::from("Medium")], size)
///     .accessible_label("Brush size");
/// ```
#[track_caller]
pub fn dropdown(
    choices: impl Fn() -> Vec<String> + 'static,
    selected: RwSignal<usize>,
) -> Element<Panel> {
    let choices: Rc<dyn Fn() -> Vec<String>> = Rc::new(choices);
    let open = crate::signal(false);
    let current = {
        let (choices, selected) = (Rc::clone(&choices), selected.clone());
        move || {
            choices()
                .into_iter()
                .nth(selected.get())
                .unwrap_or_default()
        }
    };
    let (press, key, expanded) = (open.clone(), open.clone(), open.clone());
    // An empty list opens nothing, and a list that changes while open
    // closes, since what it shows is no longer what a pick would write.
    let (pressed_choices, key_choices) = (Rc::clone(&choices), Rc::clone(&choices));
    let watched = {
        let (choices, open) = (Rc::clone(&choices), open.clone());
        let last: std::cell::RefCell<Option<Vec<String>>> = std::cell::RefCell::new(None);
        move || {
            let now = choices();
            let mut last = last.borrow_mut();
            if last.as_ref().is_some_and(|last| *last != now) && open.read(|open| *open) {
                open.set(false);
            }
            *last = Some(now);
        }
    };
    let shown_value = current.clone();
    let list = {
        let (choices, selected, open) = (Rc::clone(&choices), selected.clone(), open.clone());
        move || options(choices(), selected.clone(), open.clone())
    };
    row()
        .gap(Size {
            width: 8.0,
            height: 0.0,
        })
        .align_items(Align::Center)
        .padding_xy(Size {
            width: 10.0,
            height: 5.0,
        })
        .min_width(px(120.0))
        .background(ColorToken::SURFACE)
        .border(1.0)
        .border_color(ColorToken::BORDER)
        .corner_radius(RadiusToken::MEDIUM)
        .hover(|p| p.background(ColorToken::SURFACE_HOVER))
        .focused(|p| p.border_color(ColorToken::FOCUS_RING))
        .role(Role::ComboBox)
        .accessible_value(shown_value)
        .focusable()
        .accessible_expanded(move || expanded.get())
        .bind(Watch, watched, |_, ()| smudge_core::tree::Dirty::Clean)
        .on_press(move || {
            if press.get() || !pressed_choices().is_empty() {
                press.update(|open| *open = !*open);
            }
        })
        .on_key(move |event| {
            // Keys from inside the open list bubble here too, and belong to
            // the list.
            let opens = matches!(event.key, Key::Enter | Key::Space | Key::ArrowDown);
            if event.pressed && !event.repeat && opens && !key.get() && !key_choices().is_empty() {
                key.set(true);
                return Handled::Yes;
            }
            Handled::No
        })
        .child(label(current).grow(1.0))
        .child(
            Element::new(Chevron)
                .width(px(12.0))
                .height(px(12.0))
                .shrink(0.0),
        )
        .overlay(open, Placement::Below, list)
}

/// Builds the open list of `choices`, the one `selected` picks focused.
fn options(
    choices: Vec<String>,
    selected: RwSignal<usize>,
    open: RwSignal<bool>,
) -> Element<Panel> {
    // The picked choice takes the focus, or the first one when the index
    // names none.
    let count = choices.len();
    let picked = Some(selected.get())
        .filter(|picked| *picked < count)
        .unwrap_or(0);
    choices.into_iter().enumerate().fold(
        column()
            .role(Role::ListBox)
            .padding(px(4.0))
            .min_width(px(120.0))
            .margin(px(2.0))
            .background(ColorToken::SURFACE)
            .border(1.0)
            .border_color(ColorToken::BORDER)
            .corner_radius(RadiusToken::MEDIUM)
            .trap_focus()
            .navigate_with_arrows(Direction::Column),
        |list, (index, choice)| {
            let (choose, close, state) = (selected.clone(), open.clone(), selected.clone());
            let mut option = row()
                .padding_xy(Size {
                    width: 8.0,
                    height: 4.0,
                })
                .corner_radius(RadiusToken::SMALL)
                .hover(|p| p.background(ColorToken::SURFACE_HOVER))
                .focused(|p| p.background(ColorToken::SURFACE_HOVER))
                .role(Role::ListBoxOption)
                .accessible_label(choice.clone())
                .accessible_selected(move || state.get() == index)
                .focusable()
                .activate_on_keys()
                .on_press(move || {
                    close.set(false);
                    choose.set(index);
                })
                .child(label(choice));
            if index == picked {
                option = option.autofocus();
            }
            list.child(option)
        },
    )
}
