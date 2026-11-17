#![cfg(debug_assertions)]

use crate::geometry::{Color, Corners, Rect};
use crate::paint::{DisplayList, PaintOp};

fn fill() -> PaintOp {
    PaintOp::FillRect {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        radius: Corners::ZERO,
        color: Color::BLACK,
    }
}

#[test]
#[should_panic(expected = "PushClip with no matching PopClip")]
fn finish_panics_when_a_clip_is_not_closed() {
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
    list.push_op(fill());
    list.finish();
}

#[test]
#[should_panic(expected = "PopClip with no open clip")]
fn finish_panics_when_a_clip_is_closed_twice() {
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
    list.pop_clip();
    list.pop_clip();
    list.finish();
}

#[test]
fn finish_accepts_a_balanced_list() {
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
    list.extend_ops(&[fill()]);
    list.push_clip(Rect::new(2.0, 2.0, 4.0, 4.0));
    list.pop_clip();
    list.pop_clip();
    list.finish();
    assert_eq!(list.len(), 5);
}

/// A run at 16 logical pixels carrying `embolden` and `skew`, with no glyphs.
fn synthetic(embolden: f32, skew: f32) -> crate::paint::GlyphRun {
    crate::paint::GlyphRun {
        font: crate::paint::FontBlob {
            id: 0,
            data: std::sync::Arc::new(Vec::new()),
            index: 0,
        },
        size: 16.0,
        color: Color::BLACK,
        glyphs: Vec::new(),
        embolden,
        skew,
        ink_bounds: Rect::default(),
    }
}

#[test]
fn a_hand_built_run_draws_within_the_synthesis_limits() {
    use crate::paint::{MAX_EMBOLDEN, MAX_EMBOLDEN_SHARE, MAX_SKEW};

    assert_eq!(
        synthetic(0.5, 14.0).synthesis(),
        (0.5, 14.0),
        "text's values pass"
    );
    assert_eq!(
        synthetic(1.0e30, 90.0).synthesis(),
        (16.0 * MAX_EMBOLDEN_SHARE, MAX_SKEW)
    );
    assert_eq!(synthetic(-3.0, -90.0).synthesis(), (0.0, -MAX_SKEW));
    assert_eq!(synthetic(f32::NAN, f32::INFINITY).synthesis(), (0.0, 0.0));
    let huge = crate::paint::GlyphRun {
        size: 1.0e30,
        ..synthetic(f32::MAX, 0.0)
    };
    assert_eq!(
        huge.synthesis(),
        (MAX_EMBOLDEN, 0.0),
        "a huge size does not lift the cap"
    );
}
