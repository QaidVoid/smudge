//! Frame-driven execution of reactive effects.
//!
//! Signals and effects come from the reactive graph crate. An effect is a
//! task, and a task only runs when something polls the executor it was
//! spawned on. This module installs an executor whose tasks are polled by the
//! frame, so a signal write wakes an effect and the effect runs at the next
//! [`poll_local`] call rather than immediately.
//!
//! Every thread owns its own task pool. The executor installed in the process
//! is global, but it dispatches to the pool of the thread that calls it, so
//! effects on two threads never share a queue. Call [`init`] on each thread
//! that creates effects.

use any_spawner::{CustomExecutor, Executor, PinnedFuture, PinnedLocalFuture};
use futures_executor::{LocalPool, LocalSpawner};
use futures_task::LocalSpawn;
use std::cell::{Cell, RefCell};
use std::sync::Mutex;

thread_local! {
    static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
    static SPAWNER: LocalSpawner = POOL.with(|pool| pool.borrow().spawner());
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
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
/// reactive::poll_local();
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
/// progress.
///
/// Call this once per frame, before reading the updates that effects produced.
/// A task woken by a write made while the pool is running also runs in this
/// call.
///
/// Does nothing when called from inside a task that this pool is running,
/// because that task already runs until the pool stalls.
pub fn poll_local() {
    POOL.with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            pool.run_until_stalled();
        }
    });
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
        poll_local();
    }
}

/// Adds a task to the calling thread's pool.
fn spawn_local(future: PinnedLocalFuture<()>) {
    SPAWNER.with(|spawner| {
        spawner
            .spawn_local_obj(future.into())
            .expect("the task pool outlives every task spawned on it");
    });
}
