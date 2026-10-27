//! The button: a focusable panel with a label that presses and that Enter and
//! Space activate.

use smudge_core::element::{Element, Value};
use smudge_core::geometry::Size;
use smudge_core::semantics::Role;
use smudge_core::theme::{ColorToken, RadiusToken};
use smudge_core::widget::{Panel, panel};
use smudge_text::label;

/// The padding around the label.
const PADDING: Size = Size {
    width: 12.0,
    height: 6.0,
};

/// The width of the focus ring.
///
/// The button always lays out with a border this wide and paints it only
/// while focused, so taking the focus repaints the button without moving it
/// or its neighbors.
const FOCUS_BORDER: f32 = 2.0;

/// Creates a push button that shows `label_text`.
///
/// The button is focusable and can be pressed with
/// [`on_press`](Element::on_press), which is chained on the element it
/// returns. While it holds the keyboard focus, a press of Enter or Space
/// fires the same handler; the activation is what
/// [`activate_on_keys`](Element::activate_on_keys) provides, and a key repeat
/// fires nothing. The focus ring draws only while the focus last moved by
/// keyboard navigation, so a press on the button draws none.
///
/// The colors come from the theme: [`ColorToken::SURFACE`] at rest,
/// [`ColorToken::SURFACE_HOVER`] under the pointer,
/// [`ColorToken::SURFACE_PRESSED`] while a press is held, and a
/// [`ColorToken::FOCUS_RING`] border of two logical pixels while the focus
/// came from the keyboard. The border's space is reserved in every state, so
/// the ring appears without a layout change.
///
/// ```
/// use smudge_core::reactive::{prelude::*, signal_local};
/// use smudge_widgets::button;
///
/// let (clicks, set_clicks) = signal_local(0u32);
/// let ok = button("OK").on_press(move || {
///     set_clicks.set(clicks.get() + 1);
/// });
/// ```
pub fn button(label_text: impl Into<Value<String>>) -> Element<Panel> {
    let label_text = label_text.into();
    panel()
        .padding_xy(PADDING)
        .corner_radius(RadiusToken::MEDIUM)
        .background(ColorToken::SURFACE)
        .hover(|p| p.background(ColorToken::SURFACE_HOVER))
        .pressed(|p| p.background(ColorToken::SURFACE_PRESSED))
        .border(FOCUS_BORDER)
        .focused(|p| p.border_color(ColorToken::FOCUS_RING))
        .focusable()
        .activate_on_keys()
        .role(Role::Button)
        .accessible_label(label_text.clone())
        .child(label(label_text))
}

/// The appearance variants of a [`button`].
///
/// Import the trait to use them:
///
/// ```
/// use smudge_widgets::ButtonExt;
/// use smudge_widgets::button;
///
/// let save = button("Save").primary();
/// ```
pub trait ButtonExt: Sized {
    /// Styles the button as the primary action of its context.
    ///
    /// The background becomes [`ColorToken::ACCENT`], with
    /// [`ColorToken::ACCENT_HOVER`] under the pointer and
    /// [`ColorToken::ACCENT_PRESSED`] while a press is held, and the label
    /// paints in [`ColorToken::ACCENT_TEXT`]. The focus ring paints in
    /// [`ColorToken::ACCENT_TEXT`] too, because a ring in the focus color
    /// could match the accent it is drawn over. The hovered, pressed and
    /// focused patches replace the ones the button had, so call it before
    /// setting patches of your own.
    fn primary(self) -> Self;
}

impl ButtonExt for Element<Panel> {
    fn primary(self) -> Self {
        self.background(ColorToken::ACCENT)
            .hover(|p| p.background(ColorToken::ACCENT_HOVER))
            .pressed(|p| p.background(ColorToken::ACCENT_PRESSED))
            .focused(|p| p.border_color(ColorToken::ACCENT_TEXT))
            .style_children(|style| {
                style.paint.text_color = Some(ColorToken::ACCENT_TEXT.into());
            })
    }
}
