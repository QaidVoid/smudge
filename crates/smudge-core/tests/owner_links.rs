//! Child owners that are created and dropped under one owner must not grow
//! the heap: the links to dropped children are reclaimed.
//!
//! This file is its own test binary so that its counting allocator sees no
//! other test's allocations while it measures.

use smudge_core::reactive::Owner;
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

/// Creates and drops `count` child owners under `parent`, keeping `kept` of
/// them alive at a time.
fn churn(parent: &Owner, count: usize, kept: &mut Vec<Owner>) {
    for index in 0..count {
        let child = parent.child();
        if index % 100 == 0 {
            kept.push(child);
            if kept.len() > 10 {
                kept.remove(0);
            }
        }
    }
}

// One test runs both scenarios in turn: the counting allocator is shared by
// every thread, so two tests running in parallel would count each other.
#[test]
fn dropped_child_owners_do_not_grow_the_heap() {
    churning_children_stay_flat();
    dropping_many_live_children_frees_them_once_new_children_arrive();
}

/// Creating and dropping children under one owner holds the heap flat.
fn churning_children_stay_flat() {
    let parent = Owner::new();
    let mut kept = Vec::new();

    churn(&parent, 10_000, &mut kept);
    let settled = LIVE.load(Ordering::Relaxed);

    churn(&parent, 200_000, &mut kept);
    let after = LIVE.load(Ordering::Relaxed);

    // Between sweeps an owner holds up to about twice its live children as
    // links, so the count may sit a few dropped owners above the settled one,
    // but never anywhere near the 200,000 more owners the second churn made.
    assert!(
        after <= settled + 16 * 1024,
        "200,000 more child owners left {after} live bytes against {settled} settled"
    );
}

/// Dropping a large set of live children frees them on the next adopts,
/// however many children the owner held before.
fn dropping_many_live_children_frees_them_once_new_children_arrive() {
    let parent = Owner::new();
    let mut kept = Vec::new();
    churn(&parent, 1_000, &mut kept);
    let before = LIVE.load(Ordering::Relaxed);

    let many: Vec<Owner> = (0..50_000).map(|_| parent.child()).collect();
    drop(many);
    for _ in 0..16 {
        drop(parent.child());
    }
    let after = LIVE.load(Ordering::Relaxed);

    // Only the small working set and the grown link list may remain. The
    // 50,000 dropped owners themselves would hold about 10 MB.
    let link_list = 50_000 * std::mem::size_of::<usize>() * 2;
    assert!(
        after <= before + 64 * 1024 + link_list,
        "dropped children stayed allocated: {after} live bytes against {before} before"
    );
}
