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
