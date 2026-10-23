//! Frame-driven execution of reactive effects.
//!
//! Signals and effects come from the reactive graph crate. An effect is a
//! task, and a task runs only when something polls the executor it was spawned
//! on. This module installs an executor whose tasks are polled by the frame,
//! so a signal write wakes an effect and the effect runs at the next [`flush`]
//! call rather than immediately.
//!
//! Every thread owns its own task pool. The executor installed in the process
//! is global, but it dispatches to the pool of the thread that calls it, so
//! effects on two threads never share a queue. Call [`init`] on each thread
//! that creates effects.
//!
//! A wake is also reported to the callback set by [`set_wake_callback`], which
//! is how the platform learns that the next frame has work to do. It is not
//! reported for a wake that happens while a flush is running, because that
//! flush is already draining the queue.
//!
//! The reactive API is re-exported here so that a widget imports it from one
//! place. Types are re-exported at the module root and the traits that give
//! them their methods are in [`prelude`].
//!
//! ```
//! use smudge_core::reactive::{self, Owner, RenderEffect, prelude::*};
//! use std::cell::Cell;
//! use std::rc::Rc;
//!
//! reactive::init();
//! let owner = Owner::new();
//! owner.set();
//!
//! let (count, set_count) = reactive::signal_local(0u32);
//! let seen = Rc::new(Cell::new(0u32));
//! let recorder = Rc::clone(&seen);
//! let _effect = RenderEffect::new(move |_| recorder.set(count.get()));
//!
//! set_count.set(1);
//! assert_eq!(seen.get(), 0, "the write waits for the frame");
//!
//! reactive::flush();
//! assert_eq!(seen.get(), 1);
//! ```

use any_spawner::{CustomExecutor, Executor, PinnedFuture, PinnedLocalFuture};
use futures_executor::{LocalPool, LocalSpawner};
use futures_task::{ArcWake, LocalSpawn};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

pub use reactive_graph::{
    computed::Memo,
    effect::RenderEffect,
    owner::{Owner, provide_context, use_context},
    signal::{ReadSignal, RwSignal, WriteSignal, signal_local},
};

/// The traits and constructors that signals, memos and owners need.
///
/// Import this module with a glob to call `.get()`, `.set()`, `.update()`,
/// `.with()` and `.read()` on reactive values, and to create signals and
/// provide context.
pub mod prelude {
    pub use super::{provide_context, signal_local, use_context};
    pub use reactive_graph::traits::{Get, Read, Set, Update, With};
}

thread_local! {
    /// The reactive tasks spawned on this thread.
    static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
    /// The handle that adds tasks to this thread's pool.
    static SPAWNER: LocalSpawner = POOL.with(|pool| pool.borrow().spawner());
    /// Whether this thread has installed the frame executor.
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// Whether a task was woken since the last flush finished.
    static WOKEN: Cell<bool> = const { Cell::new(false) };
    /// Whether a task was polled during the current flush.
    static RAN: Cell<bool> = const { Cell::new(false) };
    /// How many flushes this thread has started.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// Whether this thread is inside a flush.
    static FLUSHING: Cell<bool> = const { Cell::new(false) };
    /// The redraw request of this thread, when the platform set one.
    static WAKE_CALLBACK: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Whether this process has already installed the frame executor's function
/// table, as opposed to another executor having installed its own.
static INSTALLED_IN_PROCESS: Mutex<bool> = Mutex::new(false);

/// Installs the frame executor on the calling thread.
///
/// The executor itself is process-global, but the task pool it dispatches to
/// is per thread, so every thread that creates effects must call this once
/// before it creates them. Calling it again on the same thread does nothing.
///
/// # Panics
///
/// Panics if another async executor was installed in this process first. The
/// executor is process-global, so two executors cannot coexist.
///
/// # Examples
///
/// ```
/// use smudge_core::reactive;
///
/// reactive::init();
/// reactive::flush();
/// ```
pub fn init() {
    INSTALLED.with(|installed| {
        if installed.get() {
            return;
        }

        let mut installed_in_process = INSTALLED_IN_PROCESS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // The per-thread instance is stored before the process-wide function
        // table is set, so this call succeeds on every thread even when the
        // table is already ours.
        match Executor::init_local_custom_executor(FrameExecutor) {
            Ok(()) => *installed_in_process = true,
            Err(error) => assert!(
                *installed_in_process,
                "the async executor installed in this process is not the \
                 frame executor: {error}"
            ),
        }

        installed.set(true);
    });
}

/// Runs every reactive task woken since the last call, until none can make
/// progress, and returns whether any task ran.
///
/// Call this once per frame, before reading the updates that effects produced.
/// A task woken by a write made while the pool is running also runs in this
/// call, so the return value covers everything that write set off.
///
/// Does nothing and returns `false` when called from inside a task that this
/// pool is running, because that task already runs until the pool stalls.
///
/// # Examples
///
/// ```
/// use smudge_core::reactive;
///
/// reactive::init();
/// assert!(!reactive::flush(), "no effect was woken");
/// ```
pub fn flush() -> bool {
    POOL.with(|pool| {
        let Ok(mut pool) = pool.try_borrow_mut() else {
            return false;
        };

        let _flushing = FlushingGuard::enter();
        RAN.set(false);
        GENERATION.set(GENERATION.get() + 1);
        pool.run_until_stalled();

        RAN.get()
    })
}

/// Returns how many flushes the calling thread has started, which identifies
/// the flush that is running or, between flushes, the one that ran last.
pub(crate) fn flush_generation() -> u64 {
    GENERATION.get()
}

/// Marks the calling thread as flushing until dropped.
///
/// Dropping also clears the wake flag, so a flush that unwinds from a panicking
/// effect still leaves the thread able to request the next redraw.
#[must_use = "the thread stops counting as flushing as soon as the guard drops"]
struct FlushingGuard;

impl FlushingGuard {
    fn enter() -> FlushingGuard {
        FLUSHING.set(true);
        FlushingGuard
    }
}

impl Drop for FlushingGuard {
    fn drop(&mut self) {
        FLUSHING.set(false);
        WOKEN.set(false);
    }
}

/// Sets the callback that a wake requests a redraw through.
///
/// The callback runs on the first wake after a flush and not again until the
/// next flush, so a frame that writes many signals requests one redraw. It
/// does not run for a wake that arrives during a flush, because that flush
/// applies the write itself.
///
/// Replaces any previous callback on the calling thread. A wake is reported on
/// the thread that writes the signal, so write signals only on the thread that
/// runs their effects. A write from another thread still wakes the effect, but
/// it requests no redraw on the effect's thread.
///
/// # Examples
///
/// ```
/// use smudge_core::reactive::{self, Owner, RenderEffect, prelude::*};
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// reactive::init();
/// let owner = Owner::new();
/// owner.set();
///
/// let redraws = Rc::new(Cell::new(0u32));
/// let counter = Rc::clone(&redraws);
/// reactive::set_wake_callback(move || counter.set(counter.get() + 1));
///
/// let (count, set_count) = reactive::signal_local(0u32);
/// let _effect = RenderEffect::new(move |_| count.get());
/// reactive::flush();
///
/// set_count.set(1);
/// assert_eq!(redraws.get(), 1);
/// ```
pub fn set_wake_callback(callback: impl Fn() + 'static) {
    WAKE_CALLBACK.with(|slot| *slot.borrow_mut() = Some(Rc::new(callback)));
}

/// The executor that runs reactive tasks only when the frame polls them.
struct FrameExecutor;

impl CustomExecutor for FrameExecutor {
    fn spawn(&self, future: PinnedFuture<()>) {
        spawn_local(future);
    }

    fn spawn_local(&self, future: PinnedLocalFuture<()>) {
        spawn_local(future);
    }

    fn poll_local(&self) {
        flush();
    }
}

/// A spawned reactive task, wrapped so that the runtime sees when it runs and
/// when it is woken.
struct Task {
    /// The task that the reactive graph spawned.
    inner: PinnedLocalFuture<()>,
    /// The waker this task's pool gave it, which re-queues the task.
    pool_waker: Option<Waker>,
    /// A waker that reports the wake before re-queueing through `pool_waker`.
    reporting_waker: Option<Waker>,
}

impl Future for Task {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        RAN.set(true);

        let this = self.get_mut();

        let pool_waker_changed = this
            .pool_waker
            .as_ref()
            .is_none_or(|pool_waker| !pool_waker.will_wake(cx.waker()));

        if pool_waker_changed {
            let pool_waker = cx.waker().clone();
            let reporting_waker = futures_task::waker(Arc::new(Forwarder {
                pool_waker: pool_waker.clone(),
            }));
            this.reporting_waker = Some(reporting_waker);
            this.pool_waker = Some(pool_waker);
        }

        let reporting_waker = this
            .reporting_waker
            .as_ref()
            .expect("a reporting waker is built whenever the pool's waker changes");
        let mut cx = Context::from_waker(reporting_waker);

        Pin::new(&mut this.inner).poll(&mut cx)
    }
}

/// Reports a wake to the runtime, then re-queues the task in its pool.
struct Forwarder {
    /// The waker that re-queues the wrapped task.
    pool_waker: Waker,
}

impl ArcWake for Forwarder {
    fn wake_by_ref(this: &Arc<Self>) {
        report_wake();
        this.pool_waker.wake_by_ref();
    }
}

/// Adds a task to the calling thread's pool.
fn spawn_local(future: PinnedLocalFuture<()>) {
    let task = Task {
        inner: future,
        pool_waker: None,
        reporting_waker: None,
    };
    let task: PinnedLocalFuture<()> = Box::pin(task);

    SPAWNER.with(|spawner| {
        spawner
            .spawn_local_obj(task.into())
            .expect("the task pool outlives every task spawned on it");
    });
}

/// Records that a task was woken and requests a redraw if the frame is idle.
fn report_wake() {
    if WOKEN.replace(true) {
        return;
    }

    if FLUSHING.get() {
        return;
    }

    let callback = WAKE_CALLBACK.with(|slot| slot.borrow().clone());
    if let Some(callback) = callback {
        callback();
    }
}
