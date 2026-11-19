//! Devtools that Smudge draws over a running app's window.
//!
//! The crate builds the devtools overlay: a panel on a root as large as the
//! window, meant to run as an app of its own drawn over the app a window
//! shows, so the app keeps working underneath and its own frames, numbers
//! and pixels stay its own.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{Element, show};
use smudge_core::geometry::Point;
use smudge_core::input::{Drag, Handled};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, Overflow, pct, px};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{Panel, panel};
use smudge_text::label;
use smudge_widgets::{button, row};

/// Where the panel opens, from the window's top left, in logical pixels.
const START: Point = Point { x: 16.0, y: 16.0 };

/// The width of the panel, in logical pixels.
const WIDTH: f32 = 320.0;

/// The height of the panel's body while it is open, in logical pixels.
const BODY: f32 = 160.0;

/// The width of the panel's border, in logical pixels.
const BORDER: f32 = 1.0;

/// Builds the devtools overlay: one panel over a root as large as the
/// window.
///
/// The root takes no pointer input of its own, so wherever no panel is, the
/// pointer reaches the app underneath. The panel moves with a drag on its
/// title bar, and the button on the bar collapses it to the bar and opens it
/// again.
///
/// ```
/// let overlay = smudge_devtools::overlay();
/// ```
pub fn overlay() -> Element<Panel> {
    let at = RwSignal::new(START);
    let open = RwSignal::new(true);
    // Where the press that grabbed the title bar landed on it, which stays
    // under the pointer while the drag moves the panel.
    let grab: Rc<Cell<Option<Point>>> = Rc::new(Cell::new(None));

    let place = at.clone();
    let shown = open.clone();
    let caption = open.clone();
    let toggle = open.clone();
    let grabbed = Rc::clone(&grab);
    let title = row()
        .padding(px(6.0))
        .background(ColorToken::SURFACE_HOVER)
        .on_pointer_down(move |point| grabbed.set(Some(point)))
        .child(label("Devtools").grow(1.0))
        .child(
            button(move || String::from(if caption.get() { "Collapse" } else { "Open" }))
                .on_press(move || toggle.update(|open| *open = !*open)),
        );
    let window = panel()
        .absolute()
        .inset(move || {
            let at = place.get();
            Edges {
                left: px(at.x),
                top: px(at.y),
                right: Length::Auto,
                bottom: Length::Auto,
            }
        })
        .width(px(WIDTH))
        .column()
        .background(ColorToken::SURFACE)
        .border(BORDER)
        .border_color(ColorToken::BORDER)
        .corner_radius(6.0)
        .overflow(Overflow::Clip)
        .child(title)
        .child(show(move || shown.get(), || panel().height(px(BODY))));

    // The root hears the drags that begin on the title bar, in its own
    // coordinates, which stay put while the panel moves. The title bar sits
    // inside the border, and never leaves the window by its top or its left,
    // so it can always be grabbed again.
    panel()
        .width(pct(100.0))
        .height(pct(100.0))
        .on_drag(move |step| match (step, grab.get()) {
            (Drag::Moved(point), Some(grab)) => {
                at.set(Point {
                    x: (point.x - grab.x - BORDER).max(0.0),
                    y: (point.y - grab.y - BORDER).max(0.0),
                });
                Handled::Yes
            }
            (Drag::Moved(_), None) => Handled::No,
            _ => {
                grab.set(None);
                Handled::No
            }
        })
        .child(window)
}
