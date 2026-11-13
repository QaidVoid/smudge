//! The scroll view: a clipped viewport whose content moves with the wheel.
//!
//! The view is a [`ScrollView`] panel that clips its content, handles wheel
//! events while it is hovered, and clamps its offset to what the content
//! needs. The offset is the view's scroll offset, which the core applies after
//! layout, so scrolling moves the content without a layout pass. The thumb
//! moves the same way, through the scroll offset of a track it hangs from.
//! Through [`ScrollExt::follow_end`] the view can also pin itself to the end
//! of its content while a signal reads `true`, and release itself when a
//! wheel step leaves the end.
//!
//! The view learns its own height and its content's height when the two
//! paint, and writes them to signals its bindings read. A size-driven change,
//! such as a thumb that follows a resize, therefore lands one frame after the
//! resize that caused it, which keeps layout out of the paint pass. The
//! follow pin is the one exception: the core applies it after each layout
//! pass with the heights that pass just wrote, so a frame that grows the
//! content never shows the end out of view.

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::element::{Element, IntoElement, Value};
use smudge_core::geometry::Point;
use smudge_core::input::Handled;
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, Semantics};
use smudge_core::style::{Edges, Length, Overflow, px};
use smudge_core::theme::ColorToken;
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

/// How far from the end of the content a wheel step down may land, in
/// logical pixels, and still engage following.
const FOLLOW_SLACK: f32 = 4.0;

/// The heights a scroll view measures when it paints.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    /// The height of the view's content box, the part of the content that
    /// fits on screen at once.
    visible: f32,
    /// The height inside the view's border, which the thumb travels along.
    track: f32,
}

/// The reactive state one scroll view shares between its widgets, its wheel
/// handler and its bindings.
struct ScrollChrome {
    /// The signal whose value is the scroll offset. It starts as one the view
    /// created itself, and [`ScrollExt::offset`] swaps in one of the user's
    /// while the view is built.
    offset: RefCell<RwSignal<f32>>,
    /// The signal that reports and sets whether the view follows the end of
    /// its content, when [`ScrollExt::follow_end`] set one.
    following: RefCell<Option<RwSignal<bool>>>,
    /// The height of the content, once the content has painted.
    content_height: RwSignal<Option<f32>>,
    /// The height the content takes in the coming layout, when the view's
    /// owner knows it before the content paints. It stands in for
    /// `content_height`, which lags it by a frame.
    content_extent: Option<RwSignal<f32>>,
    /// The heights of the view, once the view has painted.
    viewport: RwSignal<Option<Viewport>>,
    /// The visible height of the view, once the view has painted, reported to
    /// a virtual list that computes its rows from it.
    visible_height: RwSignal<Option<f32>>,
}

impl ScrollChrome {
    /// Returns the current offset signal, whichever one
    /// [`ScrollExt::offset`] installed.
    fn signal(&self) -> RwSignal<f32> {
        self.offset.borrow().clone()
    }

    /// Returns the furthest the view can scroll, or `None` before both the
    /// view and its content have painted. Reading it tracks both heights.
    fn max_offset(&self) -> Option<f32> {
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
    fn following(&self) -> Option<RwSignal<bool>> {
        self.following.borrow().clone()
    }
}

/// A panel that clips its content and scrolls it with the wheel.
///
/// It paints like a [`Panel`], so it takes a background and a border through
/// the style builders. Its paint also reports the view's heights to its
/// bindings, which is how the offset learns its range and the thumb its
/// track.
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
/// at the right edge shows the position while the content overflows. The
/// thumb is a position indicator only: it has no handlers and no dragging.
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
pub fn scroll(content: impl IntoElement) -> Element<ScrollView> {
    scroll_with_visible_height(content, RwSignal::new(None), None)
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
pub(crate) fn scroll_with_visible_height(
    content: impl IntoElement,
    visible_height: RwSignal<Option<f32>>,
    content_extent: Option<RwSignal<f32>>,
) -> Element<ScrollView> {
    let chrome = Rc::new(ScrollChrome {
        offset: RefCell::new(RwSignal::new(0.0)),
        following: RefCell::new(None),
        content_height: RwSignal::new(None),
        content_extent,
        viewport: RwSignal::new(None),
        visible_height,
    });

    let wheel_chrome = Rc::clone(&chrome);
    let offset_chrome = Rc::clone(&chrome);
    let content_chrome = Rc::clone(&chrome);
    let track_chrome = Rc::clone(&chrome);

    Element::new(ScrollView {
        chrome: Rc::clone(&chrome),
    })
    .column()
    .overflow(Overflow::Scroll)
    // The view takes every wheel event over it, even one it cannot use, so
    // a page around a view at its end does not scroll in its place.
    .on_wheel(move |delta| {
        let Some(max) = wheel_chrome.max_offset() else {
            return Handled::Yes;
        };
        let follow_signal = wheel_chrome.following();
        // The signal, not the offset the binding last applied, is where a
        // step starts: several wheel events can arrive before the next flush,
        // and each must add to the one before it. While the view follows it
        // sits at the end, so the step starts there even when the signal has
        // not caught up with the pin yet.
        let following = follow_signal
            .as_ref()
            .is_some_and(|signal| signal.read(|active| *active));
        let signal = wheel_chrome.signal();
        let current = if following {
            max
        } else {
            clamp_offset(signal.read(|offset| *offset), max)
        };
        let step = finite(delta.y);
        let next = clamp_offset(current + step, max);
        if next != current {
            signal.set(next);
        }
        // Any step that moves the view up releases the pin, so small steps
        // add up instead of each one snapping back. The slack applies only
        // to a step down, which engages the pin when it lands near the end.
        if let Some(signal) = &follow_signal {
            if following && next < current {
                signal.set(false);
            } else if !following && step > 0.0 && next >= max - FOLLOW_SLACK {
                signal.set(true);
            }
        }
        Handled::Yes
    })
    .scroll_offset(move || {
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
        Point { x: 0.0, y: applied }
    })
    .child(
        Element::new(ScrollContent {
            chrome: content_chrome,
        })
        .column()
        .shrink(0.0)
        .child(content),
    )
    .child(thumb(track_chrome))
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
    /// The wheel writes the same signal, and the view writes the clamped
    /// offset back to it, so the view, the thumb and the signal stay in step.
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
    /// wheel step that moves the view up from the end writes `false`, and a
    /// step down that lands within four logical pixels of the end writes
    /// `true`. Writing `true` from the application jumps to the end.
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
}

/// The key of a view's offset signal binding.
struct OffsetKey;

/// The key of a view's follow signal binding.
struct FollowKey;

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
    *view.chrome.following.borrow_mut() = Some(signal);
    Dirty::Clean
}

/// Builds the scrollbar of one scroll view: a track along the right edge, and
/// the thumb that hangs from it.
///
/// The track is absolutely positioned and zero pixels tall, so it covers
/// nothing a pointer could hit, and the thumb is its child in the flow. The
/// track's scroll offset moves the thumb down to its position, so the thumb
/// follows a scroll without a layout pass. The thumb's length is the visible
/// share of the content, at least [`MIN_THUMB_LENGTH`], and it hides while
/// the content fits.
fn thumb(chrome: Rc<ScrollChrome>) -> Element<Panel> {
    let hidden_chrome = Rc::clone(&chrome);
    let length_chrome = Rc::clone(&chrome);
    let top_chrome = chrome;
    panel()
        .absolute()
        .inset(Edges {
            top: Length::px(0.0),
            right: Length::px(THUMB_RIGHT_INSET),
            bottom: Length::Auto,
            left: Length::Auto,
        })
        .width(px(THUMB_WIDTH))
        .height(px(0.0))
        .column()
        .hidden(move || thumb_track(&hidden_chrome).is_none())
        .scroll_offset(move || Point {
            x: 0.0,
            y: -thumb_top(&top_chrome),
        })
        .child(
            panel()
                .width(px(THUMB_WIDTH))
                .height(move || Length::px(thumb_length(&length_chrome)))
                .shrink(0.0)
                .corner_radius(THUMB_WIDTH / 2.0)
                .background(ColorToken::BORDER),
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
