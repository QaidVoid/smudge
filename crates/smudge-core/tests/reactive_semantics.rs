//! Behavior tests for the frame-driven reactive runtime.
//!
//! Each test states one behavior that the frame loop or the widget tree
//! depends on. Tests that found a behavior different from the plan state what
//! the runtime actually does, so the difference cannot be lost.

use reactive_graph::effect::Effect;
use smudge_core::reactive::{self, Memo, Owner, RenderEffect, RwSignal, prelude::*, signal_local};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Installs the executor on this test thread and makes a new owner current.
///
/// The returned owner must stay alive for the test, because arena-allocated
/// signals and memos are disposed when their owner is cleaned up.
fn setup() -> Owner {
    reactive::init();
    let owner = Owner::new();
    owner.set();
    owner
}

#[test]
fn render_effect_runs_once_when_created() {
    let _owner = setup();
    let runs = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&runs);

    let effect = RenderEffect::new(move |_| recorder.borrow_mut().push(1u32));

    assert_eq!(*runs.borrow(), vec![1], "the first run is synchronous");

    reactive::flush();
    assert_eq!(*runs.borrow(), vec![1], "a flush does not repeat the run");

    drop(effect);
}

#[test]
fn signal_write_waits_for_the_next_flush() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _effect = RenderEffect::new(move |_| recorder.borrow_mut().push(count.get()));

    assert_eq!(*seen.borrow(), vec![0]);

    set_count.set(1);
    assert_eq!(*seen.borrow(), vec![0], "the write does not run the effect");

    reactive::flush();
    assert_eq!(*seen.borrow(), vec![0, 1]);
}

#[test]
fn two_writes_in_one_frame_run_the_effect_once() {
    let _owner = setup();
    let (first, set_first) = signal_local(0u32);
    let (second, set_second) = signal_local(0u32);
    let sums = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&sums);
    let _effect =
        RenderEffect::new(move |_| recorder.borrow_mut().push(first.get() + second.get()));

    set_first.set(1);
    set_second.set(2);
    reactive::flush();

    assert_eq!(*sums.borrow(), vec![0, 3]);
}

#[test]
fn unchanged_memo_stops_propagation() {
    let _owner = setup();
    let count = RwSignal::new(0i32);
    let parity = Memo::new(move |_| count.get() % 2);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _effect = RenderEffect::new(move |_| recorder.borrow_mut().push(parity.get()));

    assert_eq!(*seen.borrow(), vec![0]);

    count.set(2);
    reactive::flush();
    assert_eq!(
        *seen.borrow(),
        vec![0],
        "a memo with an unchanged value does not rerun the effect"
    );

    count.set(3);
    reactive::flush();
    assert_eq!(*seen.borrow(), vec![0, 1]);
}

#[test]
fn owner_cleanup_does_not_cancel_render_effect() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let owner = Owner::new();
    let _effect =
        owner.with(|| RenderEffect::new(move |_| recorder.borrow_mut().push(count.get())));
    owner.cleanup();

    set_count.set(1);
    reactive::flush();

    assert_eq!(*seen.borrow(), vec![0, 1]);
}

#[test]
fn dropping_the_owner_does_not_cancel_render_effect() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let owner = Owner::new();
    let _effect =
        owner.with(|| RenderEffect::new(move |_| recorder.borrow_mut().push(count.get())));
    drop(owner);

    set_count.set(1);
    reactive::flush();

    assert_eq!(*seen.borrow(), vec![0, 1]);
}

#[test]
fn owner_cleanup_cancels_arena_effect() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let owner = Owner::new();
    let _effect = owner.with(|| Effect::new(move |_| recorder.borrow_mut().push(count.get())));
    reactive::flush();
    assert_eq!(*seen.borrow(), vec![0]);

    owner.cleanup();
    set_count.set(1);
    reactive::flush();

    assert_eq!(
        *seen.borrow(),
        vec![0],
        "an effect stored in the owner's arena stops when the owner is cleaned up"
    );
}

#[test]
fn dropping_render_effect_cancels_it() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let effect = RenderEffect::new(move |_| recorder.borrow_mut().push(count.get()));

    drop(effect);
    set_count.set(1);
    reactive::flush();

    assert_eq!(*seen.borrow(), vec![0]);
}

#[test]
fn signal_local_holds_a_non_send_value() {
    let _owner = setup();
    let (shared, set_shared) = signal_local(Rc::new(RefCell::new(0u32)));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _effect = RenderEffect::new(move |_| recorder.borrow_mut().push(*shared.get().borrow()));

    set_shared.update(|value| *value.borrow_mut() += 1);
    reactive::flush();

    assert_eq!(*seen.borrow(), vec![0, 1]);
}

#[test]
fn write_from_effect_is_flushed_in_the_same_call() {
    let _owner = setup();
    let (trigger, set_trigger) = signal_local(0u32);
    let (target, set_target) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let _downstream = RenderEffect::new(move |_| recorder.borrow_mut().push(target.get()));
    let _upstream = RenderEffect::new(move |_| {
        if trigger.get() == 1 {
            set_target.set(7);
        }
    });

    set_trigger.set(1);
    reactive::flush();

    assert_eq!(
        *seen.borrow(),
        vec![0, 7],
        "the write made during the flush is picked up by the same call"
    );
}

#[test]
fn threads_run_independent_runtimes() {
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2u32)
            .map(|index| {
                scope.spawn(move || {
                    reactive::init();
                    let owner = Owner::new();
                    owner.set();

                    let (value, set_value) = signal_local(0u32);
                    let seen = Rc::new(RefCell::new(Vec::new()));
                    let recorder = Rc::clone(&seen);
                    let _effect =
                        RenderEffect::new(move |_| recorder.borrow_mut().push(value.get()));

                    set_value.set(index + 1);
                    reactive::flush();

                    seen.borrow().clone()
                })
            })
            .collect();

        handles
            .into_iter()
            .map(|handle| handle.join().expect("worker thread finishes"))
            .collect::<Vec<_>>()
    });

    assert_eq!(results, vec![vec![0, 1], vec![0, 2]]);
}

#[test]
fn write_outside_flush_requests_one_redraw() {
    let _owner = setup();
    let redraws = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&redraws);
    reactive::set_wake_callback(move || counter.set(counter.get() + 1));

    let (first, set_first) = signal_local(0u32);
    let (second, set_second) = signal_local(0u32);
    let _first_effect = RenderEffect::new(move |_| first.get());
    let _second_effect = RenderEffect::new(move |_| second.get());
    reactive::flush();
    assert_eq!(redraws.get(), 0, "a flush is not a wake");

    set_first.set(1);
    assert_eq!(
        redraws.get(),
        1,
        "the first write after a flush asks for one"
    );

    set_second.set(1);
    assert_eq!(
        redraws.get(),
        1,
        "a second task woken in the same frame asks for nothing"
    );

    reactive::flush();
    set_first.set(2);
    assert_eq!(
        redraws.get(),
        2,
        "the next write after a flush asks for a redraw again"
    );
}

#[test]
fn flush_reports_no_work_when_nothing_was_woken() {
    let _owner = setup();
    let (count, _set_count) = signal_local(0u32);
    let _effect = RenderEffect::new(move |_| count.get());

    assert!(reactive::flush(), "the first flush runs the effect's task");
    assert!(
        !reactive::flush(),
        "a flush with nothing woken runs no task"
    );
}

#[test]
fn flush_inside_a_running_flush_does_nothing() {
    let _owner = setup();
    let nested = Rc::new(Cell::new(None));
    let recorder = Rc::clone(&nested);

    // An arena effect first runs on a flush, so its body is a task that the
    // running flush is polling.
    let _effect = Effect::new(move |_| recorder.set(Some(reactive::flush())));
    reactive::flush();

    assert_eq!(
        nested.get(),
        Some(false),
        "a flush called from inside a running flush does nothing"
    );
}

#[test]
fn panicking_effect_does_not_stop_later_redraw_requests() {
    let _owner = setup();
    let redraws = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&redraws);
    reactive::set_wake_callback(move || counter.set(counter.get() + 1));

    let (failing, set_failing) = signal_local(0u32);
    let _failing_effect = RenderEffect::new(move |_| {
        assert_eq!(failing.get(), 0, "the effect fails once its value changes");
    });
    let (healthy, set_healthy) = signal_local(0u32);
    let _healthy_effect = RenderEffect::new(move |_| healthy.get());
    reactive::flush();

    set_failing.set(1);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(reactive::flush));
    assert!(
        unwound.is_err(),
        "the failing effect panics inside the flush"
    );
    let before = redraws.get();

    set_healthy.set(1);
    assert_eq!(
        redraws.get(),
        before + 1,
        "a write after the unwound flush asks for a redraw"
    );
}

#[test]
fn wake_during_flush_does_not_request_a_redraw() {
    let _owner = setup();
    let redraws = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&redraws);
    reactive::set_wake_callback(move || counter.set(counter.get() + 1));

    let (target, set_target) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _downstream = RenderEffect::new(move |_| recorder.borrow_mut().push(target.get()));

    // The first run of an arena effect happens on the next flush, so the write
    // it makes is applied by a flush that nobody has to be woken for.
    let _upstream = Effect::new(move |_| set_target.set(1));
    reactive::flush();

    assert_eq!(*seen.borrow(), vec![0, 1], "the write was applied");
    assert_eq!(
        redraws.get(),
        0,
        "a wake that a running flush applies asks for no redraw"
    );

    set_target.set(2);
    assert_eq!(
        redraws.get(),
        1,
        "a write after the flush asks for a redraw again"
    );
}
