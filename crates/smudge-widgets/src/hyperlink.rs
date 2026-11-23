//! The hyperlink: a label that runs an action on a press, Enter or Space.

use smudge_core::element::Value;
use smudge_core::theme::ColorToken;
use smudge_core::window::Cursor;
use smudge_core::{element::Element, semantics::Role};
use smudge_text::{Label, label};

/// Creates a hyperlink: a label in the theme's [`LINK`](ColorToken::LINK)
/// color that runs `on_press` when a press releases over it, or when Enter
/// or Space is pressed while it holds the focus.
///
/// The link underlines while the pointer hovers it or the keyboard focuses
/// it, and shows the pointing cursor while the pointer is over it. It is a
/// Tab stop, and a screen reader hears it as a link named by its text.
///
/// The link opens nothing itself: `on_press` runs and the application
/// decides what following the link means, since opening a URL is a service
/// of the platform the app runs on.
///
/// The text can be fixed or read from signals:
///
/// ```
/// use smudge_core::reactive::{prelude::*, signal_local};
/// use smudge_widgets::hyperlink;
///
/// let (opens, set_opens) = signal_local(0u32);
/// let link = hyperlink("Read the guide", move || {
///     set_opens.set(opens.get() + 1);
/// });
/// ```
#[track_caller]
pub fn hyperlink(
    text: impl Into<Value<String>>,
    on_press: impl FnMut() + 'static,
) -> Element<Label> {
    label(text)
        .text_color(ColorToken::LINK)
        .cursor(Cursor::Pointer)
        .hover(|p| p.underline(true))
        .focused(|p| p.underline(true))
        .activate_on_keys()
        .on_press(on_press)
        .role(Role::Link)
}
