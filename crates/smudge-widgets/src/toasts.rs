//! Toasts: short notices that show in a corner of the window and leave on
//! their own.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::element::{Element, Layer, Placement, for_each};
use smudge_core::geometry::Size;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Announcer, Politeness};
use smudge_core::style::{Align, px};
use smudge_core::theme::{ColorToken, RadiusToken};
use smudge_core::timer::{after, now};
use smudge_core::widget::{Panel, panel};
use smudge_text::label;
use web_time::Instant;

use crate::containers::{column, row};

/// How long a toast shows when [`Toasts::show`] does not say.
pub const TOAST_DURATION: Duration = Duration::from_secs(4);

/// The most toasts that show at once. A new one closes the oldest.
pub const MAX_TOASTS: usize = 4;

/// One toast the host shows.
#[derive(Clone, Debug, PartialEq)]
struct Toast {
    /// Tells this toast from the others.
    id: u64,
    /// What it says.
    message: String,
    /// How long it shows while the pointer is not over it.
    duration: Duration,
}

/// A handle that shows toasts, which [`toast_host`] draws. It is cheap to
/// clone, and every clone shows toasts in the same host.
///
/// A toast never takes the keyboard focus. A screen reader hears it through
/// the host's announcer, politely, or at once for one shown with
/// [`Toasts::show_urgent`].
#[derive(Clone)]
pub struct Toasts {
    /// The toasts that show, oldest first.
    shown: RwSignal<Vec<Toast>>,
    /// The id the next toast takes.
    next: Rc<Cell<u64>>,
    /// Speaks each toast as it shows.
    announcer: Announcer,
}

impl Default for Toasts {
    fn default() -> Self {
        Toasts::new()
    }
}

impl Toasts {
    /// Creates a handle with no toast showing.
    pub fn new() -> Toasts {
        Toasts {
            shown: RwSignal::new(Vec::new()),
            next: Rc::new(Cell::new(0)),
            announcer: Announcer::new(),
        }
    }

    /// Shows `message` for [`TOAST_DURATION`].
    ///
    /// Call it from a handler, or from the handler of a message a worker
    /// sent, on the thread the window runs on. A toast beyond
    /// [`MAX_TOASTS`] closes the oldest.
    pub fn show(&self, message: impl Into<String>) {
        self.push(message.into(), TOAST_DURATION, Politeness::Polite);
    }

    /// Shows `message` for `duration`, as [`Toasts::show`] does. A duration
    /// too long for the clock to count, such as [`Duration::MAX`], keeps the
    /// toast until it is pressed.
    pub fn show_for(&self, message: impl Into<String>, duration: Duration) {
        self.push(message.into(), duration, Politeness::Polite);
    }

    /// Shows `message` for [`TOAST_DURATION`] and has a screen reader speak
    /// it at once, interrupting what it was saying, for what the user must
    /// hear now.
    pub fn show_urgent(&self, message: impl Into<String>) {
        self.push(message.into(), TOAST_DURATION, Politeness::Assertive);
    }

    /// Shows a toast and announces it.
    fn push(&self, message: String, duration: Duration, politeness: Politeness) {
        let id = self.next.get();
        self.next.set(id.wrapping_add(1));
        self.announcer.announce(message.clone(), politeness);
        self.shown.update(|shown| {
            shown.push(Toast {
                id,
                message,
                duration,
            });
            while shown.len() > MAX_TOASTS {
                shown.remove(0);
            }
        });
    }

    /// Closes the toast `id`, when it still shows.
    fn dismiss(&self, id: u64) {
        if self
            .shown
            .read(|shown| shown.iter().any(|toast| toast.id == id))
        {
            self.shown
                .update(|shown| shown.retain(|toast| toast.id != id));
        }
    }
}

/// Creates the element that draws the toasts `toasts` shows, in the window's
/// bottom right corner over everything else, newest at the bottom.
///
/// Mount it once, anywhere in the tree, and keep it mounted for as long as
/// toasts can show: it takes no room where it sits. A toast shown while no
/// host is mounted is not announced, and its time starts once a host draws
/// it. Each toast leaves after its duration, and sooner when pressed. Its
/// timer pauses while the pointer is over it, so it can be read, and runs
/// on when the pointer leaves.
///
/// ```
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_widgets::{Toasts, button, column, toast_host};
///
/// let toasts = Toasts::new();
/// let saved = toasts.clone();
/// let view = column()
///     .child(button("Save").on_press(move || saved.show("Saved")))
///     .child(toast_host(&toasts));
/// ```
#[track_caller]
pub fn toast_host(toasts: &Toasts) -> Element<Panel> {
    let open = crate::signal(true);
    let list = toasts.clone();
    panel()
        .absolute()
        .width(px(0.0))
        .height(px(0.0))
        .announcer(&toasts.announcer)
        .overlay_in(Layer::Toast, open, Placement::BottomRight, move || {
            let (ids, toasts) = (list.clone(), list.clone());
            for_each(
                move || {
                    ids.shown
                        .with(|shown| shown.iter().map(|toast| toast.id).collect())
                },
                |id: &u64| *id,
                move |id| toast(&toasts, id),
            )
            .column()
            .align_items(Align::End)
            .gap(Size {
                width: 0.0,
                height: 8.0,
            })
            .margin(px(16.0))
        })
}

/// The timer of one toast, which pauses while the pointer is over it.
struct Countdown {
    /// How long the toast still shows once its timer runs again.
    remaining: Cell<Duration>,
    /// When the timer last started.
    started: Cell<Instant>,
    /// Counts the starts and pauses, so a timer an earlier start set closes
    /// nothing.
    wait: Cell<u64>,
}

impl Countdown {
    /// Starts the timer that closes toast `id` of `toasts` once the
    /// remaining time is up.
    fn start(self: &Rc<Self>, toasts: &Toasts, id: u64) {
        let expected = self.wait.get().wrapping_add(1);
        self.wait.set(expected);
        self.started.set(now());
        let (countdown, toasts) = (Rc::clone(self), toasts.clone());
        after(self.remaining.get(), move || {
            if countdown.wait.get() == expected {
                toasts.dismiss(id);
            }
        });
    }

    /// Pauses the timer, keeping the time it has left.
    fn pause(&self) {
        self.wait.set(self.wait.get().wrapping_add(1));
        let ran = now().saturating_duration_since(self.started.get());
        self.remaining.set(self.remaining.get().saturating_sub(ran));
    }
}

/// Builds the toast `id` of `toasts`: its message on a raised surface,
/// closed by a press or once its time is up.
fn toast(toasts: &Toasts, id: u64) -> Element<Panel> {
    let Some(shown) = toasts
        .shown
        .read(|shown| shown.iter().find(|toast| toast.id == id).cloned())
    else {
        return panel();
    };
    let countdown = Rc::new(Countdown {
        remaining: Cell::new(shown.duration),
        started: Cell::new(now()),
        wait: Cell::new(0),
    });
    countdown.start(toasts, id);
    let (hovered, pressed) = (toasts.clone(), toasts.clone());
    row()
        .min_width(px(160.0))
        .max_width(px(360.0))
        .padding_xy(Size {
            width: 14.0,
            height: 10.0,
        })
        .background(ColorToken::SURFACE)
        .border(1.0)
        .border_color(ColorToken::BORDER)
        .corner_radius(RadiusToken::LARGE)
        .on_hover(move |inside| {
            if inside {
                countdown.pause();
            } else {
                countdown.start(&hovered, id);
            }
        })
        .on_press(move || pressed.dismiss(id))
        .child(column().child(label(shown.message)))
}
