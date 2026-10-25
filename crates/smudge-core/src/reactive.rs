//! The reactive runtime: signals, memos, effects and owners driven by the frame.
//!
//! The core is a small signal engine written for Smudge. It is single-threaded
//! and stores everything in one arena per thread. A signal write marks the
//! effects it reaches and queues them; the effects run when the frame calls
//! [`flush`], in the order they were woken, and writes made by an effect are
//! picked up by the same flush. Nothing runs between flushes.
//!
//! Every thread owns its own runtime, so effects on two threads never share
//! state. There is no executor to install and nothing to poll: call [`init`]
//! if you want the runtime to exist before the first signal, but creating a
//! signal does it too.
//!
//! A wake is reported to the callback set by [`set_wake_callback`], which is
//! how the platform learns that the next frame has work to do. The callback
//! belongs to the owner tree it was set under, so two apps on one thread each
//! get their own redraw requests. It runs on the first wake of its tree after
//! a flush and not again until the next flush ends. A write inside a flush
//! that wakes a tree the flush already serves asks for nothing, and one that
//! wakes another tree asks that tree for a redraw. A wake is reported on the
//! thread that writes the signal, so write signals only on the thread that
//! runs their effects.
//!
//! ## Semantics
//!
//! - An effect runs once synchronously when it is created, and again from a
//!   flush after a source it read changed.
//! - An effect re-tracks its sources on every run, so conditional reads work.
//! - A memo recomputes when read after a source changed, and stops
//!   propagation when its new value compares equal. Memos are therefore
//!   glitch-free: a reader never sees a memo that disagrees with its sources.
//! - Effects are not glitch-free across effect-to-effect writes: an effect
//!   that reads what another effect writes sees whichever value the wake
//!   order gives it. Put a memo in between when that matters.
//! - An effect that runs more than a fixed number of times in one flush is a
//!   cycle. Debug builds panic; release builds stop running it for that
//!   flush, so the frame finishes.
//! - An effect whose body panics stays subscribed and runs again on the next
//!   write to a source it reached.
//! - Cleaning up an owner stops its effects and disposes its signals, children
//!   first. Dropping the last handle to an owner cleans it up too, so mount
//!   and unmount cycles return all of their memory.
//!
//! The reactive API is defined here, so a widget imports it from one place.
//! Types are at the module root and the traits that give them their methods
//! are in [`prelude`].
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
//! let _effect = RenderEffect::new(move || recorder.set(count.get()));
//!
//! set_count.set(1);
//! assert_eq!(seen.get(), 0, "the write waits for the frame");
//!
//! reactive::flush();
//! assert_eq!(seen.get(), 1);
//! ```

use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

mod runtime;

use runtime::{CurrentOwnerGuard, EffectCell, MemoData, NodeId, OwnerData, OwnerState, SignalData};

/// Installs the runtime on the calling thread.
///
/// Creating a signal, memo, effect or owner does this implicitly, so calling
/// `init` is never required. It exists so a thread can state that it runs the
/// reactive runtime, and so older call sites keep working. Calling it again
/// does nothing.
///
/// # Examples
///
/// ```
/// use smudge_core::reactive;
///
/// reactive::init();
/// assert!(!reactive::flush(), "nothing was woken");
/// ```
pub fn init() {
    runtime::with_runtime(|_| {});
}

/// Runs every effect woken since the last call, in the order they were woken,
/// and returns whether any effect ran.
///
/// Call this once per frame, before reading the updates that effects
/// produced. A write made while the flush runs has its effect run in the same
/// call, so the return value covers everything that write set off.
///
/// Does nothing and returns `false` when called from inside a running flush,
/// because that flush already drains the queue.
///
/// # Examples
///
/// ```
/// use smudge_core::reactive::{self, Owner, RenderEffect, prelude::*};
///
/// let owner = Owner::new();
/// owner.set();
/// let (count, set_count) = reactive::signal_local(0u32);
/// let _effect = RenderEffect::new(move || count.get());
///
/// assert!(!reactive::flush(), "the creation run is not a flush");
/// set_count.set(1);
/// assert!(reactive::flush(), "the write woke the effect");
/// assert!(!reactive::flush(), "nothing was woken since");
/// ```
pub fn flush() -> bool {
    runtime::flush()
}

/// Sets the callback that a wake between flushes runs to request a redraw.
///
/// The callback is stored on the root scope of the current owner tree, which
/// is the app whose owner is current. When no owner is current it is stored on
/// the thread's default scope. Replaces any previous callback of that scope.
///
/// The callback runs on the first wake of its tree after a flush and not
/// again until the next flush ends, so a frame that writes many signals
/// requests one redraw. A wake that arrives during a flush asks for nothing
/// when the tree already has an effect in that flush, because the flush
/// applies the write itself.
///
/// A wake is reported on the thread that writes the signal, so write signals
/// only on the thread that runs their effects.
///
/// # Examples
///
/// ```
/// use smudge_core::reactive::{self, Owner, RenderEffect, prelude::*};
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// let owner = Owner::new();
/// owner.set();
/// let redraws = Rc::new(Cell::new(0u32));
/// let counter = Rc::clone(&redraws);
/// reactive::set_wake_callback(move || counter.set(counter.get() + 1));
///
/// let (count, set_count) = reactive::signal_local(0u32);
/// let _effect = RenderEffect::new(move || count.get());
/// reactive::flush();
///
/// set_count.set(1);
/// assert_eq!(redraws.get(), 1);
/// ```
pub fn set_wake_callback(callback: impl Fn() + 'static) {
    // The previous callback drops after the runtime is released, so its drop
    // may enter the runtime.
    let scope = runtime::with_runtime(|runtime| runtime.current_scope());
    scope.wake.replace(Some(Rc::new(callback)));
}

/// The traits and constructors that signals, memos and owners need.
///
/// Import this module with a glob to call `.get()`, `.set()` and `.update()`
/// on reactive values, to read them with or without tracking, and to create
/// signals and provide context.
pub mod prelude {
    pub use super::{Get, Read, Set, Update, With, provide_context, signal_local, use_context};
}

/// Applies a closure to a reactive value, tracking it as a source.
pub trait With<T: ?Sized + 'static> {
    /// Applies `fun` to the current value and returns its result.
    ///
    /// When an effect or a memo is running, the value becomes one of its
    /// sources and a later change wakes it.
    fn with<U>(&self, fun: impl FnOnce(&T) -> U) -> U;
}

/// Applies a closure to a reactive value without tracking it.
pub trait Read<T: ?Sized + 'static> {
    /// Applies `fun` to the current value and returns its result.
    ///
    /// No source is recorded, so a later change wakes nothing through this
    /// read.
    fn read<U>(&self, fun: impl FnOnce(&T) -> U) -> U;
}

/// Clones the value of a reactive value, tracking it as a source.
pub trait Get<T: Clone + 'static>: With<T> {
    /// Returns a clone of the current value.
    fn get(&self) -> T {
        self.with(T::clone)
    }
}

impl<S, T> Get<T> for S
where
    S: With<T> + ?Sized,
    T: Clone + 'static,
{
}

/// Updates the value of a reactive value in place and notifies subscribers.
pub trait Update<T: 'static> {
    /// Applies `fun` to the value and notifies subscribers that it changed,
    /// whether or not `fun` changed anything.
    ///
    /// # Panics
    ///
    /// Panics when the value is already mutably borrowed, which happens when
    /// `fun` reads the same value.
    fn update<U>(&self, fun: impl FnOnce(&mut T) -> U) -> U;
}

/// Replaces the value of a reactive value and notifies subscribers.
pub trait Set<T: 'static>: Update<T> {
    /// Replaces the value and notifies subscribers, whether or not the new
    /// value compares equal to the old one.
    fn set(&self, value: T) {
        self.update(|slot| *slot = value);
    }
}

impl<S, T> Set<T> for S
where
    S: Update<T> + ?Sized,
    T: 'static,
{
}

/// A signal that can be read and written.
///
/// Created with [`RwSignal::new`], or split into a [`ReadSignal`] and a
/// [`WriteSignal`] by [`signal_local`]. Cloning the handle is cheap, and
/// every clone names the same signal.
pub struct RwSignal<T> {
    /// The value and the node that carries the subscribers.
    data: Rc<SignalData<T>>,
}

impl<T: 'static> RwSignal<T> {
    /// Creates a signal holding `value`, under the current owner.
    ///
    /// The signal is disposed when that owner is cleaned up. Handles that
    /// outlive it keep the value readable but notify nothing. With no current
    /// owner nothing disposes it, and its entry in the runtime stays until the
    /// thread ends.
    pub fn new(value: T) -> RwSignal<T> {
        let node = runtime::with_runtime(runtime::Runtime::new_signal);
        RwSignal {
            data: Rc::new(SignalData {
                value: RefCell::new(value),
                node: Cell::new(node),
            }),
        }
    }
}

impl<T: 'static> Clone for RwSignal<T> {
    fn clone(&self) -> RwSignal<T> {
        RwSignal {
            data: Rc::clone(&self.data),
        }
    }
}

impl<T: 'static> Clone for ReadSignal<T> {
    fn clone(&self) -> ReadSignal<T> {
        ReadSignal {
            data: Rc::clone(&self.data),
        }
    }
}

impl<T: 'static> Clone for WriteSignal<T> {
    fn clone(&self) -> WriteSignal<T> {
        WriteSignal {
            data: Rc::clone(&self.data),
        }
    }
}

impl<T: 'static> Clone for Memo<T> {
    fn clone(&self) -> Memo<T> {
        Memo {
            data: Rc::clone(&self.data),
        }
    }
}

impl<T: 'static> With<T> for RwSignal<T> {
    fn with<U>(&self, fun: impl FnOnce(&T) -> U) -> U {
        runtime::track_read(self.data.node.get());
        fun(&self.data.value.borrow())
    }
}

impl<T: 'static> Read<T> for RwSignal<T> {
    fn read<U>(&self, fun: impl FnOnce(&T) -> U) -> U {
        fun(&self.data.value.borrow())
    }
}

impl<T: 'static> Update<T> for RwSignal<T> {
    fn update<U>(&self, fun: impl FnOnce(&mut T) -> U) -> U {
        let result = fun(&mut self.data.value.borrow_mut());
        let wake = runtime::with_runtime(|runtime| runtime.notify(self.data.node.get()));
        runtime::fire_wakes(wake);
        result
    }
}

/// The read half of a signal. Cloning the handle is cheap, and every clone
/// names the same value.
pub struct ReadSignal<T> {
    /// The value and the node that carries the subscribers.
    data: Rc<SignalData<T>>,
}

impl<T: 'static> With<T> for ReadSignal<T> {
    fn with<U>(&self, fun: impl FnOnce(&T) -> U) -> U {
        runtime::track_read(self.data.node.get());
        fun(&self.data.value.borrow())
    }
}

impl<T: 'static> Read<T> for ReadSignal<T> {
    fn read<U>(&self, fun: impl FnOnce(&T) -> U) -> U {
        fun(&self.data.value.borrow())
    }
}

/// The write half of a signal. Cloning the handle is cheap, and every clone
/// writes the same value.
pub struct WriteSignal<T> {
    /// The value and the node that carries the subscribers.
    data: Rc<SignalData<T>>,
}

impl<T: 'static> Update<T> for WriteSignal<T> {
    fn update<U>(&self, fun: impl FnOnce(&mut T) -> U) -> U {
        let result = fun(&mut self.data.value.borrow_mut());
        let wake = runtime::with_runtime(|runtime| runtime.notify(self.data.node.get()));
        runtime::fire_wakes(wake);
        result
    }
}

/// Creates a signal and returns its read and write halves.
///
/// The signal is disposed when the current owner is cleaned up. Handles that
/// outlive it keep the value readable but notify nothing. With no current
/// owner nothing disposes it, and its entry in the runtime stays until the
/// thread ends.
///
/// ```
/// use smudge_core::reactive::{self, Owner, RenderEffect, prelude::*};
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// let owner = Owner::new();
/// owner.set();
/// let (count, set_count) = signal_local(3u32);
/// let seen = Rc::new(Cell::new(0u32));
/// let recorder = Rc::clone(&seen);
/// let _effect = RenderEffect::new(move || recorder.set(count.get()));
/// reactive::flush();
///
/// set_count.update(|value| *value += 1);
/// reactive::flush();
/// assert_eq!(seen.get(), 4);
/// ```
pub fn signal_local<T: 'static>(value: T) -> (ReadSignal<T>, WriteSignal<T>) {
    let node = runtime::with_runtime(runtime::Runtime::new_signal);
    let data = Rc::new(SignalData {
        value: RefCell::new(value),
        node: Cell::new(node),
    });
    (
        ReadSignal {
            data: Rc::clone(&data),
        },
        WriteSignal { data },
    )
}

/// A value computed from signals, recomputed when a read needs it.
///
/// A memo recomputes when it is read after one of its sources changed, and it
/// stops propagation when the new value compares equal to the old one, so
/// effects that read it do not rerun. Because a memo is always brought up to
/// date on read, a reader never sees a memo that disagrees with its sources.
///
/// The memo is disposed when the current owner is cleaned up. Handles that
/// outlive it keep the last value readable. With no current owner nothing
/// disposes it, so its computation stays in the runtime until the thread
/// ends. Cloning the handle is cheap, and every clone names the same memo.
pub struct Memo<T> {
    /// The value and the node that carries the subscribers.
    data: Rc<MemoData<T>>,
}

impl<T: 'static> Memo<T> {
    /// Creates a memo that computes its value with `compute`.
    ///
    /// The first computation runs at once, tracking the signals it reads into
    /// the effect or memo that is running, if any.
    ///
    /// The value type needs [`PartialEq`] so the memo can recognize an
    /// unchanged value and stop propagation.
    pub fn new(compute: impl Fn() -> T + 'static) -> Memo<T>
    where
        T: PartialEq,
    {
        let data = Rc::new(MemoData {
            value: RefCell::new(None),
            node: Cell::new(NodeId::default()),
        });
        let stored = Rc::clone(&data);
        let run: Rc<dyn Fn()> = Rc::new(move || {
            let value = compute();
            let changed = stored
                .value
                .borrow()
                .as_ref()
                .is_none_or(|old| *old != value);
            if changed {
                *stored.value.borrow_mut() = Some(value);
                let wake = runtime::with_runtime(|runtime| runtime.notify(stored.node.get()));
                runtime::fire_wakes(wake);
            }
        });
        let node = runtime::with_runtime(|runtime| runtime.new_memo(run));
        data.node.set(node);
        runtime::compute_new_memo(node);
        Memo { data }
    }
}

impl<T: 'static> With<T> for Memo<T> {
    fn with<U>(&self, fun: impl FnOnce(&T) -> U) -> U {
        runtime::update_memo(self.data.node.get());
        runtime::track_read(self.data.node.get());
        let value = self.data.value.borrow();
        fun(value.as_ref().expect(
            "a memo holds a value once its first computation finished, so a \
             read sees one unless the memo reads itself",
        ))
    }
}

impl<T: 'static> Read<T> for Memo<T> {
    fn read<U>(&self, fun: impl FnOnce(&T) -> U) -> U {
        runtime::update_memo(self.data.node.get());
        let value = self.data.value.borrow();
        fun(value.as_ref().expect(
            "a memo holds a value once its first computation finished, so a \
             read sees one unless the memo reads itself",
        ))
    }
}

/// A function that reruns when a signal or memo it read changes.
///
/// The first run happens when the effect is created, with tracking, so the
/// sources it reaches are the ones that wake it later. Every run re-tracks
/// from scratch, which makes conditional reads work.
///
/// The effect is owned by the current owner: cleaning that owner up, or
/// dropping its last handle, stops the effect and frees its closure. Created
/// with no current owner, the effect lives until the returned handle drops.
/// Dropping the handle of an owned effect does nothing.
///
/// A write inside an effect body queues the effect again, and a body that
/// keeps waking itself is stopped at a fixed number of runs per flush:
/// debug builds panic, release builds skip the effect for the rest of that
/// flush.
pub struct RenderEffect {
    /// The node that carries the sources, the queue state and the body.
    node: NodeId,
    /// Whether an owner took responsibility for disposing the node.
    owned: bool,
}

impl RenderEffect {
    /// Creates an effect and runs its body once, with tracking.
    ///
    /// The effect is owned by the current owner, when there is one. See the
    /// type's documentation for the lifetime rules. The body's return value
    /// is ignored.
    pub fn new<T>(mut run: impl FnMut() -> T + 'static) -> RenderEffect {
        let owned = runtime::with_runtime(|runtime| runtime.current_owner().is_some());
        let scope = runtime::with_runtime(|runtime| runtime.current_scope());
        let cell = Rc::new(EffectCell {
            run: RefCell::new(Some(Box::new(move || {
                run();
            }))),
        });
        let node = runtime::with_runtime(|runtime| runtime.new_effect(Rc::clone(&cell), scope));
        let mut guard = runtime::DisposeOnUnwind {
            node,
            armed: !owned,
        };
        runtime::run_body(&cell, node);
        guard.armed = false;
        RenderEffect { node, owned }
    }
}

impl Drop for RenderEffect {
    fn drop(&mut self) {
        if !self.owned {
            runtime::dispose(self.node);
        }
    }
}

/// The owner of the reactive values and effects created while it is current.
///
/// Owners form a tree. [`Owner::new`] attaches to the current owner when there
/// is one, [`Owner::child`] attaches to an owner directly, and [`Owner::set`]
/// and [`Owner::with`] make one current, which is where new values and effects
/// are registered. [`Owner::cleanup`] stops the owner's effects and disposes
/// its values, children first; dropping the last handle to an owner does the
/// same, so nothing has to be cleaned up by hand.
///
/// Context values are provided on an owner with [`provide_context`] and read
/// by the owner and its descendants with [`use_context`].
#[derive(Clone)]
pub struct Owner {
    /// The parent, scope and contents of the owner.
    inner: Rc<OwnerData>,
}

impl Default for Owner {
    fn default() -> Owner {
        Owner::new()
    }
}

impl Owner {
    /// Creates an owner under the current owner, when there is one.
    ///
    /// With no current owner the new owner starts a new root scope, which
    /// carries its own wake callback.
    pub fn new() -> Owner {
        let parent = runtime::with_runtime(|runtime| runtime.current_owner());
        let scope = runtime::with_runtime(|runtime| runtime.owner_scope(parent.as_deref()));
        let inner = Rc::new(OwnerData {
            parent: parent.as_ref().map(Rc::downgrade),
            scope,
            cleaned: Cell::new(false),
            state: RefCell::new(OwnerState::default()),
        });
        if let Some(parent) = parent {
            parent.adopt(&inner);
        }
        Owner { inner }
    }

    /// Creates an owner under this one.
    ///
    /// The child inherits this owner's root scope, so its effects wake the
    /// same redraw requests.
    pub fn child(&self) -> Owner {
        let inner = Rc::new(OwnerData {
            parent: Some(Rc::downgrade(&self.inner)),
            scope: Rc::clone(&self.inner.scope),
            cleaned: Cell::new(false),
            state: RefCell::new(OwnerState::default()),
        });
        self.inner.adopt(&inner);
        Owner { inner }
    }

    /// Makes this owner current until another one is set or a `with` block
    /// restores the previous one.
    pub fn set(&self) {
        runtime::with_runtime(|runtime| runtime.set_current(&self.inner));
    }

    /// Runs `fun` with this owner current, restoring the previous one after.
    pub fn with<T>(&self, fun: impl FnOnce() -> T) -> T {
        let _guard = CurrentOwnerGuard::enter(&self.inner);
        fun()
    }

    /// Stops the owner's effects and disposes the values created under it,
    /// children first, then runs the cleanup functions that
    /// [`Owner::on_cleanup`] registered.
    ///
    /// Running it twice does nothing. Dropping the last handle to the owner
    /// runs it too.
    pub fn cleanup(&self) {
        self.inner.cleanup();
    }

    /// Sets the callback that a wake in this owner's tree runs to request a
    /// redraw.
    ///
    /// Every effect created under this owner reports its wake here, so two
    /// owners on one thread each get their own redraw requests. See
    /// [`set_wake_callback`] for when the callback runs.
    pub fn set_wake_callback(&self, callback: impl Fn() + 'static) {
        self.inner.scope.wake.replace(Some(Rc::new(callback)));
    }

    /// Registers `fun` to run when the current owner is cleaned up.
    ///
    /// With no current owner, `fun` is dropped.
    pub fn on_cleanup(fun: impl FnOnce() + 'static) {
        let Some(owner) = Owner::current() else {
            return;
        };
        owner.state.borrow_mut().cleanups.push(Box::new(fun));
    }

    /// Returns the current owner, when one is set and still alive.
    fn current() -> Option<Rc<OwnerData>> {
        runtime::with_runtime(|runtime| runtime.current_owner())
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.cleanup();
        }
    }
}

/// Stores `value` on the current owner for [`use_context`] to read.
///
/// The value is visible to the owner it was provided on and to everything
/// created under it, and invisible to its siblings and ancestors. A second
/// value of the same type replaces the first.
///
/// With no current owner, `value` is dropped.
pub fn provide_context<T: 'static>(value: T) {
    let Some(owner) = Owner::current() else {
        return;
    };
    let key = TypeId::of::<T>();
    let value: Rc<dyn std::any::Any> = Rc::new(value);
    let mut state = owner.state.borrow_mut();
    match state
        .contexts
        .iter_mut()
        .find(|(existing, _)| *existing == key)
    {
        Some((_, slot)) => *slot = value,
        None => state.contexts.push((key, value)),
    }
}

/// Returns a clone of the context value of type `T`, provided on the current
/// owner or the closest ancestor that provided one.
///
/// Returns `None` when neither has one.
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    let key = TypeId::of::<T>();
    let mut current = Owner::current();
    while let Some(owner) = current {
        let found = owner
            .state
            .borrow()
            .contexts
            .iter()
            .find(|(existing, _)| *existing == key)
            .map(|(_, value)| Rc::clone(value));
        if let Some(value) = found
            && let Ok(value) = value.downcast::<T>()
        {
            return Some(T::clone(&value));
        }
        current = owner.parent.as_ref().and_then(Weak::upgrade);
    }
    None
}
