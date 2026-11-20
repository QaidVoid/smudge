use std::time::Duration;

use smudge_core::geometry::Rect;
use smudge_core::paint::PaintOp;
use smudge_core::style::Style;
use smudge_core::theme::Theme;
use smudge_core::widget::{PaintCx, Widget};

use super::{BAR, FRAMES, FrameRecord, Timeline, TimelineView};

/// A presented frame whose layout took `micros`.
fn frame(micros: u64) -> FrameRecord {
    FrameRecord {
        layout: Duration::from_micros(micros),
        presented: true,
        ..FrameRecord::default()
    }
}

/// The operations `timeline` paints in a box of 300 by 96.
fn painted(timeline: Timeline) -> Vec<PaintOp> {
    let view = TimelineView {
        timeline,
        hovered: None,
    };
    let rect = Rect::new(0.0, 0.0, 300.0, 96.0);
    let style = Style::default();
    let theme = Theme::dark();
    let mut out = Vec::new();
    view.paint(
        &PaintCx {
            rect,
            content: rect,
            style: &style,
            theme: &theme,
        },
        &mut out,
    );
    out
}

#[test]
fn the_timeline_keeps_the_last_frames_and_drops_the_oldest() {
    let mut timeline = Timeline::default();
    for index in 0..FRAMES as u64 + 60 {
        timeline.push(frame(index));
    }
    assert_eq!(timeline.frames().len(), FRAMES);
    assert_eq!(
        timeline.frames().next().map(|frame| frame.layout),
        Some(Duration::from_micros(60)),
        "the oldest kept frame is the 61st"
    );
}

#[test]
fn the_bar_under_the_pointer_is_its_frame_with_the_newest_at_the_right() {
    let mut timeline = Timeline::default();
    for index in 0..3 {
        timeline.push(frame(index));
    }
    let width = 300.0;
    assert_eq!(
        timeline
            .at(width - BAR / 2.0, width)
            .map(|frame| frame.layout),
        Some(Duration::from_micros(2))
    );
    assert_eq!(
        timeline
            .at(width - 2.5 * BAR, width)
            .map(|frame| frame.layout),
        Some(Duration::from_micros(0))
    );
    assert_eq!(
        timeline.at(width - 4.0 * BAR, width),
        None,
        "left of the oldest bar"
    );
}

#[test]
fn a_skipped_present_leaves_a_gap() {
    let mut timeline = Timeline::default();
    timeline.push(frame(4000));
    timeline.push(FrameRecord {
        presented: false,
        ..frame(4000)
    });
    timeline.push(frame(4000));
    let bars: Vec<f32> = painted(timeline)
        .iter()
        .filter_map(|op| match op {
            PaintOp::FillRect { rect, .. } if rect.height() > 1.0 => Some(rect.x0),
            _ => None,
        })
        .collect();
    assert_eq!(
        bars,
        vec![300.0 - 3.0 * BAR, 300.0 - BAR],
        "the middle frame draws no bar"
    );
}

#[test]
fn a_bar_over_budget_crosses_the_budget_line() {
    let mut timeline = Timeline::default();
    timeline.push(frame(25_000));
    let ops = painted(timeline);
    let line = ops
        .iter()
        .find_map(|op| match op {
            PaintOp::FillRect { rect, .. } if rect.height() == 1.0 => Some(rect.y0),
            _ => None,
        })
        .expect("the budget line is drawn");
    let bar_top = ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::FillRect { rect, .. } if rect.height() > 1.0 => Some(rect.y0),
            _ => None,
        })
        .fold(f32::INFINITY, f32::min);
    assert!(bar_top < line, "25 ms reaches above the 16.7 ms line");
}
