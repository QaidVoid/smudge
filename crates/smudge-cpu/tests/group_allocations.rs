//! A frame must not allocate to draw groups, once their layers exist, nor to
//! draw a list without any.
//!
//! This file is its own test binary so that its counting allocator sees no
//! other test's allocations while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, PixelRect, Rect};
use smudge_core::paint::{DisplayList, PaintOp};
use smudge_cpu::CpuRenderer;

/// How many allocations have been made.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// A wrapper around the system allocator that counts allocations.
struct Counting;

// SAFETY: forwarding to the system allocator with an atomic counter update,
// which is thread-safe and keeps the allocator contract.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller of this allocator hook guarantees a valid layout,
        // and the system allocator upholds the pointer contract for it.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from `alloc` above with the same layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A square of `color` at `x`, `y`.
fn square(x: f32, y: f32, color: Color) -> PaintOp {
    PaintOp::FillRect {
        rect: Rect::new(x, y, x + 20.0, y + 20.0),
        radius: Corners::all(3.0),
        color,
    }
}

/// Renders `list` twice through two damage rectangles, and returns how many
/// allocations the second frame made.
fn second_frame_allocations(list: &DisplayList) -> usize {
    let mut renderer = CpuRenderer::new(64, 64);
    let damage = [
        PixelRect {
            x: 0,
            y: 0,
            width: 64,
            height: 30,
        },
        PixelRect {
            x: 0,
            y: 30,
            width: 64,
            height: 34,
        },
    ];
    let frame = FrameOutput {
        list,
        damage: &damage,
        full: false,
        clear_color: Color::WHITE,
        scale: 1.0,
    };
    renderer.render(&frame);
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    renderer.render(&frame);
    ALLOCATIONS.load(Ordering::Relaxed) - before
}

#[test]
fn frames_with_and_without_groups_allocate_nothing_once_warm() {
    let mut plain = DisplayList::new();
    plain.push_op(square(4.0, 4.0, Color::rgb(200, 40, 40)));
    plain.push_op(square(14.0, 24.0, Color::rgb(40, 40, 200)));
    plain.finish();
    assert_eq!(second_frame_allocations(&plain), 0, "a list without groups");

    let mut grouped = DisplayList::new();
    grouped.push_op(square(4.0, 4.0, Color::rgb(200, 40, 40)));
    grouped.push_group(0.5);
    grouped.push_op(square(14.0, 24.0, Color::rgb(40, 40, 200)));
    grouped.push_group(0.5);
    grouped.push_op(square(30.0, 30.0, Color::rgb(40, 200, 40)));
    grouped.pop_group();
    grouped.pop_group();
    grouped.finish();
    assert_eq!(
        second_frame_allocations(&grouped),
        0,
        "a list with nested groups"
    );
}
