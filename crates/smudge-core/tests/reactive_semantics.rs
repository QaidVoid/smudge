//! Behavior tests for the frame-driven reactive runtime.
//!
//! Each test states one behavior that the frame loop or the widget tree
//! depends on. Tests that changed with the in-house core, because the old
//! behavior pinned a workaround, say so in a comment.

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{self, Memo, Owner, RenderEffect, RwSignal, prelude::*, signal_local};
use smudge_core::widget::panel;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Installs the runtime on this test thread and makes a new owner current.
///
/// The returned owner must stay alive for the test, because the signals and
/// memos it owns are disposed when it is cleaned up.
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

    let effect = RenderEffect::new(move || recorder.borrow_mut().push(1u32));

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
    let _effect = RenderEffect::new(move || recorder.borrow_mut().push(count.get()));

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
    let _effect = RenderEffect::new(move || recorder.borrow_mut().push(first.get() + second.get()));

    set_first.set(1);
    set_second.set(2);
    reactive::flush();

    assert_eq!(*sums.borrow(), vec![0, 3]);
}

#[test]
fn unchanged_memo_stops_propagation() {
    let _owner = setup();
    let count = RwSignal::new(0i32);
    let source = count.clone();
    let parity = Memo::new(move || source.get() % 2);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _effect = RenderEffect::new(move || recorder.borrow_mut().push(parity.get()));

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
fn memo_with_an_equal_value_does_not_rerun_its_dependents() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let source = count.clone();
    // The memo saturates at 5, so some writes move the source but leave the
    // memo's value where it was.
    let level = Memo::new(move || if source.get() >= 5 { 1u32 } else { 0 });
    let runs = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&runs);
    let _effect = RenderEffect::new(move || {
        counter.set(counter.get() + 1);
        level.get();
    });
    assert_eq!(runs.get(), 1, "the first run happens at creation");

    set_count.set(7);
    reactive::flush();
    assert_eq!(runs.get(), 2, "a real change reaches the effect");

    set_count.set(8);
    reactive::flush();
    assert_eq!(
        runs.get(),
        2,
        "a write that leaves the memo's value equal reruns nothing"
    );
}

#[test]
fn owner_cleanup_stops_its_effects() {
    // This replaces two tests that pinned the old library's behavior, where
    // owner cleanup left a render effect running and only the handle could
    // stop it. Cleanup stopping effects is the documented behavior now.
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let owner = Owner::new();
    owner.with(|| {
        RenderEffect::new(move || recorder.borrow_mut().push(count.get()));
    });
    reactive::flush();
    assert_eq!(*seen.borrow(), vec![0]);

    owner.cleanup();
    set_count.set(1);
    reactive::flush();

    assert_eq!(
        *seen.borrow(),
        vec![0],
        "cleanup stops the effects the owner holds"
    );
}

#[test]
fn dropping_the_owner_stops_its_effects() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let owner = Owner::new();
    owner.with(|| {
        RenderEffect::new(move || recorder.borrow_mut().push(count.get()));
    });
    reactive::flush();
    assert_eq!(*seen.borrow(), vec![0]);

    drop(owner);
    set_count.set(1);
    reactive::flush();

    assert_eq!(
        *seen.borrow(),
        vec![0],
        "dropping the last owner handle cleans the owner up"
    );
}

#[test]
fn dropping_the_handle_disposes_an_unowned_effect() {
    // An effect created with no current owner is owned by its handle, which
    // is the behavior the old handle-dropping workaround provided.
    reactive::init();
    let (count, set_count) = signal_local(0u32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let effect = RenderEffect::new(move || recorder.borrow_mut().push(count.get()));
    assert_eq!(*seen.borrow(), vec![0]);

    drop(effect);
    set_count.set(1);
    reactive::flush();

    assert_eq!(
        *seen.borrow(),
        vec![0],
        "dropping the handle of an unowned effect disposes it"
    );
}

#[test]
fn signal_local_holds_a_non_send_value() {
    let _owner = setup();
    let (shared, set_shared) = signal_local(Rc::new(RefCell::new(0u32)));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _effect = RenderEffect::new(move || recorder.borrow_mut().push(*shared.get().borrow()));

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

    let _downstream = RenderEffect::new(move || recorder.borrow_mut().push(target.get()));
    let _upstream = RenderEffect::new(move || {
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
                        RenderEffect::new(move || recorder.borrow_mut().push(value.get()));

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
    let _first_effect = RenderEffect::new(move || first.get());
    let _second_effect = RenderEffect::new(move || second.get());
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
        "a second effect woken in the same frame asks for nothing"
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
    let _effect = RenderEffect::new(move || count.get());

    // The first run of an effect happens at creation, so the first flush has
    // nothing woken. The old runtime deferred the first run to the flush.
    assert!(!reactive::flush(), "the creation run is not a flush");
    assert!(
        !reactive::flush(),
        "a flush with nothing woken runs no effect"
    );
}

#[test]
fn flush_inside_a_running_flush_does_nothing() {
    let _owner = setup();
    let (trigger, set_trigger) = signal_local(0u32);
    let nested = Rc::new(Cell::new(None));
    let runs = Rc::new(Cell::new(0u32));
    let recorder = Rc::clone(&nested);
    let counter = Rc::clone(&runs);

    let _effect = RenderEffect::new(move || {
        trigger.get();
        counter.set(counter.get() + 1);
        recorder.set(Some(reactive::flush()));
    });
    assert_eq!(runs.get(), 1, "the creation run happens outside any flush");

    set_trigger.set(1);
    reactive::flush();

    assert_eq!(runs.get(), 2, "the write ran the effect inside the flush");
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
    let _failing_effect = RenderEffect::new(move || {
        assert_eq!(failing.get(), 0, "the effect fails once its value changes");
    });
    let (healthy, set_healthy) = signal_local(0u32);
    let _healthy_effect = RenderEffect::new(move || healthy.get());
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
fn effects_queued_behind_a_panicking_effect_run_in_the_next_flush() {
    let _owner = setup();
    let redraws = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&redraws);
    reactive::set_wake_callback(move || counter.set(counter.get() + 1));

    let (trigger, set_trigger) = signal_local(0u32);
    let (relay, set_relay) = signal_local(0u32);
    let set_in_effect = set_relay.clone();
    let _failing = RenderEffect::new(move || {
        if trigger.get() > 0 {
            set_in_effect.set(1);
            panic!("the effect fails after its write");
        }
    });
    let seen = Rc::new(Cell::new(0u32));
    let recorder = Rc::clone(&seen);
    let _reader = RenderEffect::new(move || recorder.set(relay.get()));

    set_trigger.set(1);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(reactive::flush));
    assert!(unwound.is_err(), "the first effect panics inside the flush");
    assert_eq!(seen.get(), 0, "the panic came before the reader ran");
    reactive::flush();
    assert_eq!(seen.get(), 1, "the reader ran in the next flush");

    set_trigger.set(2);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(reactive::flush));
    assert!(unwound.is_err(), "the first effect panics again");
    let before = redraws.get();
    set_relay.set(2);
    assert_eq!(
        redraws.get(),
        before + 1,
        "a write to an effect the unwound flush left queued asks for a redraw"
    );
    reactive::flush();
    assert_eq!(seen.get(), 2);
}

#[test]
fn panicking_effect_runs_again_on_the_next_write() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&runs);
    let _effect = RenderEffect::new(move || {
        counter.set(counter.get() + 1);
        let value = count.get();
        assert_eq!(value, 0, "the effect fails once its value changes");
    });
    assert_eq!(runs.get(), 1, "the creation run happens");

    set_count.set(1);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(reactive::flush));
    assert!(unwound.is_err(), "the effect panics inside the flush");
    assert_eq!(runs.get(), 2, "the write reached the effect");

    set_count.set(2);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(reactive::flush));
    assert!(
        unwound.is_err(),
        "the effect stays subscribed, so the next write runs it again"
    );
    assert_eq!(runs.get(), 3);
}

#[test]
fn wake_during_flush_does_not_request_a_redraw() {
    let _owner = setup();
    let redraws = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&redraws);
    reactive::set_wake_callback(move || counter.set(counter.get() + 1));

    let (armed, set_armed) = signal_local(false);
    let (target, set_target) = signal_local(0u32);
    let set_in_effect = set_target.clone();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _downstream = RenderEffect::new(move || recorder.borrow_mut().push(target.get()));
    let _upstream = RenderEffect::new(move || {
        if armed.get() {
            set_in_effect.set(1);
        }
    });
    reactive::flush();
    assert_eq!(redraws.get(), 0, "a flush is not a wake");

    // The write that arms the effect asks for the redraw. The write the effect
    // makes while that flush runs asks for nothing, because the flush applies
    // it itself.
    set_armed.set(true);
    assert_eq!(redraws.get(), 1, "the write outside the flush asks for one");
    reactive::flush();

    assert_eq!(*seen.borrow(), vec![0, 1], "the write was applied");
    assert_eq!(
        redraws.get(),
        1,
        "a wake that a running flush applies asks for no redraw"
    );

    set_target.set(2);
    assert_eq!(
        redraws.get(),
        2,
        "a write after the flush asks for a redraw again"
    );
}

#[test]
fn an_effect_that_wakes_itself_is_stopped_at_the_run_limit() {
    let _owner = setup();
    let (count, set_count) = signal_local(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&runs);
    let _effect = RenderEffect::new(move || {
        counter.set(counter.get() + 1);
        count.get();
        set_count.update(|value| *value += 1);
    });
    assert_eq!(runs.get(), 1, "the creation run happens once");

    // The creation run reads `count` before it writes it, so the write wakes
    // the effect and the flush enters the cycle. (A body that writes a signal
    // before it reads it does not wake itself: the run clears the effect's
    // source links first and the write then finds no subscriber.)
    //
    // Debug builds panic at the run limit inside the flush, and release builds
    // skip the effect for the rest of the flush. Either way the body runs a
    // fixed number of times and then the flush ends.
    let _unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(reactive::flush));
    let runs_in_flush = runs.get() - 1;
    assert_eq!(
        runs_in_flush, 16,
        "the body runs 16 times in the flush, then the limit stops it"
    );
}

#[test]
fn an_effect_that_reads_a_memo_counts_each_run_once_against_the_limit() {
    let _owner = setup();
    let (forwarded, set_forwarded) = signal_local(0u32);
    let (step, set_step) = signal_local(0u32);
    let current = Memo::new(move || forwarded.get());
    let _forward = RenderEffect::new(move || set_forwarded.set(step.get()));
    let runs = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&runs);
    let _advance = RenderEffect::new(move || {
        let value = current.get();
        counter.set(counter.get() + 1);
        if value < 10 {
            set_step.set(value + 1);
        }
    });

    // The two effects take turns ten times. Each run of the second one pulls
    // the memo, whose change notifies the running effect. That notification
    // must not queue it again, or every run would count twice against the
    // limit of 16 and the chain would be stopped as a cycle.
    let before = runs.get();
    reactive::flush();
    assert_eq!(runs.get() - before, 10, "the chain runs to its end");
}

#[test]
fn an_effect_does_not_compute_a_memo_only_its_old_branch_read() {
    let _owner = setup();
    let (value, set_value) = signal_local(Some(1u32));
    let reader = value.clone();
    let doubled = Memo::new(move || reader.get().expect("read only while some") * 2);
    let _effect = RenderEffect::new(move || {
        if value.get().is_some() {
            doubled.get();
        }
    });

    // The effect's first source moved, so it reruns without bringing the memo
    // up to date, and the new run takes the branch that does not read it.
    set_value.set(None);
    reactive::flush();
}

#[test]
fn cleaning_up_an_owner_drops_effect_bodies_that_hold_effect_handles() {
    reactive::init();
    let unowned = RenderEffect::new(|| ());
    let owner = Owner::new();
    owner.with(|| {
        RenderEffect::new(move || {
            let _held = &unowned;
        });
    });

    // Dropping the body drops the unowned handle, which disposes its effect
    // and so enters the runtime while the cleanup is running.
    owner.cleanup();
}

#[test]
fn context_is_visible_to_descendants_and_not_to_siblings() {
    let _owner = setup();

    let parent = Owner::new();
    parent.with(|| provide_context(7u32));

    let child = parent.child();
    let from_child = child.with(use_context::<u32>);
    assert_eq!(from_child, Some(7), "a descendant reads the context");

    let sibling = Owner::new();
    let from_sibling = sibling.with(use_context::<u32>);
    assert_eq!(
        from_sibling, None,
        "a sibling of the provider reads nothing"
    );

    parent.with(|| provide_context(9u32));
    let updated = child.with(use_context::<u32>);
    assert_eq!(
        updated,
        Some(9),
        "a second value of the same type replaces the first"
    );
}

/// The settings for a one-window app in these tests.
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
fn a_second_app_does_not_take_the_first_app_s_redraw_requests() {
    reactive::init();
    let (dark, set_dark) = signal_local(false);

    let first_redraws = Rc::new(Cell::new(0u32));
    let second_redraws = Rc::new(Cell::new(0u32));

    let mut first = App::new(config(), panel);
    let counter = Rc::clone(&first_redraws);
    first.set_wake_callback(move || counter.set(counter.get() + 1));

    let mut second = App::new(config(), panel);
    let counter = Rc::clone(&second_redraws);
    second.set_wake_callback(move || counter.set(counter.get() + 1));

    // The theme binding belongs to the first app's owner, so its wake reports
    // to the first app's callback. In the old runtime one callback per thread
    // served every app, so the second app took the first one's redraws.
    first.bind_theme(move || {
        dark.get();
        smudge_core::theme::Theme::dark()
    });
    first.frame();
    second.frame();

    set_dark.set(true);
    assert_eq!(
        first_redraws.get(),
        1,
        "the first app's write reaches the first app's callback"
    );
    assert_eq!(
        second_redraws.get(),
        0,
        "the second app's callback stays out of it"
    );
}

#[test]
fn a_write_inside_a_flush_wakes_another_owner_tree() {
    reactive::init();
    let first = Owner::new();
    let second = Owner::new();
    let first_redraws = Rc::new(Cell::new(0u32));
    let second_redraws = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&first_redraws);
    first.set_wake_callback(move || counter.set(counter.get() + 1));
    let counter = Rc::clone(&second_redraws);
    second.set_wake_callback(move || counter.set(counter.get() + 1));

    let (trigger, set_trigger) = signal_local(0u32);
    let (relay, set_relay) = signal_local(0u32);
    first.with(|| {
        RenderEffect::new(move || {
            if trigger.get() > 0 {
                set_relay.set(1);
            }
        })
    });
    second.with(|| RenderEffect::new(move || relay.get()));
    reactive::flush();

    set_trigger.set(1);
    reactive::flush();
    assert_eq!(
        (first_redraws.get(), second_redraws.get()),
        (1, 1),
        "the second tree's app needs a frame to apply what its effect produced"
    );
}

#[test]
fn two_apps_woken_in_one_frame_each_request_a_redraw() {
    reactive::init();
    let (first_dark, set_first_dark) = signal_local(false);
    let (second_dark, set_second_dark) = signal_local(false);
    let (shared, set_shared) = signal_local(0u32);
    let shared_second = shared.clone();

    let first_redraws = Rc::new(Cell::new(0u32));
    let second_redraws = Rc::new(Cell::new(0u32));
    let mut first = App::new(config(), panel);
    let counter = Rc::clone(&first_redraws);
    first.set_wake_callback(move || counter.set(counter.get() + 1));
    let mut second = App::new(config(), panel);
    let counter = Rc::clone(&second_redraws);
    second.set_wake_callback(move || counter.set(counter.get() + 1));

    first.bind_theme(move || {
        first_dark.get();
        shared.get();
        smudge_core::theme::Theme::dark()
    });
    second.bind_theme(move || {
        second_dark.get();
        shared_second.get();
        smudge_core::theme::Theme::dark()
    });
    first.frame();
    second.frame();

    set_first_dark.set(true);
    set_second_dark.set(true);
    assert_eq!(
        (first_redraws.get(), second_redraws.get()),
        (1, 1),
        "the first app's pending redraw does not hide the second app's wake"
    );

    first.frame();
    second.frame();
    set_shared.set(1);
    assert_eq!(
        (first_redraws.get(), second_redraws.get()),
        (2, 2),
        "one write that reaches both apps wakes both"
    );
}
