//! A counting global allocator, so a frame's allocations can be recorded.
//!
//! The counter is only installed in this binary. It wraps [`System`] and adds
//! nothing but the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

/// How many allocations the process has made so far.
///
/// Every call to `alloc`, `alloc_zeroed` and `realloc` counts as one
/// allocation. `dealloc` does not count.
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

/// An allocator that counts allocation calls and forwards to [`System`].
struct Counting;

// SAFETY: the implementation only adds the count and forwards every call to
// `System` unchanged, so its soundness is `System`'s own.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the layout is forwarded to System unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout are forwarded to System unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the pointer, layout and new size are forwarded to System
        // unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the layout is forwarded to System unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Returns how many allocations the process has made so far.
pub(crate) fn count() -> u64 {
    ALLOCATIONS.load(Ordering::Relaxed)
}
