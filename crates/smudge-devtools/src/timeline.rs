//! The frame timeline: what each of the app's last frames cost, stage by
//! stage, and what it repainted.

use std::collections::VecDeque;
use std::time::Duration;

use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::PaintOp;
use smudge_core::widget::{PaintCx, Widget};

/// How many frames the timeline keeps.
pub(crate) const FRAMES: usize = 240;

/// The time one frame may take at 60 frames a second, which the timeline
/// draws as its budget line.
pub(crate) const BUDGET: Duration = Duration::from_micros(16_667);

/// The width of one frame's bar, in logical pixels.
pub(crate) const BAR: f32 = 1.2;

/// The height of the timeline, in logical pixels. The budget line sits at
/// half of it.
pub(crate) const HEIGHT: f32 = 96.0;

/// One frame of the app as the devtools record it: what each stage cost and
/// what the frame repainted.
///
/// The fields are plain values, so the window that feeds them needs to name
/// no type of this crate's but this one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameRecord {
    /// Running the effects the frame's changes woke.
    pub flush: Duration,
    /// Laying the tree out.
    pub layout: Duration,
    /// Painting the nodes that changed.
    pub paint: Duration,
    /// Building the display list.
    pub list: Duration,
    /// Working out the damage.
    pub damage: Duration,
    /// Drawing the frame.
    pub raster: Duration,
    /// Waiting for a buffer to draw into.
    pub acquire: Duration,
    /// Copying and presenting the frame.
    pub present: Duration,
    /// Whether the frame was presented. A frame that changed nothing while
    /// the app draws frames on its own skips its present, which the timeline
    /// shows as a gap.
    pub presented: bool,
    /// How many nodes the frame painted.
    pub nodes_painted: u32,
    /// How many rectangles the frame's damage was merged into.
    pub damage_rects: u32,
    /// How many physical pixels the merged damage covers.
    pub damage_pixels: u64,
    /// How many physical pixels the damage covered before merging.
    pub damage_pixels_before_merge: u64,
    /// How many rounds the frame's flush ran.
    pub flush_rounds: u32,
    /// Whether the frame laid the tree out.
    pub layout_ran: bool,
}

impl FrameRecord {
    /// The cost of each stage, in the order the timeline stacks them from
    /// the bottom.
    pub(crate) fn stages(&self) -> [Duration; 8] {
        [
            self.flush,
            self.layout,
            self.paint,
            self.list,
            self.damage,
            self.raster,
            self.acquire,
            self.present,
        ]
    }

    /// What the whole frame cost.
    pub(crate) fn total(&self) -> Duration {
        self.stages().iter().sum()
    }

    /// Describes what the frame did, as hovering its bar shows it.
    pub(crate) fn describe(&self) -> String {
        format!(
            "{} {} painted, {} damage {} over {} of {} pixels, {} flush {}, {}",
            self.nodes_painted,
            plural(self.nodes_painted, "node", "nodes"),
            self.damage_rects,
            plural(self.damage_rects, "rect", "rects"),
            self.damage_pixels,
            self.damage_pixels_before_merge,
            self.flush_rounds,
            plural(self.flush_rounds, "round", "rounds"),
            if self.layout_ran {
                "laid out"
            } else {
                "no layout"
            },
        )
    }
}

/// Returns `one` when `count` is one and `many` otherwise.
fn plural(count: u32, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

/// The names of the stages, in the order of [`FrameRecord::stages`].
pub(crate) const STAGE_NAMES: [&str; 8] = [
    "Effects",
    "Layout",
    "Paint",
    "Display list",
    "Damage",
    "Draw",
    "Buffer wait",
    "Present",
];

/// The colors of the stages, from the bottom of a bar to its top.
pub(crate) const STAGE_COLORS: [Color; 8] = [
    Color::rgb(120, 160, 230),
    Color::rgb(90, 200, 160),
    Color::rgb(230, 190, 90),
    Color::rgb(200, 130, 220),
    Color::rgb(230, 120, 120),
    Color::rgb(110, 210, 230),
    Color::rgb(160, 160, 160),
    Color::rgb(240, 150, 80),
];

/// The color of the budget line.
const BUDGET_COLOR: Color = Color::rgba(255, 255, 255, 110);

/// The length of a dash of the budget line and of the gap after it, in
/// logical pixels.
const DASH: f32 = 4.0;

/// The color of the band behind the bar under the pointer.
const HOVERED: Color = Color::rgba(255, 255, 255, 40);

/// What the presented frames of a [`Timeline`] cost.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Summary {
    /// How many frames were presented.
    pub(crate) frames: usize,
    /// What the newest presented frame cost.
    pub(crate) last: Duration,
    /// What a frame cost on average.
    pub(crate) average: Duration,
    /// What the slowest frame cost.
    pub(crate) slowest: Duration,
    /// How many frames cost more than [`BUDGET`].
    pub(crate) over: usize,
    /// What each stage cost on average, in the order of
    /// [`FrameRecord::stages`].
    pub(crate) stages: [Duration; 8],
}

/// The app's last frames, oldest first, at most [`FRAMES`] of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Timeline {
    /// The frames, oldest first.
    frames: VecDeque<FrameRecord>,
}

impl Timeline {
    /// Adds `record` as the newest frame, dropping the oldest past
    /// [`FRAMES`].
    pub(crate) fn push(&mut self, record: FrameRecord) {
        if self.frames.len() == FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(record);
    }

    /// Returns the frames, oldest first.
    pub(crate) fn frames(&self) -> impl ExactSizeIterator<Item = &FrameRecord> {
        self.frames.iter()
    }

    /// Makes this timeline hold the frames of `other`, reusing its own
    /// storage.
    pub(crate) fn copy_from(&mut self, other: &Timeline) {
        self.frames.clone_from(&other.frames);
    }

    /// Sums up what the presented frames cost.
    pub(crate) fn summary(&self) -> Summary {
        let mut summary = Summary::default();
        let mut total = Duration::ZERO;
        for frame in self.frames.iter().filter(|frame| frame.presented) {
            let cost = frame.total();
            summary.frames += 1;
            summary.last = cost;
            summary.slowest = summary.slowest.max(cost);
            summary.over += usize::from(cost > BUDGET);
            total += cost;
            for (sum, stage) in summary.stages.iter_mut().zip(frame.stages()) {
                *sum += stage;
            }
        }
        if let Ok(count) = u32::try_from(summary.frames)
            && count > 0
        {
            summary.average = total / count;
            for stage in &mut summary.stages {
                *stage /= count;
            }
        }
        summary
    }

    /// Returns the frame whose bar holds `x`, in logical pixels from the
    /// timeline's left edge, or `None` where no bar is.
    ///
    /// The newest frame's bar ends at the right edge, `width` from the left.
    pub(crate) fn at(&self, x: f32, width: f32) -> Option<&FrameRecord> {
        let from_right = ((width - x) / BAR).floor();
        if !(0.0..self.frames.len() as f32).contains(&from_right) {
            return None;
        }
        self.frames.get(self.frames.len() - 1 - from_right as usize)
    }
}

/// The widget that draws a [`Timeline`] as stacked bars.
#[derive(Default)]
pub(crate) struct TimelineView {
    /// The frames it draws.
    pub(crate) timeline: Timeline,
    /// Where the pointer is, in logical pixels from the view's left edge,
    /// while it is over the view.
    pub(crate) hovered: Option<f32>,
}

impl Widget for TimelineView {
    /// Draws a bar per frame, newest at the right edge, each stage stacked
    /// on the one below in its color, at a height that puts the budget at
    /// half the timeline. A frame whose present was skipped draws no bar,
    /// and a dashed line marks the budget, so a bar that crosses it was
    /// over. A band marks the bar under the pointer.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        let area = cx.content;
        let scale = HEIGHT / 2.0 / BUDGET.as_secs_f32();
        let count = self.timeline.frames.len();
        if let Some(x) = self.hovered {
            let from_right = ((area.width() - x) / BAR).floor();
            if (0.0..count as f32).contains(&from_right) {
                let right = area.x1 - from_right * BAR;
                out.push(PaintOp::FillRect {
                    rect: Rect::new(right - BAR, area.y0, right, area.y1),
                    radius: Corners::all(0.0),
                    color: HOVERED,
                });
            }
        }
        for (index, frame) in self.timeline.frames().enumerate() {
            if !frame.presented {
                continue;
            }
            let right = area.x1 - (count - 1 - index) as f32 * BAR;
            let left = right - BAR;
            let mut bottom = area.y1;
            for (stage, color) in frame.stages().iter().zip(STAGE_COLORS) {
                let height = stage.as_secs_f32() * scale;
                let top = (bottom - height).max(area.y0);
                if top < bottom {
                    out.push(PaintOp::FillRect {
                        rect: Rect::new(left, top, right, bottom),
                        radius: Corners::all(0.0),
                        color,
                    });
                }
                bottom = top;
            }
        }
        let budget = area.y1 - HEIGHT / 2.0;
        let mut x = area.x0;
        while x < area.x1 {
            out.push(PaintOp::FillRect {
                rect: Rect::new(x, budget, (x + DASH).min(area.x1), budget + 1.0),
                radius: Corners::all(0.0),
                color: BUDGET_COLOR,
            });
            x += 2.0 * DASH;
        }
    }
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
