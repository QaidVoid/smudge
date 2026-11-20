//! The scroll view: a clipped viewport whose content moves with the wheel.
//!
//! The view is a [`ScrollView`] panel that clips its content, handles wheel
//! events while it is hovered, and clamps its offset to what the content
//! needs. The offset is the view's scroll offset, which the core applies after
//! layout, so scrolling moves the content without a layout pass. The thumb
//! moves the same way, through the scroll offset of a track it hangs from.
//! Through [`ScrollExt::follow_end`] the view can also pin itself to the end
//! of its content while a signal reads `true`, and release itself when a
//! wheel step, a thumb drag or a page leaves the end. When the keyboard focus
//! moves to a node inside the view by any means but a pointer press, the view
//! scrolls to show that node, as [`Element::on_reveal`] describes.
//!
//! The view learns its own size and its content's size when the two paint,
//! and writes them to signals its bindings read. A size-driven change,
//! such as a thumb that follows a resize, therefore lands one frame after the
//! resize that caused it, which keeps layout out of the paint pass. The
//! follow pin is the one exception: the core applies it after each layout
//! pass with the extent that pass just measured, so a frame that grows the
//! content never shows the end out of view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use smudge_core::element::{Element, IntoElement, Value};
use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Drag, Handled};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, Semantics};
use smudge_core::style::{Align, Direction, Edges, Length, Overflow, pct, px};
use smudge_core::theme::ColorToken;
use smudge_core::timer::{TimerHandle, every};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};

/// The width of the scrollbar thumb, in logical pixels.
const THUMB_WIDTH: f32 = 4.0;

/// The gap between the thumb and the right edge of the viewport, in logical
/// pixels.
const THUMB_RIGHT_INSET: f32 = 2.0;

/// The shortest the thumb is ever drawn, in logical pixels, so a long content
/// keeps a thumb that can still be seen.
const MIN_THUMB_LENGTH: f32 = 24.0;

/// How often a drag held past the view's edge scrolls it.
const AUTO_SCROLL_PERIOD: Duration = Duration::from_millis(16);

/// How far one auto-scroll step goes for each logical pixel the pointer is
/// past the edge.
const AUTO_SCROLL_RATE: f32 = 0.2;

/// How far past the edge the pointer still speeds the auto-scroll up, in
/// logical pixels.
const AUTO_SCROLL_REACH: f32 = 100.0;

/// How far from the end of the content a wheel step down may land, in
/// logical pixels, and still engage following.
const FOLLOW_SLACK: f32 = 4.0;

/// Which way a scroll view scrolls.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ScrollAxis {
    /// Up and down. The content is as wide as the view and as tall as it
    /// needs.
    #[default]
    Vertical,
    /// Sideways. The content is as tall as the view and as wide as it
    /// needs, and at least as wide as the view.
    Horizontal,
    /// Both ways. The content is as wide and as tall as it needs, and at
    /// least as wide as the view.
    Both,
}

impl ScrollAxis {
    /// Returns whether the view scrolls up and down.
    fn vertical(self) -> bool {
        matches!(self, ScrollAxis::Vertical | ScrollAxis::Both)
    }

    /// Returns whether the view scrolls sideways.
    fn horizontal(self) -> bool {
        matches!(self, ScrollAxis::Horizontal | ScrollAxis::Both)
    }
}

/// The sizes a scroll view measures when it paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    /// The height of the view's content box, the part of the content that
    /// fits on screen at once.
    visible: f32,
    /// The height inside the view's border, which the thumb travels along.
    track: f32,
    /// The height of the view's whole rectangle, whose edges a drag
    /// auto-scrolls past.
    outer: f32,
    /// The width of the view's content box.
    visible_width: f32,
    /// The width inside the view's border.
    track_width: f32,
    /// The width of the view's whole rectangle.
    outer_width: f32,
}

/// The reactive state one scroll view shares between its widgets, its wheel
/// handler and its bindings.
struct ScrollChrome {
    /// Which way the view scrolls, which [`ScrollExt::axis`] sets while the
    /// view is built.
    axis: Cell<ScrollAxis>,
    /// Whether the view is a virtual list's, which scrolls up and down only,
    /// whatever axis it is given.
    list: bool,
    /// The signal whose value is the vertical scroll offset. It starts as one
    /// the view created itself, and [`ScrollExt::offset`] swaps in one of the
    /// user's while the view is built.
    offset: RefCell<RwSignal<f32>>,
    /// The signal whose value is the horizontal scroll offset, which
    /// [`ScrollExt::horizontal_offset`] swaps the same way.
    horizontal: RefCell<RwSignal<f32>>,
    /// The signal that reports and sets whether the view follows the end of
    /// its content, when [`ScrollExt::follow_end`] set one. It sits in a
    /// signal of its own, so a list that lays its content out differently
    /// while the view follows learns of a follow signal installed after it
    /// was built.
    following: RwSignal<Option<RwSignal<bool>>>,
    /// The height of the content, once the content has painted.
    content_height: RwSignal<Option<f32>>,
    /// The width of the content, once the content has painted.
    content_width: RwSignal<Option<f32>>,
    /// The height the content takes in the coming layout, when the view's
    /// owner knows it before the content paints. It stands in for
    /// `content_height`, which lags it by a frame.
    content_extent: Option<RwSignal<f32>>,
    /// The sizes of the view, once the view has painted.
    viewport: RwSignal<Option<Viewport>>,
    /// The visible height of the view, once the view has painted, reported to
    /// a virtual list that computes its rows from it.
    visible_height: RwSignal<Option<f32>>,
}

impl ScrollChrome {
    /// Returns the current vertical offset signal, whichever one
    /// [`ScrollExt::offset`] installed.
    fn signal(&self) -> RwSignal<f32> {
        self.offset.borrow().clone()
    }

    /// Returns the current horizontal offset signal, whichever one
    /// [`ScrollExt::horizontal_offset`] installed.
    fn horizontal_signal(&self) -> RwSignal<f32> {
        self.horizontal.borrow().clone()
    }

    /// Returns the furthest the view can scroll sideways, zero for a view
    /// that does not scroll sideways, or `None` before both the view and its
    /// content have painted. Reading it tracks both widths.
    fn max_offset_x(&self) -> Option<f32> {
        if !self.axis.get().horizontal() {
            return Some(0.0);
        }
        let content = self.content_width.get()?;
        let viewport = self.viewport.get()?;
        Some((content - viewport.visible_width).max(0.0))
    }

    /// Returns the furthest the view can scroll, or `None` before both the
    /// view and its content have painted. Reading it tracks both heights.
    fn max_offset(&self) -> Option<f32> {
        if !self.axis.get().vertical() {
            return Some(0.0);
        }
        let content = self.content()?;
        let viewport = self.viewport.get()?;
        Some((content - viewport.visible).max(0.0))
    }

    /// Returns the height of the content: the extent the owner gave, or the
    /// painted height, which is `None` before the content has painted.
    /// Reading it tracks the signal it comes from.
    fn content(&self) -> Option<f32> {
        match &self.content_extent {
            Some(extent) => Some(finite(extent.get()).max(0.0)),
            None => self.content_height.get(),
        }
    }

    /// Returns the follow signal, when [`ScrollExt::follow_end`] set one.
    /// Reading it tracks the slot the signal sits in.
    fn following(&self) -> Option<RwSignal<bool>> {
        self.following.get()
    }
}

/// A panel that clips its content and scrolls it with the wheel.
///
/// It paints like a [`Panel`], so it takes a background and a border through
/// the style builders. Its paint also reports the view's sizes to its
/// bindings, which is how the offsets learn their ranges and the thumbs
/// their tracks.
pub struct ScrollView {
    /// The state shared with the wheel handler and the bindings.
    chrome: Rc<ScrollChrome>,
}

impl Widget for ScrollView {
    /// Reports the view as a scrollable region.
    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics::new(Role::ScrollView))
    }

    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let border = cx.style.layout.border_width;
        let visible = finite(cx.content.height());
        report(
            &self.chrome.viewport,
            Viewport {
                visible,
                track: finite(cx.rect.height() - border.top - border.bottom).max(0.0),
                outer: finite(cx.rect.height()),
                visible_width: finite(cx.content.width()),
                track_width: finite(cx.rect.width() - border.left - border.right).max(0.0),
                outer_width: finite(cx.rect.width()),
            },
        );
        report(&self.chrome.visible_height, visible);
        Panel.paint(cx, out);
    }
}

/// The wrapper around the user's content, whose paint reports the content
/// height.
///
/// It paints nothing: the content paints itself, one node below.
struct ScrollContent {
    /// The state shared with the wheel handler and the bindings.
    chrome: Rc<ScrollChrome>,
}

impl Widget for ScrollContent {
    fn paint(&self, cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {
        report(&self.chrome.content_height, finite(cx.rect.height()));
        report(&self.chrome.content_width, finite(cx.rect.width()));
    }
}

/// Returns `value`, or zero when it is not a number.
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// Writes a measured value into a geometry signal, when it changed.
///
/// An equal write would still wake every binding that reads the signal, so
/// the comparison is what keeps an idle scroll view from asking for frames.
fn report<T: PartialEq + 'static>(signal: &RwSignal<Option<T>>, value: T) {
    let value = Some(value);
    if signal.read(|current| current != &value) {
        signal.set(value);
    }
}

/// Clamps a desired offset to the range `0..=max`.
///
/// A desired offset that is not a number counts as zero, so a NaN written to
/// the offset signal lands at the top instead of moving every rectangle out of
/// the arithmetic.
fn clamp_offset(desired: f32, max: f32) -> f32 {
    finite(desired).clamp(0.0, max)
}

/// Creates a view that clips `content` and scrolls it with the wheel.
///
/// The view is a panel one viewport tall, so give it a height, either
/// directly or through a parent that sizes it. The content is laid out at its
/// full height, however tall that is. A wheel scroll moves the content up
/// while the pointer is over the view, clamped so the content's bottom edge
/// never rises above the bottom of the view's content box, and a thin thumb
/// at the right edge shows the position while the content overflows.
/// Dragging the thumb scrolls the view by the content's share of each pixel
/// it moves, held to the content's ends, and a press on the track beside the
/// thumb scrolls one page, the height the view shows, toward the press.
/// [`ScrollExt::axis`] makes the view scroll sideways or both ways instead.
/// While the content overflows, the scrollbar takes the presses along the
/// view's right edge, eight logical pixels wide, over the content there, and
/// a press on it leaves the keyboard focus where it is.
///
/// When the keyboard focus moves to a node inside the view that is out of
/// view, the view scrolls to show it, for every move that
/// [`Element::on_reveal`] lists: the least it takes to bring the node to its
/// nearest edge, or to align its top when the node is taller than the view.
/// Nested views each scroll. A pointer press that focuses a node scrolls
/// nothing.
///
/// The offset can be controlled from outside through a signal, with
/// [`ScrollExt::offset`]:
///
/// ```
/// use smudge_core::geometry::Color;
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_widgets::{ScrollExt, column, scroll};
///
/// let offset = RwSignal::new(0.0);
/// let view = scroll(
///     column().child(panel().height(px(400.0)).background(Color::WHITE)),
/// )
/// .offset(offset.clone())
/// .height(px(120.0));
///
/// // Jump towards the bottom: the view clamps to what the content needs.
/// offset.set(f32::MAX);
/// ```
///
/// Once the view and its content have painted, it writes the clamped offset
/// back to the signal, so the signal always reads the position on screen.
/// Before that, it applies any offset that is not negative as it is, which
/// lets a view mount at a remembered position.
#[track_caller]
pub fn scroll(content: impl IntoElement) -> Element<ScrollView> {
    scroll_with_visible_height(
        content,
        crate::signal(None),
        None,
        crate::signal(None),
        false,
    )
}

/// Creates a view like [`scroll`] that reports the visible height of its
/// content box to `visible_height` once it has painted, and again whenever
/// that height changes.
///
/// The height is a measurement only the view has, because it follows the
/// style and the layout around it, so the virtual list reads it from this
/// signal instead of asking for a fixed viewport.
///
/// A list that sizes its content itself passes that size as
/// `content_extent`, and the view clamps its offset with it instead of the
/// content height it painted last frame. Otherwise an offset the list moves
/// in the frame that grows the content would be clamped to the old end.
///
/// The view writes the signal [`ScrollExt::follow_end`] installs to
/// `following`, so a list that reads the slot learns whether the view
/// follows its end. A view built for a virtual `list` scrolls up and down
/// only, whatever [`ScrollExt::axis`] it is given.
#[track_caller]
pub(crate) fn scroll_with_visible_height(
    content: impl IntoElement,
    visible_height: RwSignal<Option<f32>>,
    content_extent: Option<RwSignal<f32>>,
    following: RwSignal<Option<RwSignal<bool>>>,
    list: bool,
) -> Element<ScrollView> {
    let content_height = crate::signal(None);
    let content_width = crate::signal(None);
    let viewport = crate::signal(None);
    // A remount after a hot patch can hand these the measurements of the
    // old build's layout, which say nothing of the new one, and would clamp
    // a carried offset before the new view measured itself.
    if content_height.read(Option::is_some) {
        content_height.set(None);
    }
    if content_width.read(Option::is_some) {
        content_width.set(None);
    }
    if viewport.read(Option::is_some) {
        viewport.set(None);
    }
    if visible_height.read(Option::is_some) {
        visible_height.set(None);
    }
    // A remount after a hot patch can hand the slot the old build's follow
    // signal, which the new build may no longer give the view.
    if following.read(Option::is_some) {
        following.set(None);
    }
    let chrome = Rc::new(ScrollChrome {
        axis: Cell::new(ScrollAxis::Vertical),
        list,
        offset: RefCell::new(crate::signal(0.0)),
        horizontal: RefCell::new(crate::signal(0.0)),
        following,
        content_height,
        content_width,
        content_extent,
        viewport,
        visible_height,
    });

    let wheel_chrome = Rc::clone(&chrome);
    let offset_chrome = Rc::clone(&chrome);
    let content_chrome = Rc::clone(&chrome);
    let track_chrome = Rc::clone(&chrome);
    let reveal_chrome = Rc::clone(&chrome);
    let drag_chrome = Rc::clone(&chrome);
    let sizing_chrome = Rc::clone(&chrome);
    let (direction_chrome, align_chrome) = (Rc::clone(&chrome), Rc::clone(&chrome));
    let auto = Rc::new(AutoScroll::default());

    Element::new(ScrollView {
        chrome: Rc::clone(&chrome),
    })
    // A view that scrolls sideways lays its content out in a row, at the
    // content's own width. One that scrolls both ways keeps the content's
    // own height too, rather than stretching it to the view's and spilling
    // the rest past it.
    .direction(move || {
        if direction_chrome.axis.get().horizontal() {
            Direction::Row
        } else {
            Direction::Column
        }
    })
    .align_items(move || {
        if align_chrome.axis.get() == ScrollAxis::Both {
            Align::Start
        } else {
            Align::Stretch
        }
    })
    .overflow(Overflow::Scroll)
    // The view takes every wheel event over it, even one it cannot use, so
    // a page around a view at its end does not scroll in its place.
    .on_wheel(move |delta| {
        let axis = wheel_chrome.axis.get();
        if axis.vertical() {
            scroll_by(&wheel_chrome, finite(delta.y), FOLLOW_SLACK);
        }
        if axis.horizontal() {
            scroll_x_by(&wheel_chrome, finite(delta.x));
        }
        Handled::Yes
    })
    .scroll_offset(move || {
        let axis = offset_chrome.axis.get();
        let x = if axis.horizontal() {
            let signal = offset_chrome.horizontal_signal();
            match offset_chrome.max_offset_x() {
                Some(max) => {
                    let desired = signal.get();
                    let clamped = clamp_offset(desired, max);
                    if clamped != desired {
                        signal.set(clamped);
                    }
                    clamped
                }
                None => finite(signal.get()).max(0.0),
            }
        } else {
            0.0
        };
        let signal = offset_chrome.signal();
        let following = offset_chrome.following().is_some_and(|signal| signal.get());
        let applied = match offset_chrome.max_offset() {
            // While the view follows, the end is the offset, whatever the
            // signal held: the core pins the node with the heights the last
            // layout pass wrote, which the painted ones the signal
            // round-trips through lag by a frame. The read is tracked, so a
            // write from the application is overwritten in the next flush.
            Some(max) if following => {
                if signal.get() != max {
                    signal.set(max);
                }
                max
            }
            Some(max) => {
                let desired = signal.get();
                let clamped = clamp_offset(desired, max);
                if clamped != desired {
                    signal.set(clamped);
                }
                clamped
            }
            None => finite(signal.get()).max(0.0),
        };
        Point { x, y: applied }
    })
    // A drag that began inside the view and is held past its top or bottom
    // edge scrolls it, faster the farther out the pointer is, until the
    // pointer comes back or the press ends. The content that scrolls under
    // the pointer hears the drag again, so a selection follows it. The view
    // takes the move only while it can still scroll that way, so a view
    // around it takes over once it reaches its end.
    .on_drag(move |step| match step {
        Drag::Moved(at) => {
            let Some(height) = drag_chrome
                .viewport
                .read(|viewport| viewport.map(|v| v.outer))
            else {
                return Handled::No;
            };
            let past = if at.y < 0.0 {
                at.y
            } else if at.y > height {
                at.y - height
            } else {
                0.0
            };
            if past == 0.0 || !can_scroll(&drag_chrome, past) {
                auto.stop();
                return Handled::No;
            }
            auto.past
                .set(past.clamp(-AUTO_SCROLL_REACH, AUTO_SCROLL_REACH));
            if auto.timer.borrow().is_none() {
                auto.last.set(Some(smudge_core::timer::now()));
                let chrome = Rc::clone(&drag_chrome);
                // The timer holds the state weakly: the state holds the
                // timer, and a view unmounted mid-drag drops both.
                let state = Rc::downgrade(&auto);
                let timer = every(AUTO_SCROLL_PERIOD, move || {
                    let Some(state) = state.upgrade() else {
                        return;
                    };
                    let now = smudge_core::timer::now();
                    let elapsed = state
                        .last
                        .replace(Some(now))
                        .map_or(AUTO_SCROLL_PERIOD, |last| now.duration_since(last))
                        .min(AUTO_SCROLL_PERIOD * 6);
                    let ticks = elapsed.as_secs_f32() / AUTO_SCROLL_PERIOD.as_secs_f32();
                    auto_scroll(&chrome, state.past.get() * AUTO_SCROLL_RATE * ticks);
                });
                *auto.timer.borrow_mut() = Some(timer);
            }
            Handled::Yes
        }
        _ => {
            auto.stop();
            Handled::No
        }
    })
    // The core already moved the view to show the focused node. The signal
    // takes the offset, so the binding above applies the same one, and a
    // view pinned to its end lets go of it, because the core only moves a
    // pinned view up from the end.
    .on_reveal(move |revealed| {
        let signal = reveal_chrome.signal();
        if signal.read(|offset| *offset != revealed.y) {
            signal.set(revealed.y);
        }
        if let Some(following) = reveal_chrome.following()
            && following.read(|active| *active)
        {
            following.set(false);
        }
    })
    .child(
        Element::new(ScrollContent {
            chrome: content_chrome,
        })
        .column()
        .shrink(0.0)
        // A view that scrolls sideways lays its content out in a row, at
        // the content's own width, and at least the view's, so narrow
        // content still fills it.
        .min_width(move || {
            if sizing_chrome.axis.get().horizontal() {
                pct(100.0)
            } else {
                Length::Auto
            }
        })
        .child(content),
    )
    .child(thumb(track_chrome))
}

/// The auto-scroll of one scroll view while a drag is held past its edge.
#[derive(Default)]
struct AutoScroll {
    /// How far past the edge the pointer is, negative above the top, capped
    /// at the reach.
    past: Cell<f32>,
    /// When the last step scrolled, so a step covers the time since then
    /// whatever the frame rate.
    last: Cell<Option<web_time::Instant>>,
    /// The timer that scrolls, while the pointer is past an edge.
    timer: RefCell<Option<TimerHandle>>,
}

impl AutoScroll {
    /// Stops scrolling.
    fn stop(&self) {
        self.timer.borrow_mut().take();
        self.past.set(0.0);
        self.last.set(None);
    }
}

/// Scrolls the view by `step` from where it is, held to its range, as a
/// wheel step, a page or a thumb drag does, and engages following when a
/// step down lands within `slack` of the end. Does nothing before the view
/// and its content have painted.
fn scroll_by(chrome: &ScrollChrome, step: f32, slack: f32) {
    let Some(max) = chrome.max_offset() else {
        return;
    };
    let follow_signal = chrome.following();
    // The signal, not the offset the binding last applied, is where a step
    // starts: several steps can arrive before the next flush, and each must
    // add to the one before it. While the view follows it sits at the end,
    // so the step starts there even when the signal has not caught up with
    // the pin yet.
    let following = follow_signal
        .as_ref()
        .is_some_and(|signal| signal.read(|active| *active));
    let signal = chrome.signal();
    let current = if following {
        max
    } else {
        clamp_offset(signal.read(|offset| *offset), max)
    };
    let next = clamp_offset(current + step, max);
    if next != current {
        signal.set(next);
    }
    // Any step that moves the view up releases the pin, so small steps add
    // up instead of each one snapping back. The slack applies only to a step
    // down, which engages the pin when it lands near the end.
    if let Some(signal) = &follow_signal {
        if following && next < current {
            signal.set(false);
        } else if !following && step > 0.0 && next >= max - slack {
            signal.set(true);
        }
    }
}

/// Scrolls the view sideways by `step` from where it is, held to its range.
/// Does nothing before the view and its content have painted.
fn scroll_x_by(chrome: &ScrollChrome, step: f32) {
    let Some(max) = chrome.max_offset_x() else {
        return;
    };
    let signal = chrome.horizontal_signal();
    let current = clamp_offset(signal.read(|offset| *offset), max);
    let next = clamp_offset(current + step, max);
    if next != current {
        signal.set(next);
    }
}

/// Returns where the view is scrolled to, the end while it follows, or
/// `None` before the view and its content have painted.
fn current_offset(chrome: &ScrollChrome) -> Option<f32> {
    let max = chrome.max_offset()?;
    let pinned = chrome
        .following()
        .is_some_and(|signal| signal.read(|on| *on));
    Some(if pinned {
        max
    } else {
        clamp_offset(chrome.signal().read(|offset| *offset), max)
    })
}

/// Returns whether the view of `chrome` can scroll toward `past`: up while
/// it is below its top, and down while it is above its end and not pinned
/// to it.
fn can_scroll(chrome: &ScrollChrome, past: f32) -> bool {
    let (Some(max), Some(current)) = (chrome.max_offset(), current_offset(chrome)) else {
        return false;
    };
    if past < 0.0 {
        current > 0.0
    } else {
        current < max
    }
}

/// Scrolls the view of `chrome` by `by` logical pixels, clamped to its
/// content, and lets go of the end when the scroll goes up from it. A drag
/// that reaches the end does not pin the view to it again, unlike a wheel
/// step, so a selection that stops there stays where the user left it.
fn auto_scroll(chrome: &ScrollChrome, by: f32) {
    let Some(max) = chrome.max_offset() else {
        return;
    };
    let following = chrome.following();
    let pinned = following
        .as_ref()
        .is_some_and(|signal| signal.read(|on| *on));
    if pinned && by >= 0.0 {
        return;
    }
    let signal = chrome.signal();
    let current = if pinned {
        max
    } else {
        clamp_offset(signal.read(|offset| *offset), max)
    };
    let next = clamp_offset(current + by, max);
    if next != current {
        signal.set(next);
    }
    if pinned && let Some(following) = following {
        following.set(false);
    }
}

/// Controls the offset of a scroll view through a signal.
///
/// Import the trait to use it:
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_core::widget::panel;
/// use smudge_widgets::{ScrollExt, column, scroll};
///
/// let offset = RwSignal::new(0.0);
/// let view = scroll(column().child(panel())).offset(offset);
/// ```
pub trait ScrollExt: Sized {
    /// Scrolls with `signal`: the view reads the signal for its offset, so a
    /// write from anywhere moves the content, clamped to the content's size.
    ///
    /// The wheel, a thumb drag and a press on the track write the same
    /// signal, and so does a focus move that scrolls the view to show the
    /// focused node. The view writes the clamped offset back to it, so the
    /// view, the thumb and the signal stay in step. A focus move scrolls the
    /// content in the frame it happens and moves the thumb in the frame after
    /// it.
    fn offset(self, signal: RwSignal<f32>) -> Self;

    /// Keeps the view at the end of its content while `signal` reads `true`.
    ///
    /// While it follows, every frame that grows the content leaves the view
    /// pinned to the end in the same frame, so a line appended to a log is
    /// already in view. The core applies that pin after each layout pass,
    /// where the painted heights the offset signal round-trips through are
    /// one frame old. The view writes the painted end back to the offset
    /// signal, so a signal set with [`ScrollExt::offset`] tracks the position
    /// on screen and catches up one frame after a frame that grew the
    /// content.
    ///
    /// The signal is also where the view reports where the user is: any
    /// wheel step, thumb drag or page that moves the view up from the end
    /// writes `false`, and so does a focus move that scrolls the view up to
    /// show the focused node, while a wheel step down that lands within four
    /// logical pixels of the end, or a drag or a page that reaches it,
    /// writes `true`. Writing `true` from the application jumps to the end.
    /// A view that starts following with content shorter than itself stays
    /// at offset zero until the content grows past it.
    ///
    /// Give the signal to one view. Two views that share it pin and release
    /// together, and a wheel step over either one decides for both.
    ///
    /// While it follows, the offset signal is the view's to write: a write
    /// from the application is overwritten, and the way to move the view is
    /// to write `false` first.
    ///
    /// ```
    /// use smudge_core::geometry::Color;
    /// use smudge_core::reactive::{RwSignal, prelude::*};
    /// use smudge_core::style::px;
    /// use smudge_core::widget::panel;
    /// use smudge_widgets::{ScrollExt, column, scroll};
    ///
    /// let following = RwSignal::new(true);
    /// let view = scroll(
    ///     column().child(panel().height(px(400.0)).background(Color::WHITE)),
    /// )
    /// .follow_end(following.clone())
    /// .height(px(120.0));
    ///
    /// // The view shows the end of the content from the first frame on,
    /// // and stays there while the content grows.
    /// following.set(false);
    /// ```
    fn follow_end(self, signal: RwSignal<bool>) -> Self;

    /// Sets which way the view scrolls: up and down, the default, sideways,
    /// or both.
    ///
    /// A view that scrolls sideways lays its content out at the content's
    /// own width, at least as wide as the view, and clips the rest, as a
    /// vertical view does with the height. A view that scrolls only one way
    /// keeps the other offset at zero. Following the end, with
    /// [`ScrollExt::follow_end`], stays on the vertical axis. A virtual list
    /// scrolls up and down only, whatever axis it is given.
    ///
    /// ```
    /// use smudge_core::style::px;
    /// use smudge_text::label;
    /// use smudge_widgets::{ScrollAxis, ScrollExt, scroll};
    ///
    /// let line = scroll(label("A line far wider than the view it sits in"))
    ///     .axis(ScrollAxis::Horizontal)
    ///     .width(px(120.0))
    ///     .height(px(24.0));
    /// ```
    fn axis(self, axis: ScrollAxis) -> Self;

    /// Scrolls sideways with `signal`, as [`ScrollExt::offset`] does up and
    /// down: the view reads the signal for its horizontal offset, clamped to
    /// the content's width, and writes the clamped offset back to it.
    ///
    /// It has an effect on a view whose [`ScrollExt::axis`] scrolls
    /// sideways, and none on a virtual list, which scrolls up and down only.
    fn horizontal_offset(self, signal: RwSignal<f32>) -> Self;
}

/// The key of a view's offset signal binding.
struct OffsetKey;

/// The key of a view's follow signal binding.
struct FollowKey;

/// The key of a view's axis binding.
struct AxisKey;

/// The key of a view's horizontal offset signal binding.
struct HorizontalKey;

impl ScrollExt for Element<ScrollView> {
    fn offset(self, signal: RwSignal<f32>) -> Self {
        self.bind(OffsetKey, Value::Static(signal), use_offset_signal)
    }

    fn follow_end(self, signal: RwSignal<bool>) -> Self {
        // `follow` is the core builder, named apart from this trait method so
        // a call to `follow_end` on a view element reaches this one, which
        // pairs the node's pin with the chrome's wheel handling.
        self.follow(signal.clone())
            .bind(FollowKey, Value::Static(signal), use_follow_signal)
    }

    fn axis(self, axis: ScrollAxis) -> Self {
        self.bind(AxisKey, Value::Static(axis), |view, axis| {
            if !view.chrome.list {
                view.chrome.axis.set(axis);
            }
            Dirty::Clean
        })
    }

    fn horizontal_offset(self, signal: RwSignal<f32>) -> Self {
        self.bind(HorizontalKey, Value::Static(signal), |view, signal| {
            *view.chrome.horizontal.borrow_mut() = signal;
            Dirty::Clean
        })
    }
}

/// Makes a scroll view read and write `signal` for its offset.
///
/// This runs while the view is built, before any of its bindings has read the
/// signal it replaces.
fn use_offset_signal(view: &mut ScrollView, signal: RwSignal<f32>) -> Dirty {
    *view.chrome.offset.borrow_mut() = signal;
    Dirty::Clean
}

/// Makes a scroll view turn `signal` with its wheel and pin its offset while
/// it reads `true`.
///
/// This runs while the view is built, before any wheel event can arrive.
fn use_follow_signal(view: &mut ScrollView, signal: RwSignal<bool>) -> Dirty {
    view.chrome.following.set(Some(signal));
    Dirty::Clean
}

/// Builds the scrollbar of one scroll view: a track along the right edge, and
/// the thumb in it.
///
/// The track is absolutely positioned along the view's right edge, inside
/// its border, as wide as the thumb and its inset on both sides, and the
/// thumb is its child in the flow. The track's scroll offset moves the thumb
/// down to its position, so the thumb follows a scroll without a layout
/// pass. The thumb's length is the visible share of the content, at least
/// [`MIN_THUMB_LENGTH`], and the scrollbar hides while the content fits.
///
/// Dragging the thumb scrolls the view by the content's share of each pixel
/// the thumb moves, and a press on the track beside the thumb scrolls one
/// page toward the press. Either way the view moves as a wheel step moves
/// it, so the end of the content engages following again.
fn thumb(chrome: Rc<ScrollChrome>) -> Element<Panel> {
    // Where a drag of the thumb began, along the track, and where the
    // thumb's top was then, before rounding. Each move puts the top as far
    // from there as the pointer has come and maps it through the geometry
    // of the moment, so the pointer keeps its place on the thumb even while
    // the content grows under the drag.
    let grab: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));
    let hidden_chrome = Rc::clone(&chrome);
    let length_chrome = Rc::clone(&chrome);
    let top_chrome = Rc::clone(&chrome);
    let press_chrome = Rc::clone(&chrome);
    let grab_chrome = Rc::clone(&chrome);
    let drag_chrome = chrome;
    let press_grab = Rc::clone(&grab);
    let thumb_grab = Rc::clone(&grab);
    panel()
        .absolute()
        .inset(Edges {
            top: Length::px(0.0),
            right: Length::px(0.0),
            bottom: Length::px(0.0),
            left: Length::Auto,
        })
        .width(px(THUMB_WIDTH + 2.0 * THUMB_RIGHT_INSET))
        .padding_xy(Size {
            width: THUMB_RIGHT_INSET,
            height: 0.0,
        })
        .column()
        .keeps_focus()
        .hidden(move || thumb_track(&hidden_chrome).is_none())
        .scroll_offset(move || Point {
            x: 0.0,
            y: -thumb_top(&top_chrome),
        })
        // A press on the thumb is the thumb's own. A press the track hears
        // beside the thumb, in its padding, grabs the thumb too, and one
        // above or below it pages.
        .on_pointer_down(move |at| {
            let (Some(track), Some(exact)) = (thumb_track(&press_chrome), exact_top(&press_chrome))
            else {
                return;
            };
            let top = thumb_top(&press_chrome);
            if at.y >= top && at.y < top + track.length {
                press_grab.set(Some((at.y, exact)));
                return;
            }
            let Some(page) = press_chrome
                .viewport
                .read(|viewport| viewport.map(|v| v.visible))
            else {
                return;
            };
            let step = if at.y < top { -page } else { page };
            scroll_by(&press_chrome, step, 0.0);
        })
        // A drag that began on the thumb moves it, measured from the track,
        // which stays put while the thumb moves. Every drag the track hears
        // began on it, so it takes each move, and the view does not scroll
        // the drag past its edge.
        .on_drag(move |step| match (step, grab.get()) {
            (Drag::Moved(at), Some((start, exact))) => {
                let (Some(track), Some(current)) =
                    (thumb_track(&drag_chrome), current_offset(&drag_chrome))
                else {
                    return Handled::Yes;
                };
                let travel = track.track - track.length;
                if travel > 0.0 {
                    let wanted = (exact + at.y - start) / travel * track.max_offset;
                    scroll_by(&drag_chrome, wanted - current, 0.0);
                }
                Handled::Yes
            }
            (Drag::Moved(_), None) => Handled::Yes,
            _ => {
                grab.set(None);
                Handled::No
            }
        })
        .child(
            panel()
                .width(px(THUMB_WIDTH))
                .height(move || Length::px(thumb_length(&length_chrome)))
                .shrink(0.0)
                .corner_radius(THUMB_WIDTH / 2.0)
                .background(ColorToken::BORDER)
                .hover(|patch| patch.background(ColorToken::TEXT_MUTED))
                .pressed(|patch| patch.background(ColorToken::TEXT))
                .keeps_focus()
                .on_pointer_down(move |at| {
                    if let Some(exact) = exact_top(&grab_chrome) {
                        thumb_grab.set(Some((thumb_top(&grab_chrome) + at.y, exact)));
                    }
                }),
        )
}

/// The geometry the thumb is drawn with.
struct Track {
    /// The length of the thumb.
    length: f32,
    /// The height the thumb travels along.
    track: f32,
    /// The furthest the view can scroll.
    max_offset: f32,
}

/// Returns the thumb's geometry, or `None` while the content fits or before
/// the view has painted, when the thumb is hidden.
fn thumb_track(chrome: &ScrollChrome) -> Option<Track> {
    let max_offset = chrome.max_offset()?;
    let content = chrome.content()?;
    let viewport = chrome.viewport.get()?;
    if max_offset <= 0.0 || viewport.track <= 0.0 {
        return None;
    }
    // The layout rounds the length to whole pixels, so the top is computed
    // from the length the thumb is actually drawn with.
    let length = (viewport.track * viewport.visible / content)
        .clamp(MIN_THUMB_LENGTH.min(viewport.track), viewport.track)
        .round();
    Some(Track {
        length,
        track: viewport.track,
        max_offset,
    })
}

/// Returns the thumb's distance from the top of the track for where the view
/// is scrolled to, before the rounding it is drawn with, or `None` while the
/// thumb is hidden.
fn exact_top(chrome: &ScrollChrome) -> Option<f32> {
    let track = thumb_track(chrome)?;
    let offset = current_offset(chrome)?;
    Some(offset / track.max_offset * (track.track - track.length))
}

/// Returns the length the thumb is drawn with, or zero while it is hidden.
fn thumb_length(chrome: &ScrollChrome) -> f32 {
    thumb_track(chrome).map_or(0.0, |track| track.length)
}

/// Returns the thumb's distance from the top of the track, which maps the
/// offset onto the part of the track the thumb does not cover, or zero while
/// it is hidden. It is rounded to whole pixels, like every laid out edge, so
/// the thumb's edges stay sharp.
fn thumb_top(chrome: &ScrollChrome) -> f32 {
    let Some(track) = thumb_track(chrome) else {
        return 0.0;
    };
    let offset = clamp_offset(chrome.signal().get(), track.max_offset);
    (offset / track.max_offset * (track.track - track.length)).round()
}
