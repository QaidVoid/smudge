//! Mount and unmount cycles must not grow the heap.
//!
//! This file is its own test binary so that its counting allocator sees no
//! other test's allocations while it measures.

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::show;
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{self, prelude::*, signal_local};
use smudge_core::style::px;
use smudge_core::widget::panel;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The live byte count, grown by every allocation and shrunk by every deallocation.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// A wrapper around the system allocator that keeps the live byte count.
struct Counting;

// SAFETY: forwarding to the system allocator with an atomic counter update,
// which is thread-safe and keeps the allocator contract.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller of this allocator hook guarantees a valid layout,
        // and the system allocator upholds the pointer contract for it.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from `alloc` above with the same layout.
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Returns the live heap bytes right now.
fn live_heap_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// The settings for the app under test.
fn config() -> AppConfig {
    AppConfig {
        viewport: Size {
            width: 100.0,
            height: 60.0,
        },
        scale: 1.0,
        clear_color: Color::WHITE,
        repaint: RepaintMode::Damage,
    }
}

#[test]
fn mount_and_unmount_cycles_do_not_grow_the_heap() {
    reactive::init();
    let (visible, set_visible) = signal_local(true);
    let mut app = App::new(config(), move || {
        panel().child(show(
            move || visible.get(),
            || panel().width(px(10.0)).height(px(10.0)),
        ))
    });
    app.frame();

    // TOGGLE=0 runs the same frames with a static tree, to separate the cost
    // of the frames from the cost of the mount cycles.
    let toggling = std::env::var("TOGGLE")
        .map(|value| value != "0")
        .unwrap_or(true);

    if !toggling {
        for cycle in 0..1000 {
            app.frame();
            // A window hands these to its accessibility backend every frame;
            // draining keeps the tree's pending-change buffer out of the
            // measurement, like it is out of it in a real window.
            app.take_a11y_changes();
            if (cycle + 1) % 200 == 0 {
                eprintln!("static cycle {}: live = {}", cycle + 1, live_heap_bytes());
            }
        }
        let settled = live_heap_bytes();
        assert_eq!(live_heap_bytes(), settled, "static frames stay flat");
        return;
    }

    // The first cycles grow every buffer the frame pipeline reuses, so the
    // measurement starts from a settled tree and pipeline. The eprintln runs
    // before the settled count is taken, so the harness's capture buffer is
    // part of the baseline and cannot pollute the measured window: any write
    // between the two counts would grow that buffer and show up as a leak.
    for _ in 0..100 {
        set_visible.set(false);
        app.frame();
        app.take_a11y_changes();
        set_visible.set(true);
        app.frame();
        app.take_a11y_changes();
    }
    eprintln!("settling done: live = {}", live_heap_bytes());
    let settled = live_heap_bytes();

    for _ in 0..1000 {
        set_visible.set(false);
        app.frame();
        app.take_a11y_changes();
        set_visible.set(true);
        app.frame();
        app.take_a11y_changes();
    }
    let after = live_heap_bytes();

    assert!(
        after <= settled,
        "1000 more mount and unmount cycles left {after} live bytes, which is \
         more than the {settled} the first 100 cycles settled at"
    );
}
