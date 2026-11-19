//! The split: two panes side by side or one above the other, and a divider
//! between them that a drag or the keyboard moves.
//!
//! The first pane's flex basis is a share of the split's length on its
//! axis, the ratio, and the second pane grows into the rest. The minimum
//! sizes hold in layout, so they also hold when the window shrinks, and a
//! drag or a key never asks for less. An app binds the ratio to a signal of
//! its own to keep the layout.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::element::{Element, IntoElement, Value};
use smudge_core::geometry::Point;
use smudge_core::input::{Drag, Handled, Key, KeyEvent};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, ValueRange};
use smudge_core::style::{Direction, Length, Overflow, pct, px};
use smudge_core::theme::ColorToken;
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_core::window::Cursor;
use web_time::Instant;

/// The thickness of the divider, in logical pixels.
const DIVIDER: f32 = 6.0;

/// How far one arrow key moves the divider, in logical pixels.
const KEY_STEP: f32 = 16.0;

/// The longest gap between two presses on the divider that counts as a
/// double click. It is the text inputs' value, `DOUBLE_CLICK_TIME` in
/// `smudge-text`'s `chrome.rs`.
const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(400);

/// The farthest apart two presses on the divider can land and still count as
/// a double click, in logical pixels. It is the text inputs' value,
/// `DOUBLE_CLICK_DISTANCE` in `smudge-text`'s `chrome.rs`.
const DOUBLE_CLICK_DISTANCE: f32 = 4.0;

/// The ratio a split starts from when no signal is bound.
const DEFAULT_RATIO: f32 = 0.5;

/// What one split shares between its widget, its divider and its bindings.
struct SplitState {
    /// The axis the panes sit along.
    direction: Direction,
    /// The signal that holds the first pane's share of the split's length,
    /// held in a signal of its own so the pane follows a signal bound after
    /// the split was built.
    ratio: RwSignal<RwSignal<f32>>,
    /// The ratio a double click on the divider goes back to: the one the
    /// split was built with.
    initial: Cell<f32>,
    /// The first pane's minimum length on the axis, in logical pixels.
    min_first: RwSignal<f32>,
    /// The second pane's minimum length on the axis, in logical pixels.
    min_second: RwSignal<f32>,
    /// The split's length on its axis, inside its border and padding, once
    /// it has painted.
    length: Cell<Option<f32>>,
    /// How far the split's content box starts from its rectangle on its
    /// axis, its border and padding, once it has painted. A drag measures
    /// from the rectangle, and the panes from the content box.
    inset: Cell<f32>,
    /// Where on the divider the held press began, along the axis, while the
    /// divider is dragged.
    grab: Cell<Option<f32>>,
    /// When and where the last press on the divider began, for a double
    /// click.
    last_press: Cell<Option<(Instant, Point)>>,
}

impl SplitState {
    /// Returns the first pane's length the split shows: the ratio's share of
    /// `length`, held to the minimums as layout holds it.
    fn first(&self, length: f32) -> f32 {
        self.clamped(
            self.ratio.read(|ratio| ratio.read(|value| *value)) * length,
            length,
        )
    }

    /// Returns `first` held to the minimums within `length`.
    fn clamped(&self, first: f32, length: f32) -> f32 {
        let low = self.min_first.read(|min| *min);
        let high = (length - DIVIDER - self.min_second.read(|min| *min)).max(low);
        first.clamp(low, high)
    }

    /// Moves the divider so the first pane is `first` long, held to the
    /// minimums, and writes the ratio only when it changed, answering
    /// whether it did.
    fn set_first(&self, first: f32) -> bool {
        let Some(length) = self.length.get().filter(|length| *length > 0.0) else {
            return false;
        };
        let ratio = self.clamped(first, length) / length;
        let signal = self.ratio.read(Clone::clone);
        let changed = signal.read(|current| *current != ratio);
        if changed {
            signal.set(ratio);
        }
        changed
    }

    /// Returns the position of `point` along the split's axis.
    fn along(&self, point: Point) -> f32 {
        match self.direction {
            Direction::Row => point.x,
            Direction::Column => point.y,
        }
    }
}

/// A container that lays out two panes and a divider, and reports its own
/// length on its axis when it paints, which the divider's drag and keys
/// measure against.
///
/// It paints like a [`Panel`], so it takes a background and a border through
/// the style builders.
pub struct Split {
    /// What the split shares with its divider and bindings.
    state: Rc<SplitState>,
}

impl Widget for Split {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let (length, inset) = match self.state.direction {
            Direction::Row => (cx.content.width(), cx.content.x0 - cx.rect.x0),
            Direction::Column => (cx.content.height(), cx.content.y0 - cx.rect.y0),
        };
        self.state.length.set(Some(length));
        self.state.inset.set(inset);
        Panel.paint(cx, out);
    }
}

/// Creates a split that shows `first` and `second` along `direction`, side
/// by side for a row and one above the other for a column, with a divider
/// between them.
///
/// Dragging the divider moves it, and so does the keyboard while it has the
/// focus: the arrow keys of its axis by 16 logical pixels, and Home and End
/// to the minimums. A double click on it goes back to the ratio the split was
/// built with. The pointer shows the column or row resize cursor over it,
/// and a screen reader announces it as a splitter named "Resize", with the
/// share of the split its ratio asks the first pane to take as a value from
/// 0 to 100, whole. The minimums can hold the divider away from that share.
/// Give the split a size, or put it where a parent sizes it, and set its
/// ratio and minimums with [`SplitExt`]. The split clips its panes.
///
/// Each pane stretches its content to fill it, overriding the height and the
/// growth the content sets directly, so a split nested in a pane fills it.
/// Wrap content that keeps its own size, such as a toolbar, in a
/// [`column`](crate::column). Content taller than its pane ends at the
/// pane's edge, so a scroll view below a toolbar in a column scrolls instead
/// of running past the pane. Splits nest: either pane can be a split of its
/// own.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_core::style::Direction;
/// use smudge_core::widget::panel;
/// use smudge_widgets::{SplitExt, split};
///
/// let ratio = RwSignal::new(0.3);
/// let editor = split(Direction::Row, panel(), panel())
///     .ratio(ratio)
///     .min_first(160.0)
///     .min_second(240.0);
/// ```
#[track_caller]
pub fn split(
    direction: Direction,
    first: impl IntoElement,
    second: impl IntoElement,
) -> Element<Split> {
    let state = Rc::new(SplitState {
        direction,
        ratio: crate::signal(crate::signal(DEFAULT_RATIO)),
        initial: Cell::new(DEFAULT_RATIO),
        min_first: crate::signal(0.0),
        min_second: crate::signal(0.0),
        length: Cell::new(None),
        inset: Cell::new(0.0),
        grab: Cell::new(None),
        last_press: Cell::new(None),
    });

    let first_pane = {
        let ratio = state.ratio.clone();
        let min = state.min_first.clone();
        let pane = pane()
            .basis(move || pct(ratio.get().get() * 100.0))
            .child(first);
        along(direction, pane, move || px(min.get()))
    };
    let second_pane = {
        let min = state.min_second.clone();
        let pane = pane().basis(px(0.0)).grow(1.0).child(second);
        along(direction, pane, move || px(min.get()))
    };

    let down = Rc::clone(&state);
    let up = Rc::clone(&state);
    let keys = Rc::clone(&state);
    let divider = panel()
        .shrink(0.0)
        .background(ColorToken::BORDER)
        .focused(|patch| patch.background(ColorToken::ACCENT))
        .cursor(match direction {
            Direction::Row => Cursor::ColumnResize,
            Direction::Column => Cursor::RowResize,
        })
        .role(Role::Splitter)
        .accessible_label("Resize")
        .accessible_range({
            let state = Rc::clone(&state);
            move || ValueRange {
                value: (f64::from(state.ratio.get().get()) * 100.0).round(),
                min: 0.0,
                max: 100.0,
            }
        })
        .on_pointer_down(move |at| {
            let now = smudge_core::timer::now();
            let double = down.last_press.get().is_some_and(|(then, there)| {
                now.duration_since(then) <= DOUBLE_CLICK_TIME
                    && (at.x - there.x).powi(2) + (at.y - there.y).powi(2)
                        <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
            });
            if double {
                down.last_press.set(None);
                down.grab.set(None);
                match down.length.get() {
                    Some(length) => {
                        down.set_first(down.initial.get() * length);
                    }
                    None => down.ratio.read(Clone::clone).set(down.initial.get()),
                }
            } else {
                down.last_press.set(Some((now, at)));
                down.grab.set(Some(down.along(at)));
            }
        })
        .on_pointer_up(move |_| up.grab.set(None))
        .on_key(move |event: &KeyEvent| divider_key(&keys, event));
    let divider = match direction {
        Direction::Row => divider.width(px(DIVIDER)),
        Direction::Column => divider.height(px(DIVIDER)),
    };

    let drag = Rc::clone(&state);
    // The split clips its panes, so minimums longer than the split never
    // paint past it.
    let element = Element::new(Split {
        state: Rc::clone(&state),
    })
    .overflow(Overflow::Clip)
    // A drag that began on the divider moves it, measured from the split's
    // own rectangle, which stays put while the divider moves. The split
    // takes the move, so a scroll view around it does not scroll.
    .on_drag(move |step| match (step, drag.grab.get()) {
        (Drag::Moved(at), Some(grab)) => {
            // A press after a drag that moved the divider starts anew rather
            // than counting as the second press of a double click.
            if drag.set_first(drag.along(at) - drag.inset.get() - grab) {
                drag.last_press.set(None);
            }
            Handled::Yes
        }
        (Drag::Moved(_), None) => Handled::No,
        _ => {
            drag.grab.set(None);
            Handled::No
        }
    })
    .child(first_pane)
    .child(divider)
    .child(second_pane);
    match direction {
        Direction::Row => element.row(),
        Direction::Column => element.column(),
    }
}

/// Creates an empty pane, which clips its content and makes it fill the pane:
/// a column stretches its content across, and the content grows down it.
///
/// Content with no minimum height of its own gets a zero one, so it takes
/// the pane's height rather than its content's: a column that holds a
/// scroll view ends at the pane's bottom, and the scroll view scrolls.
fn pane() -> Element<Panel> {
    panel()
        .column()
        .overflow(Overflow::Clip)
        .style_children(|style| {
            style.layout.grow = 1.0;
            if style.layout.min_height == Length::Auto {
                style.layout.min_height = Length::Px(0.0);
            }
        })
}

/// Gives `pane` its minimum length on the split's axis.
fn along(
    direction: Direction,
    pane: Element<Panel>,
    min: impl Fn() -> Length + 'static,
) -> Element<Panel> {
    match direction {
        Direction::Row => pane.min_width(min),
        Direction::Column => pane.min_height(min),
    }
}

/// Moves the divider with the arrow keys of its axis, by one step, and with
/// Home and End to the minimums.
fn divider_key(state: &SplitState, event: &KeyEvent) -> Handled {
    // A key with Ctrl, Alt or Cmd is an app's shortcut, not a divider move.
    let modifiers = event.modifiers;
    if !event.pressed || modifiers.control || modifiers.alt || modifiers.meta {
        return Handled::No;
    }
    let Some(length) = state.length.get() else {
        return Handled::No;
    };
    let first = state.first(length);
    let (back, on) = match state.direction {
        Direction::Row => (Key::ArrowLeft, Key::ArrowRight),
        Direction::Column => (Key::ArrowUp, Key::ArrowDown),
    };
    let target = if event.key == back {
        first - KEY_STEP
    } else if event.key == on {
        first + KEY_STEP
    } else if event.key == Key::Home {
        0.0
    } else if event.key == Key::End {
        length
    } else {
        return Handled::No;
    };
    state.set_first(target);
    Handled::Yes
}

/// Sets the ratio and the minimums of a [`split`].
///
/// Import the trait to use them:
///
/// ```
/// use smudge_core::style::Direction;
/// use smudge_core::widget::panel;
/// use smudge_widgets::{SplitExt, split};
///
/// let sidebar = split(Direction::Row, panel(), panel()).min_first(120.0);
/// ```
pub trait SplitExt: Sized {
    /// Binds the first pane's share of the split's length to `ratio`, a
    /// number from 0 to 1.
    ///
    /// The divider writes the signal as it moves, so an app can keep the
    /// layout by keeping the signal, and writing the signal moves the
    /// divider. A double click on the divider goes back to the value the
    /// signal holds when this is called.
    fn ratio(self, ratio: RwSignal<f32>) -> Self;

    /// Keeps the first pane at least `length` logical pixels long on the
    /// split's axis.
    fn min_first(self, length: f32) -> Self;

    /// Keeps the second pane at least `length` logical pixels long on the
    /// split's axis.
    fn min_second(self, length: f32) -> Self;
}

/// The key of a split's ratio binding.
struct RatioKey;

/// The key of a split's first minimum binding.
struct MinFirstKey;

/// The key of a split's second minimum binding.
struct MinSecondKey;

impl SplitExt for Element<Split> {
    fn ratio(self, ratio: RwSignal<f32>) -> Self {
        self.bind(RatioKey, Value::Static(ratio), use_ratio)
    }

    fn min_first(self, length: f32) -> Self {
        self.bind(MinFirstKey, Value::Static(length), set_min_first)
    }

    fn min_second(self, length: f32) -> Self {
        self.bind(MinSecondKey, Value::Static(length), set_min_second)
    }
}

/// Makes the split read and write `ratio`, and go back to its present value
/// on a double click.
fn use_ratio(split: &mut Split, ratio: RwSignal<f32>) -> Dirty {
    split.state.initial.set(ratio.read(|value| *value));
    split.state.ratio.set(ratio);
    Dirty::Clean
}

/// Sets the first pane's minimum length.
fn set_min_first(split: &mut Split, length: f32) -> Dirty {
    split.state.min_first.set(length.max(0.0));
    Dirty::Clean
}

/// Sets the second pane's minimum length.
fn set_min_second(split: &mut Split, length: f32) -> Dirty {
    split.state.min_second.set(length.max(0.0));
    Dirty::Clean
}
