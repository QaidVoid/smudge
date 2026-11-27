use std::rc::Rc;

use smudge_core::element::{AnyElement, IntoElement};
use smudge_core::geometry::{Point, Size};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Edges, Length};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};

use super::Zone;
use crate::signal;

/// The fraction of a group's shorter span its edge bands reach.
const BAND_FRACTION: f32 = 0.2;
/// The smallest reach of an edge band, in logical pixels.
const BAND_FLOOR: f32 = 24.0;
/// The largest share of the shorter span an edge band takes, so the floor
/// never lets the bands of opposite sides meet over the middle of a small
/// group.
const BAND_CAP: f32 = 0.45;

/// A group's size, measured when the group paints, for the zone logic and
/// the highlight that follow it.
pub(super) struct Frame {
    size: RwSignal<Size>,
}

impl Frame {
    /// Creates a frame that measures zero until its group first paints.
    pub(super) fn new() -> Self {
        Self {
            size: signal(Size::default()),
        }
    }
}

/// Paints as a [`Panel`] while reporting the size of the group that carries
/// it into a shared [`Frame`].
pub(super) struct GroupFrame {
    /// Receives the size of the group this widget paints in.
    pub(super) frame: Rc<Frame>,
}

impl Widget for GroupFrame {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let size = cx.content.size();
        if self.frame.size.read(|current| *current) != size {
            self.frame.size.set(size);
        }
        Panel.paint(cx, out);
    }
}

/// Returns the zone a pointer at `at`, measured from the top left of the
/// group's content area, drops a panel into.
///
/// The tab strip never splits: a pointer over it always lands in the center
/// zone, where the panel joins the group. Below it, the middle third of
/// each side's edge band picks that side, and the rest is the center.
pub(super) fn zone_of(frame: &Frame, at: Point) -> Zone {
    let strip = super::strip::STRIP;
    let size = frame.size.read(|size| *size);
    let below = Point {
        x: at.x,
        y: at.y - strip,
    };
    if below.y < 0.0 {
        return Zone::Center;
    }
    zone_at(
        Size {
            width: size.width,
            height: (size.height - strip).max(0.0),
        },
        below,
    )
}

fn zone_at(size: Size, at: Point) -> Zone {
    let span = size.width.min(size.height);
    let band = (BAND_FRACTION * span).max(BAND_FLOOR).min(BAND_CAP * span);
    let third_x = size.width / 3.0;
    let third_y = size.height / 3.0;
    let middle_x = at.x >= third_x && at.x <= size.width - third_x;
    let middle_y = at.y >= third_y && at.y <= size.height - third_y;
    if at.y < band && middle_x {
        Zone::Top
    } else if at.y > size.height - band && middle_x {
        Zone::Bottom
    } else if at.x < band && middle_y {
        Zone::Left
    } else if at.x > size.width - band && middle_y {
        Zone::Right
    } else {
        Zone::Center
    }
}

/// Builds the translucent rectangle a group draws while a dragged panel
/// would land in one of its zones.
///
/// The rectangle hangs out of flow, so showing it moves nothing, and only
/// the group whose frame the dock's shared hover names shows it. It draws
/// under the drag preview. Each rectangle spans the pane the zone names,
/// below the strip the group that lands there draws, so it never covers a
/// tab strip: tabs stay reachable for drops while it shows. None shows
/// while the pointer is over a tab strip either, because the strip is the
/// surface its tabs own, and a release over it joins the group the strip
/// names wherever the pointer left the pane.
pub(super) fn highlight(zone: RwSignal<Option<(Rc<Frame>, Zone)>>, frame: Rc<Frame>) -> AnyElement {
    let hidden = {
        let zone = zone.clone();
        let frame = Rc::clone(&frame);
        move || {
            !zone
                .get()
                .is_some_and(|(shown, _)| Rc::ptr_eq(&shown, &frame))
        }
    };
    let inset = {
        let frame = Rc::clone(&frame);
        move || {
            let size = frame.size.get();
            let zone = zone
                .get()
                .filter(|(shown, _)| Rc::ptr_eq(shown, &frame))
                .map(|(_, zone)| zone);
            rect(size, zone.unwrap_or(Zone::Center))
        }
    };
    panel()
        .absolute()
        .inset(inset)
        .background(ColorToken::ACCENT)
        .opacity(0.25)
        .hidden(hidden)
        .into_any()
}

fn rect(size: Size, zone: Zone) -> Edges<Length> {
    let strip = super::strip::STRIP;
    let (top, right, bottom, left) = match zone {
        Zone::Center => (strip, 0.0, 0.0, 0.0),
        Zone::Left => (strip, size.width / 2.0, 0.0, 0.0),
        Zone::Right => (strip, 0.0, 0.0, size.width / 2.0),
        Zone::Top => (strip, 0.0, size.height / 2.0, 0.0),
        Zone::Bottom => (size.height / 2.0 + strip, 0.0, 0.0, 0.0),
    };
    Edges {
        top: Length::px(top),
        right: Length::px(right),
        bottom: Length::px(bottom),
        left: Length::px(left),
    }
}
