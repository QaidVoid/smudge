//! Devtools that Smudge draws over a running app's window.
//!
//! The crate builds the devtools overlay: a panel on a root as large as the
//! window, meant to run as an app of its own drawn over the app a window
//! shows, so the app keeps working underneath and its own frames, numbers
//! and pixels stay its own.

mod timeline;

use std::cell::Cell;
use std::rc::Rc;

pub use timeline::FrameRecord;
use timeline::{HEIGHT, Timeline, TimelineView};

use smudge_core::element::{Element, show};
use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Drag, Handled};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length, Overflow, pct, px};
use smudge_core::theme::ColorToken;
use smudge_core::tree::Dirty;
use smudge_core::widget::{Panel, panel};
use smudge_text::label;
use smudge_widgets::{button, column, row};

/// Where the panel opens, from the window's top left, in logical pixels.
const START: Point = Point { x: 16.0, y: 16.0 };

/// The width of the panel, in logical pixels.
const WIDTH: f32 = 320.0;

/// The width of the panel's border, in logical pixels.
const BORDER: f32 = 1.0;

/// The padding around the panel's body, in logical pixels.
const PAD: f32 = 8.0;

/// The width of the timeline, in logical pixels: the panel's inside.
const TIMELINE_WIDTH: f32 = WIDTH - 2.0 * (BORDER + PAD);

/// How much of the panel's top stays in the window when a drag or a resize
/// takes it toward the bottom edge, in logical pixels, so its title bar can
/// be grabbed again.
const KEEP: f32 = 32.0;

/// What the devtools show, which the window that runs them feeds and the
/// overlay reads. Cloning the handle is cheap, and every clone shares the
/// same state.
#[derive(Clone)]
pub struct Tools {
    /// The app's last frames.
    timeline: RwSignal<Rc<Timeline>>,
}

impl Default for Tools {
    fn default() -> Self {
        Tools::new()
    }
}

impl Tools {
    /// Creates the devtools state with no frames recorded.
    pub fn new() -> Tools {
        Tools {
            timeline: RwSignal::new(Rc::new(Timeline::default())),
        }
    }

    /// Records one frame of the app, which the timeline shows as its newest
    /// bar.
    pub fn record(&self, record: FrameRecord) {
        self.timeline
            .update(|timeline| Rc::make_mut(timeline).push(record));
    }
}

/// Builds the devtools overlay: one panel over a root as large as the
/// window, whose size in logical pixels `viewport` reads, showing what
/// `tools` holds.
///
/// The root takes no pointer input of its own, so wherever no panel is, the
/// pointer reaches the app underneath. The panel moves with a drag on its
/// title bar, and the button on the bar collapses it to the bar and opens it
/// again. It stays inside the window, across a drag and a resize, as far as
/// the window is wide and tall enough to hold its title bar.
///
/// The panel shows the app's last frames as a timeline of stacked bars, one
/// per frame and one color per stage, with a line at the budget of a frame
/// at 60 frames a second. Hovering a bar describes its frame below.
///
/// ```
/// use smudge_core::geometry::Size;
/// use smudge_devtools::Tools;
///
/// let tools = Tools::new();
/// let overlay = smudge_devtools::overlay(&tools, || Size {
///     width: 800.0,
///     height: 600.0,
/// });
/// ```
pub fn overlay(tools: &Tools, viewport: impl Fn() -> Size + 'static) -> Element<Panel> {
    let viewport: Rc<dyn Fn() -> Size> = Rc::new(viewport);
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
        .inset({
            let viewport = Rc::clone(&viewport);
            move || {
                let at = inside(place.get(), viewport());
                Edges {
                    left: px(at.x),
                    top: px(at.y),
                    right: Length::Auto,
                    bottom: Length::Auto,
                }
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
        .child(show(move || shown.get(), {
            let tools = tools.clone();
            move || body(&tools)
        }));

    // The root hears the drags that begin on the title bar, in its own
    // coordinates, which stay put while the panel moves. The title bar sits
    // inside the border.
    panel()
        .width(pct(100.0))
        .height(pct(100.0))
        .on_drag(move |step| match (step, grab.get()) {
            (Drag::Moved(point), Some(grab)) => {
                let wanted = Point {
                    x: point.x - grab.x - BORDER,
                    y: point.y - grab.y - BORDER,
                };
                at.set(inside(wanted, viewport()));
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

/// Returns `at` moved as little as it takes for a panel there to stay inside
/// a window of `viewport`: its left and top edges in the window, its right
/// edge too while the window is wide enough, and at least [`KEEP`] of it
/// above the bottom edge.
fn inside(at: Point, viewport: Size) -> Point {
    Point {
        x: at.x.min(viewport.width - WIDTH).max(0.0),
        y: at.y.min(viewport.height - KEEP).max(0.0),
    }
}

/// The marker of the timeline's frames, which [`Element::bind`] keys on.
struct Frames;

/// Builds the panel's body: the frame timeline, and a line that describes
/// the frame under the pointer. The line follows the bars as new frames
/// move them, so it always describes the bar the pointer is over.
fn body(tools: &Tools) -> Element<Panel> {
    let hovered: RwSignal<Option<f32>> = RwSignal::new(None);
    let frames = tools.timeline.clone();
    let described = tools.timeline.clone();
    let (over, left, shown) = (hovered.clone(), hovered.clone(), hovered);
    let timeline = Element::new(TimelineView::default())
        .width(px(TIMELINE_WIDTH))
        .height(px(HEIGHT))
        .bind(
            Frames,
            move || frames.get(),
            |view, timeline| {
                view.timeline.copy_from(&timeline);
                Dirty::Paint
            },
        )
        .on_pointer_move(move |point| {
            over.set(Some(point.x));
        })
        .on_hover(move |inside| {
            if !inside {
                left.set(None);
            }
        });
    column()
        .width(pct(100.0))
        .padding(px(PAD))
        .child(timeline)
        .child(label(move || {
            let x = shown.get();
            described.with(|timeline| {
                x.and_then(|x| timeline.at(x, TIMELINE_WIDTH)).map_or_else(
                    || String::from("Hover a bar to see its frame."),
                    FrameRecord::describe,
                )
            })
        }))
}
