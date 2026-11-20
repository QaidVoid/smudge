//! Modal dialogs: content centered over a dimmed window that holds the
//! keyboard until it closes.

use std::rc::Rc;

use smudge_core::animation::Spring;
use smudge_core::element::{AnyElement, Element, IntoElement, Layer, Placement};
use smudge_core::reactive::RwSignal;
use smudge_core::semantics::Role;
use smudge_core::style::{Align, Justify, px};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{Widget, panel};

/// A modal dialog, which [`modal`] creates. It mounts where it is placed
/// in the tree as a node that takes no room, and shows its dialog over the
/// whole window.
pub struct Modal<W: Widget> {
    /// The signal that keeps the dialog open.
    open: RwSignal<bool>,
    /// Builds the dialog's content each time it opens.
    content: Rc<dyn Fn() -> Element<W>>,
    /// The spring the dialog fades in and out under, when it fades.
    fade: Option<Spring>,
}

impl<W: Widget + 'static> Modal<W> {
    /// Returns the dialog fading in and out under `spring`, with the
    /// backdrop, which an app with reduced motion skips.
    pub fn fade(mut self, spring: Spring) -> Modal<W> {
        self.fade = Some(spring);
        self
    }
}

impl<W: Widget + 'static> IntoElement for Modal<W> {
    fn into_any(self) -> AnyElement {
        let Modal {
            open,
            content,
            fade,
        } = self;
        panel()
            .absolute()
            .width(px(0.0))
            .height(px(0.0))
            .overlay_in(Layer::Dialog, open, Placement::Cover, move || {
                let backdrop = panel()
                    .background(ColorToken::BACKDROP)
                    .align_items(Align::Center)
                    .justify_content(Justify::Center)
                    .child(content().role(Role::Dialog).trap_focus().autofocus_first());
                match fade {
                    Some(spring) => backdrop.enter(spring).exit(spring),
                    None => backdrop,
                }
            })
            .into_any()
    }
}

/// Creates a modal dialog that shows the element `content` builds, centered
/// over the window on a dimmed backdrop, while `open` reads `true`.
///
/// The dialog draws over everything opened before it, wherever it is
/// placed in the tree, so one inside a scroll view still covers the whole
/// window, and what opens from inside it draws above it. While it shows,
/// the backdrop takes every press and the dialog holds the keyboard: Tab
/// stays inside it, and nothing behind it hears a key. The first node
/// inside it that can take the focus takes it as it opens, unless a node
/// asks for it with [`autofocus`](Element::autofocus). Escape closes it, by
/// writing `false` to `open`, and the focus goes back to the node that held
/// it before it opened, or nowhere when that node is gone. A press on the
/// backdrop does not close it, since a dialog often holds work in progress.
///
/// The element `content` builds is the dialog itself: it reports the dialog
/// role, as modal, and a screen reader reaches nothing behind it while it
/// shows. Name it with [`accessible_label`](Element::accessible_label), and
/// give it the look of a dialog, a surface, a border and padding, as the
/// app's design asks. Two dialogs open at once stack in the order they
/// opened, and Escape closes the top one.
///
/// ```
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_core::theme::ColorToken;
/// use smudge_text::label;
/// use smudge_widgets::{button, column, modal};
///
/// let open = RwSignal::new(false);
/// let close = open.clone();
/// let dialog = modal(open.clone(), move || {
///     let close = close.clone();
///     column()
///         .padding(px(16.0))
///         .background(ColorToken::SURFACE)
///         .accessible_label("Delete the map?")
///         .child(label("Delete the map?"))
///         .child(button("Cancel").on_press(move || close.set(false)))
/// });
/// ```
#[track_caller]
pub fn modal<W: Widget + 'static>(
    open: RwSignal<bool>,
    content: impl Fn() -> Element<W> + 'static,
) -> Modal<W> {
    let content = smudge_core::hot::wrap(content);
    Modal {
        open,
        content: Rc::new(content),
        fade: None,
    }
}
