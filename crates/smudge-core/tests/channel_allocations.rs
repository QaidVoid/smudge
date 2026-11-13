//! A frame must not allocate for the channels it drains.
//!
//! This file is its own test binary so that its counting allocator sees no
//! other test's allocations while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::channel;
use smudge_core::geometry::{Color, Size};
use smudge_core::widget::panel;

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

/// The settings for the apps under test.
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

/// Returns how many allocations one idle frame and one frame check of `app`
/// make, after a few frames have warmed its buffers up.
fn idle_frame_allocations(app: &mut App) -> usize {
    for _ in 0..4 {
        let _ = app.frame();
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let _ = app.frame();
    let _ = app.needs_frame();
    ALLOCATIONS.load(Ordering::Relaxed) - before
}

#[test]
fn live_channels_add_no_allocation_to_an_idle_frame() {
    let mut plain = App::new(config(), panel);
    let without = idle_frame_allocations(&mut plain);

    let holder = Rc::new(RefCell::new(Vec::new()));
    let slot = Rc::clone(&holder);
    let mut with_channels = App::new(config(), move || {
        for _ in 0..3 {
            slot.borrow_mut().push(channel::channel(|_: u32| {}));
        }
        panel()
    });
    let with = idle_frame_allocations(&mut with_channels);
    assert_eq!(
        with, without,
        "an idle frame with live channels allocates no more than one without"
    );
    assert_eq!(holder.borrow().len(), 3, "the senders stay alive");
}
