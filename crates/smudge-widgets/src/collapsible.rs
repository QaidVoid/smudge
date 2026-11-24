//! A section that folds away under a header.

use crate::canvas::{Canvas, canvas};
use smudge_core::element::{Element, IntoElement, show};
use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Handled, Key};
use smudge_core::path::Path;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Align, px};
use smudge_core::theme::{ColorToken, ColorValue, RadiusToken};
use smudge_core::widget::{Panel, panel};

/// How wide and tall the disclosure triangle is, in logical pixels.
const DISCLOSURE: f32 = 12.0;

/// Creates a section that folds its content away under a header.
///
/// The header is a row with a disclosure triangle in front of `header`, and
/// is a Tab stop. A press, Enter or Space toggles `open`; Right opens and
/// Left closes. While `open` holds, the content mounts below the header
/// through [`show`], and a closed section holds no content at all, so it
/// costs nothing.
///
/// A screen reader hears the header as a button whose expanded state
/// follows `open`.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_text::label;
/// use smudge_widgets::collapsible;
///
/// let advanced = RwSignal::new(false);
/// let section = collapsible(
///     advanced,
///     label("Advanced"),
///     || label("Expert settings live here"),
/// );
/// ```
#[track_caller]
pub fn collapsible<E, H>(
    open: RwSignal<bool>,
    header: H,
    content: impl Fn() -> E + 'static,
) -> Element<Panel>
where
    E: IntoElement,
    H: IntoElement,
{
    let expanded = open.clone();
    let toggled = open.clone();
    let folded = open.clone();
    let visible = open.clone();
    panel()
        .column()
        .child(
            panel()
                .row()
                .gap(Size {
                    width: 8.0,
                    height: 0.0,
                })
                .align_items(Align::Center)
                .padding(px(2.0))
                .corner_radius(RadiusToken::SMALL)
                .focused(|patch| patch.border_color(ColorToken::FOCUS_RING))
                .role(Role::Button)
                .accessible_expanded(move || expanded.get())
                .focusable()
                .activate_on_keys()
                .on_press(move || toggled.update(|open| *open = !*open))
                .on_key(move |event| {
                    if !event.pressed {
                        return Handled::No;
                    }
                    match event.key {
                        Key::ArrowRight => {
                            folded.set(true);
                            Handled::Yes
                        }
                        Key::ArrowLeft => {
                            folded.set(false);
                            Handled::Yes
                        }
                        _ => Handled::No,
                    }
                })
                .child(disclosure(open))
                .child(header),
        )
        .child(show(move || visible.get(), content))
}

/// Draws a triangle that points along the fold: right while closed, down
/// while open.
#[track_caller]
fn disclosure(open: RwSignal<bool>) -> Element<Canvas> {
    canvas(move |painter| {
        let size = painter.size();
        let (width, height) = (size.width, size.height);
        let inset = 1.0;
        let (left, top, right, bottom) = (inset, inset, width - inset, height - inset);
        // A fifth of the way in from each leading edge keeps the point of
        // the triangle from reaching the corner.
        let lead = 0.2;
        let points = if open.get() {
            [
                Point {
                    x: left,
                    y: top + (bottom - top) * lead,
                },
                Point {
                    x: right,
                    y: top + (bottom - top) * lead,
                },
                Point {
                    x: (left + right) / 2.0,
                    y: bottom,
                },
            ]
        } else {
            [
                Point {
                    x: left + (right - left) * lead,
                    y: top,
                },
                Point {
                    x: left + (right - left) * lead,
                    y: bottom,
                },
                Point {
                    x: right,
                    y: (top + bottom) / 2.0,
                },
            ]
        };
        painter.fill(
            &Path::polygon(points),
            ColorValue::Token(ColorToken::TEXT).resolve(painter.theme()),
        );
    })
    .size(Size {
        width: DISCLOSURE,
        height: DISCLOSURE,
    })
}
