//! The signal core: one thread-local runtime holding every reactive node.
//!
//! This module is crate-private. The public types in the parent module are
//! thin handles over what is defined here. A node in the arena holds only
//! bookkeeping, and the typed value of a signal or a memo lives in the handle
//! that names it, so a steady write and flush allocates nothing once the
//! arena and its lists have grown.
//!
//! No closure is ever called while the runtime is borrowed. Every step that
//! touches the runtime is a short re-entrant call, so an effect body can
//! write signals and read memos freely.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use slotmap::{SlotMap, new_key_type};

use crate::reactive::Owner;

new_key_type! {
    /// Identifies one node of the arena: a signal, a memo or an effect.
    ///
    /// The key is generational, so an id whose node was disposed never names
    /// another node. Every access checks liveness first.
    pub(crate) struct NodeId;
}

/// How many times one effect may run in one flush before it counts as a cycle.
const MAX_RUNS_PER_FLUSH: u32 = 16;

/// The wake callback of one root owner tree.
///
/// Every effect records the scope it was created under, so a write wakes the
/// redraw request of the app whose tree it belongs to and not another app's.
pub(crate) struct Scope {
    /// The callback that the first wake between flushes runs.
    pub(crate) wake: RefCell<Option<Rc<dyn Fn()>>>,
    /// Whether the callback ran since the last flush ended.
    ///
    /// Kept per scope, so one app's pending redraw never hides another app's
    /// wake.
    woken: Cell<bool>,
    /// Whether wakes go unreported, because the app this scope belongs to is
    /// running a frame that flushes until no effect is left.
    pub(crate) quiet: Cell<bool>,
}

impl Scope {
    /// Creates an empty scope.
    pub(crate) fn new() -> Scope {
        Scope {
            wake: RefCell::new(None),
            woken: Cell::new(false),
            quiet: Cell::new(false),
        }
    }
}

/// The fewest links an owner holds before it sweeps, so an owner with a few
/// children never sweeps.
const MIN_SWEEP: usize = 8;

/// What an owner holds.
#[derive(Default)]
pub(crate) struct OwnerState {
    /// The owners created under this one.
    pub(crate) children: Vec<Weak<OwnerData>>,
    /// The functions registered to run at cleanup.
    pub(crate) cleanups: Vec<Box<dyn FnOnce()>>,
    /// The nodes created under this owner, disposed at cleanup.
    pub(crate) nodes: Vec<NodeId>,
    /// The context values, one per type.
    pub(crate) contexts: Vec<(TypeId, Rc<dyn Any>)>,
}

/// The data of one owner: its parent, its scope and everything it holds.
pub(crate) struct OwnerData {
    /// The owner this one was created under, if any.
    pub(crate) parent: Option<Weak<OwnerData>>,
    /// The root scope this owner tree belongs to.
    pub(crate) scope: Rc<Scope>,
    /// Whether [`cleanup`](OwnerData::cleanup) already ran.
    pub(crate) cleaned: Cell<bool>,
    /// What the owner holds, taken out while cleanup runs user code.
    pub(crate) state: RefCell<OwnerState>,
    /// How many children were freed since the last sweep, which is at least
    /// the number of dead links in `state.children`. A child counts itself
    /// here when it is freed, so the count is a cell that needs no borrow of
    /// `state`. It can run ahead of the dead links, for a child freed after
    /// a cleanup took the list, which only makes a sweep come early.
    pub(crate) dead_children: Cell<usize>,
    /// The timer book of the app this owner tree belongs to, set on the root
    /// owner when the app is created. A timer registers into the book of the
    /// root its owner chain leads to.
    pub(crate) timer_book: RefCell<Option<Weak<RefCell<crate::timer::TimerBook>>>>,
    /// The clipboard of the app this owner tree belongs to, set on the root
    /// owner when the app installs one. A handler reaches the clipboard of
    /// the root its owner chain leads to.
    pub(crate) clipboard: RefCell<Option<crate::clipboard::WeakSlot>>,
    /// The channel book of the app this owner tree belongs to, set on the
    /// root owner when the app is created. A channel registers into the book
    /// of the root its owner chain leads to.
    pub(crate) channel_book: RefCell<Option<Weak<RefCell<crate::channel::ChannelBook>>>>,
    /// The window book of the app this owner tree belongs to, set on the
    /// root owner when the app is created. A window command queues into the
    /// book of the root its owner chain leads to.
    pub(crate) window_book: RefCell<Option<Weak<RefCell<crate::window::WindowBook>>>>,
}

impl OwnerData {
    /// Links `child` under this owner, so cleaning this owner up cleans the
    /// child up first.
    ///
    /// Dead links keep their owner's allocation alive, so the list is swept
    /// of them, but only once the dead count reaches half its length. A sweep
    /// then walks at most twice as many links as children died since the
    /// last one, and every child dies once, so adopting costs constant time
    /// on average, where sweeping on
    /// every adopt made mounting many children under one owner quadratic.
    /// The list holds at most about twice as many links as live children,
    /// plus a few, however many children came and went before.
    pub(crate) fn adopt(&self, child: &Rc<OwnerData>) {
        let mut state = self.state.borrow_mut();
        let links = state.children.len();
        if links >= MIN_SWEEP && self.dead_children.get() * 2 >= links {
            state.children.retain(|child| child.strong_count() > 0);
            self.dead_children.set(0);
        }
        state.children.push(Rc::downgrade(child));
    }

    /// Stops the owner's effects and disposes its nodes, children first.
    ///
    /// Running it twice does nothing. Cleanup functions run after the owner is
    /// marked cleaned, so a cleanup that registers another cleanup on the same
    /// owner registers it on an owner that will not run it again.
    pub(crate) fn cleanup(self: &Rc<Self>) {
        if self.cleaned.get() {
            return;
        }
        self.cleaned.set(true);
        let state = std::mem::take(&mut *self.state.borrow_mut());
        self.dead_children.set(0);
        for child in &state.children {
            if let Some(child) = child.upgrade() {
                child.cleanup();
            }
        }
        for cleanup in state.cleanups {
            cleanup();
        }
        for node in state.nodes {
            dispose(node);
        }
    }

    /// Creates an owner under this one and returns it as a handle.
    ///
    /// The child inherits this owner's root scope, so its effects wake the
    /// same redraw requests.
    pub(crate) fn child(self: &Rc<Self>) -> Owner {
        let inner = Rc::new(OwnerData {
            parent: Some(Rc::downgrade(self)),
            scope: Rc::clone(&self.scope),
            cleaned: Cell::new(false),
            state: RefCell::new(OwnerState::default()),
            dead_children: Cell::new(0),
            timer_book: RefCell::new(None),
            clipboard: RefCell::new(None),
            channel_book: RefCell::new(None),
            window_book: RefCell::new(None),
        });
        self.adopt(&inner);
        Owner { inner }
    }
}

impl Drop for OwnerData {
    /// Counts this owner as a dead link on its parent, so the parent's next
    /// adopt knows when sweeping its links pays off.
    fn drop(&mut self) {
        if let Some(parent) = self.parent.as_ref().and_then(Weak::upgrade) {
            parent.dead_children.set(parent.dead_children.get() + 1);
        }
    }
}

/// The typed value of one signal, shared by every handle that names it.
pub(crate) struct SignalData<T> {
    /// The current value.
    pub(crate) value: RefCell<T>,
    /// The node that carries the version and the subscribers.
    pub(crate) node: Cell<NodeId>,
}

/// The typed value of one memo, shared by every handle that names it.
pub(crate) struct MemoData<T> {
    /// The computed value, `None` until the first computation ran.
    pub(crate) value: RefCell<Option<T>>,
    /// The node that carries the version and the subscribers.
    pub(crate) node: Cell<NodeId>,
}

/// The body of one effect, shared between the arena node and the handle.
pub(crate) struct EffectCell {
    /// The user's body. Never borrowed while it runs, so the body can enter
    /// the runtime freely.
    pub(crate) run: RefCell<Option<Box<dyn FnMut()>>>,
}

/// One node of the arena.
pub(crate) enum Node {
    /// A value that notifies its subscribers when it changes.
    Signal {
        /// The memos and effects that read this signal.
        subscribers: Vec<NodeId>,
        /// Bumped on every write, so subscribers compare versions.
        version: u32,
    },
    /// A value computed from other nodes and recomputed when a read needs it.
    Memo {
        /// Computes the value into the typed handle and notifies on change.
        run: Option<Rc<dyn Fn()>>,
        /// The memos and effects that read this memo.
        subscribers: Vec<NodeId>,
        /// The nodes read at the last computation, with the versions seen.
        sources: Vec<(NodeId, u32)>,
        /// How far behind the value is.
        state: MemoState,
        /// Bumped when a recomputation changes the value.
        version: u32,
        /// The owner that was current when the memo was created. Only
        /// [`observer_owner`] reads it; a recomputation does not make it
        /// current.
        owner: Option<Weak<OwnerData>>,
    },
    /// A function that reruns when a source it read changes.
    Effect {
        /// The body, shared with the handle.
        cell: Rc<EffectCell>,
        /// The nodes read at the last run, with the versions seen.
        sources: Vec<(NodeId, u32)>,
        /// Whether the effect waits in the flush queue.
        queued: bool,
        /// How many times the effect ran in the flush `generation`.
        runs: u32,
        /// The flush the run counter belongs to.
        generation: u64,
        /// The root scope a wake of this effect reports to.
        scope: Rc<Scope>,
        /// The owner that was current when the effect was created. Only
        /// [`observer_owner`] reads it; a rerun does not make it current, so
        /// what a rerun creates is not adopted by it.
        owner: Option<Weak<OwnerData>>,
    },
}

impl Node {
    /// The subscriber list of a node that others can subscribe to.
    fn subscribers_mut(&mut self) -> Option<&mut Vec<NodeId>> {
        match self {
            Node::Signal { subscribers, .. } | Node::Memo { subscribers, .. } => Some(subscribers),
            Node::Effect { .. } => None,
        }
    }

    /// The subscriber list of a node that others can subscribe to.
    fn subscribers(&self) -> Option<&Vec<NodeId>> {
        match self {
            Node::Signal { subscribers, .. } | Node::Memo { subscribers, .. } => Some(subscribers),
            Node::Effect { .. } => None,
        }
    }

    /// The source list of a node that reads others.
    fn sources_mut(&mut self) -> Option<&mut Vec<(NodeId, u32)>> {
        match self {
            Node::Memo { sources, .. } | Node::Effect { sources, .. } => Some(sources),
            Node::Signal { .. } => None,
        }
    }

    /// The source list of a node that reads others.
    fn sources(&self) -> Option<&Vec<(NodeId, u32)>> {
        match self {
            Node::Memo { sources, .. } | Node::Effect { sources, .. } => Some(sources),
            Node::Signal { .. } => None,
        }
    }

    /// The version of a node that others compare against.
    fn version(&self) -> Option<u32> {
        match self {
            Node::Signal { version, .. } | Node::Memo { version, .. } => Some(*version),
            Node::Effect { .. } => None,
        }
    }

    /// Removes one subscriber, keeping the list's allocation.
    fn remove_subscriber(&mut self, id: NodeId) {
        if let Some(subscribers) = self.subscribers_mut()
            && let Some(position) = subscribers.iter().position(|sub| *sub == id)
        {
            subscribers.swap_remove(position);
        }
    }
}

/// How far behind a memo's value is.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum MemoState {
    /// The value is current.
    Clean,
    /// A source changed directly, so the memo must recompute.
    Dirty,
    /// A source may have changed, so the memo must check before it reads.
    Check,
}

/// The per-thread runtime: the arena, the flush queue and the current owner.
pub(crate) struct Runtime {
    /// Every node of this thread.
    pub(crate) arena: SlotMap<NodeId, Node>,
    /// The effects woken since the last flush started draining.
    queue: VecDeque<NodeId>,
    /// Whether a flush is draining the queue.
    flushing: bool,
    /// Whether any effect ran in the current flush.
    ran: bool,
    /// The scopes woken since the last flush ended, cleared when it ends.
    woken: Vec<Rc<Scope>>,
    /// How many flushes this thread has started.
    generation: u64,
    /// The owner that `Owner::new` and the context functions see.
    current: Weak<OwnerData>,
    /// The scope of nodes and owners created with no current owner.
    default_scope: Rc<Scope>,
}

thread_local! {
    /// The runtime of this thread.
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
    /// The effect or memo whose run tracks reads right now.
    static OBSERVER: Cell<Option<NodeId>> = const { Cell::new(None) };
}

/// Runs `f` with the thread's runtime.
///
/// This is private to the module on purpose. The rest of the crate reaches
/// the runtime through the functions below it and the other `pub(crate)`
/// functions of this module, which are neither generic nor inlined. Generic
/// code compiled in an application crate therefore reaches the runtime by
/// calling into this crate, and never addresses the runtime's thread-locals
/// itself. A hot patch is built from the application's crates, and a patch
/// that addressed a thread-local would carry a fresh, empty copy of it
/// instead of the running runtime. `tests/consumers/runtime-entry` checks the
/// rule on the IR of an application crate.
///
/// Inside this module the accessor inlines, so the flush loop makes no calls
/// to reach the runtime. A variant that sent every access through one
/// non-inlined function made writing 10,000 signals and flushing their
/// effects about twice as slow in the `reactive` benchmark. The entry points
/// cost about 4 percent more instructions per write and flush.
fn with_runtime<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RUNTIME.with_borrow_mut(f)
}

/// Creates the thread's runtime, when it does not exist yet.
#[inline(never)]
pub(crate) fn init() {
    with_runtime(|_| {});
}

/// Returns the current owner, when one is set and still alive.
#[inline(never)]
pub(crate) fn current_owner() -> Option<Rc<OwnerData>> {
    with_runtime(|runtime| runtime.current_owner())
}

/// Makes `owner` the current one, until another replaces it.
#[inline(never)]
pub(crate) fn set_current(owner: &Rc<OwnerData>) {
    with_runtime(|runtime| runtime.set_current(owner));
}

/// Returns the scope new nodes and owners belong to: the current owner's
/// scope, or the default one when no owner is current.
#[inline(never)]
pub(crate) fn current_scope() -> Rc<Scope> {
    with_runtime(|runtime| runtime.current_scope())
}

/// Returns the scope a new owner under `parent` belongs to, or a fresh root
/// scope when `parent` is `None`.
#[inline(never)]
pub(crate) fn owner_scope(parent: Option<&OwnerData>) -> Rc<Scope> {
    with_runtime(|runtime| runtime.owner_scope(parent))
}

/// Creates a signal node under the current owner.
#[inline(never)]
pub(crate) fn new_signal() -> NodeId {
    with_runtime(Runtime::new_signal)
}

/// Creates a memo node that computes with `run`, under the current owner.
#[inline(never)]
pub(crate) fn new_memo(run: Rc<dyn Fn()>) -> NodeId {
    with_runtime(|runtime| runtime.new_memo(run))
}

/// Creates an effect node that runs `cell` and wakes `scope`, under the
/// current owner.
#[inline(never)]
pub(crate) fn new_effect(cell: Rc<EffectCell>, scope: Rc<Scope>) -> NodeId {
    with_runtime(|runtime| runtime.new_effect(cell, scope))
}

/// Marks the node `id` as changed, wakes its subscribers and runs the wake
/// callbacks of the scopes that were asleep.
#[inline(never)]
pub(crate) fn notify(id: NodeId) {
    let wakes = with_runtime(|runtime| runtime.notify(id));
    fire_wakes(wakes);
}

impl Runtime {
    fn new() -> Runtime {
        Runtime {
            arena: SlotMap::with_key(),
            queue: VecDeque::new(),
            flushing: false,
            ran: false,
            woken: Vec::new(),
            generation: 0,
            current: Weak::default(),
            default_scope: Rc::new(Scope::new()),
        }
    }

    /// Returns the node, or `None` when the id was disposed.
    fn node(&self, id: NodeId) -> Option<&Node> {
        self.arena.get(id)
    }

    /// Returns the node's version, or `None` when it was disposed.
    fn version_of(&self, id: NodeId) -> Option<u32> {
        self.node(id).and_then(Node::version)
    }

    /// Returns whether the node is a memo.
    fn is_memo(&self, id: NodeId) -> bool {
        matches!(self.node(id), Some(Node::Memo { .. }))
    }

    /// Returns the current owner, when one is set and still alive.
    pub(crate) fn current_owner(&self) -> Option<Rc<OwnerData>> {
        self.current.upgrade()
    }

    /// Replaces the current owner. The previous one is restored by
    /// [`CurrentOwnerGuard`] when a `with` block ends.
    pub(crate) fn set_current(&mut self, owner: &Rc<OwnerData>) {
        self.current = Rc::downgrade(owner);
    }

    /// Returns the scope new nodes and owners belong to: the current owner's
    /// scope, or the default one when no owner is current.
    pub(crate) fn current_scope(&self) -> Rc<Scope> {
        self.current_owner().map_or_else(
            || Rc::clone(&self.default_scope),
            |owner| Rc::clone(&owner.scope),
        )
    }

    /// Returns the scope a new owner belongs to: its parent's, or a fresh
    /// root scope with its own wake callback when it starts a new tree.
    pub(crate) fn owner_scope(&self, parent: Option<&OwnerData>) -> Rc<Scope> {
        match parent {
            Some(parent) => Rc::clone(&parent.scope),
            None => Rc::new(Scope::new()),
        }
    }

    /// Restores the current owner that a `with` block replaced.
    pub(crate) fn restore_current_owner(&mut self, previous: Option<Weak<OwnerData>>) {
        self.current = previous.unwrap_or_default();
    }

    /// Registers a fresh node under the current owner, if there is one.
    fn register_node(&mut self, id: NodeId) {
        let Some(owner) = self.current_owner() else {
            return;
        };
        owner.state.borrow_mut().nodes.push(id);
    }

    /// Creates a signal node and returns its id.
    pub(crate) fn new_signal(&mut self) -> NodeId {
        let id = self.arena.insert(Node::Signal {
            subscribers: Vec::new(),
            version: 0,
        });
        self.register_node(id);
        id
    }

    /// Creates a memo node and returns its id.
    pub(crate) fn new_memo(&mut self, run: Rc<dyn Fn()>) -> NodeId {
        let id = self.arena.insert(Node::Memo {
            run: Some(run),
            subscribers: Vec::new(),
            sources: Vec::new(),
            state: MemoState::Dirty,
            version: 0,
            owner: self.current.upgrade().as_ref().map(Rc::downgrade),
        });
        self.register_node(id);
        id
    }

    /// Creates an effect node and returns its id.
    pub(crate) fn new_effect(&mut self, cell: Rc<EffectCell>, scope: Rc<Scope>) -> NodeId {
        let owner = self.current.upgrade().as_ref().map(Rc::downgrade);
        let id = self.arena.insert(Node::Effect {
            cell,
            sources: Vec::new(),
            queued: false,
            runs: 0,
            generation: u64::MAX,
            scope,
            owner,
        });
        self.register_node(id);
        id
    }

    /// Removes a node and unlinks its edges, returning the node. A stale id
    /// does nothing.
    ///
    /// Call it through [`dispose`], which drops the node after the runtime is
    /// released.
    fn dispose_node(&mut self, id: NodeId) -> Option<Node> {
        let count = self.node(id).and_then(Node::sources).map_or(0, Vec::len);
        for index in 0..count {
            let Some(source) = self
                .node(id)
                .and_then(Node::sources)
                .and_then(|sources| sources.get(index))
                .map(|entry| entry.0)
            else {
                break;
            };
            if let Some(node) = self.arena.get_mut(source) {
                node.remove_subscriber(id);
            }
        }
        self.arena.remove(id)
    }

    /// Bumps the version of a signal or memo and marks its subscribers.
    ///
    /// Returns where the scopes this write woke start in the woken list, for
    /// [`fire_wakes`] to run their callbacks once the runtime is released.
    pub(crate) fn notify(&mut self, id: NodeId) -> Wakes {
        let wakes = Wakes(self.woken.len());
        let count = {
            let Some(node) = self.arena.get_mut(id) else {
                return wakes;
            };
            match node {
                Node::Signal { version, .. } | Node::Memo { version, .. } => {
                    *version = version.wrapping_add(1);
                    node.subscribers().map_or(0, Vec::len)
                }
                Node::Effect { .. } => return wakes,
            }
        };
        for index in 0..count {
            let subscriber = {
                let Some(subscribers) = self.node(id).and_then(Node::subscribers) else {
                    break;
                };
                match subscribers.get(index) {
                    Some(subscriber) => *subscriber,
                    None => break,
                }
            };
            self.mark_dirty(subscriber);
        }
        wakes
    }

    /// Marks a node whose source certainly changed.
    ///
    /// A memo must recompute when next read, and its own subscribers may
    /// change, so they are marked in turn. An effect is queued.
    fn mark_dirty(&mut self, id: NodeId) {
        let propagated = match self.arena.get_mut(id) {
            Some(Node::Memo { state, .. }) => {
                let fresh = *state == MemoState::Clean;
                *state = MemoState::Dirty;
                fresh
            }
            Some(Node::Effect { .. }) => false,
            _ => return,
        };
        if propagated {
            self.mark_subscribers(id);
        } else {
            self.queue_effect(id);
        }
    }

    /// Marks a node whose source may have changed.
    ///
    /// A memo that is already behind stays as it is, because it will check its
    /// sources anyway. A clean memo is set to check and its subscribers are
    /// marked in turn. An effect is queued: it compares versions when it runs.
    fn mark_check(&mut self, id: NodeId) {
        let propagated = match self.arena.get_mut(id) {
            Some(Node::Memo { state, .. }) => {
                let fresh = *state == MemoState::Clean;
                if fresh {
                    *state = MemoState::Check;
                }
                fresh
            }
            Some(Node::Effect { .. }) => false,
            _ => return,
        };
        if propagated {
            self.mark_subscribers(id);
        } else {
            self.queue_effect(id);
        }
    }

    /// Marks every subscriber of `id` as maybe changed.
    fn mark_subscribers(&mut self, id: NodeId) {
        let count = self
            .node(id)
            .and_then(Node::subscribers)
            .map_or(0, Vec::len);
        for index in 0..count {
            let subscriber = {
                let Some(subscribers) = self.node(id).and_then(Node::subscribers) else {
                    break;
                };
                match subscribers.get(index) {
                    Some(subscriber) => *subscriber,
                    None => break,
                }
            };
            self.mark_check(subscriber);
        }
    }

    /// Queues an effect for the next flush, once, and reports the wake when
    /// the effect's scope was not woken since the last flush ended.
    ///
    /// Every effect a flush runs woke its scope when it was queued, so a write
    /// made inside a flush stays silent for the scopes that flush already
    /// serves and still wakes a scope it does not, whose app needs a frame to
    /// apply what the effect produced.
    fn queue_effect(&mut self, id: NodeId) {
        let Some(Node::Effect { queued, scope, .. }) = self.arena.get_mut(id) else {
            return;
        };
        // An effect that an unwound flush left queued has a scope whose wake
        // was cleared, so the wake is checked even when the effect is queued.
        // A quiet scope leaves its wake state alone, so the first write after
        // its frame still asks for a redraw.
        let first_wake =
            (!scope.quiet.get() && !scope.woken.replace(true)).then(|| Rc::clone(scope));
        if !*queued {
            *queued = true;
            self.queue.push_back(id);
        }
        if let Some(scope) = first_wake {
            self.woken.push(scope);
        }
    }

    /// Returns how far behind the memo is.
    fn memo_state(&self, id: NodeId) -> MemoState {
        match self.node(id) {
            Some(Node::Memo { state, .. }) => *state,
            _ => MemoState::Clean,
        }
    }

    /// Returns the reader's source count.
    fn source_count(&self, id: NodeId) -> usize {
        self.node(id).and_then(Node::sources).map_or(0, Vec::len)
    }

    /// Returns the reader's source at `index`, with the version last seen.
    fn source_at(&self, id: NodeId, index: usize) -> Option<(NodeId, u32)> {
        let sources = self.node(id).and_then(Node::sources)?;
        sources.get(index).copied()
    }

    /// Marks the memo clean without recomputing, because no source changed.
    fn memo_set_clean(&mut self, id: NodeId) {
        if let Some(Node::Memo { state, .. }) = self.arena.get_mut(id) {
            *state = MemoState::Clean;
        }
    }

    /// Returns the memo's computation, if the memo is still live.
    fn memo_run(&self, id: NodeId) -> Option<Rc<dyn Fn()>> {
        match self.node(id) {
            Some(Node::Memo { run, .. }) => run.clone(),
            _ => None,
        }
    }

    /// Prepares a memo for recomputation: it unsubscribes from its old
    /// sources and counts as clean while it runs.
    fn begin_memo(&mut self, id: NodeId) {
        if let Some(Node::Memo { state, .. }) = self.arena.get_mut(id) {
            *state = MemoState::Clean;
        }
        self.clear_sources(id);
    }

    /// Counts one effect run against this flush's limit. Returns whether the
    /// effect may run.
    ///
    /// The effect stays marked as queued while it pulls its memos up to date,
    /// so a memo that changes during the pull does not queue it a second
    /// time. [`Runtime::release_effect`] clears the mark before the body runs.
    fn begin_effect(&mut self, id: NodeId) -> bool {
        let runs = {
            let Some(Node::Effect {
                queued,
                runs,
                generation,
                ..
            }) = self.arena.get_mut(id)
            else {
                return false;
            };
            if !*queued {
                return false;
            }
            if *generation != self.generation {
                *generation = self.generation;
                *runs = 1;
            } else {
                *runs += 1;
            }
            *runs
        };
        if runs > MAX_RUNS_PER_FLUSH {
            self.release_effect(id);
            debug_assert!(
                false,
                "a binding ran {runs} times in one flush, so it is part of a \
                 cycle: it writes a signal that wakes it again, directly or \
                 through another binding"
            );
            return false;
        }
        true
    }

    /// Clears the queued mark of an effect taken off the queue, so a later
    /// write can queue it again.
    fn release_effect(&mut self, id: NodeId) {
        if let Some(Node::Effect { queued, .. }) = self.arena.get_mut(id) {
            *queued = false;
        }
    }

    /// Returns the effect's body, if the effect is still live.
    fn effect_cell(&self, id: NodeId) -> Option<Rc<EffectCell>> {
        match self.node(id) {
            Some(Node::Effect { cell, .. }) => Some(Rc::clone(cell)),
            _ => None,
        }
    }

    /// Returns the live owner that was current when the memo or effect `id`
    /// was created.
    fn creation_owner(&self, id: NodeId) -> Option<Rc<OwnerData>> {
        match self.node(id) {
            Some(Node::Effect { owner, .. } | Node::Memo { owner, .. }) => owner
                .as_ref()
                .and_then(Weak::upgrade)
                .filter(|owner| !owner.cleaned.get()),
            _ => None,
        }
    }

    /// Unsubscribes the reader from its old sources, keeping the list's
    /// allocation, so the run ahead records the sources it reaches.
    fn clear_sources(&mut self, id: NodeId) {
        let count = self.source_count(id);
        for index in 0..count {
            let Some((source, _)) = self.source_at(id, index) else {
                break;
            };
            if let Some(node) = self.arena.get_mut(source) {
                node.remove_subscriber(id);
            }
        }
        if let Some(node) = self.arena.get_mut(id)
            && let Some(sources) = node.sources_mut()
        {
            sources.clear();
        }
    }

    /// Records that `reader` read `source` at `version`, linking the two nodes
    /// when they are not linked yet.
    pub(crate) fn track(&mut self, reader: NodeId, source: NodeId, version: u32) {
        let already = match self.node(reader) {
            Some(node) => node
                .sources()
                .is_some_and(|sources| sources.iter().any(|(seen, _)| *seen == source)),
            None => return,
        };
        if already {
            return;
        }
        if let Some(node) = self.arena.get_mut(source)
            && let Some(subscribers) = node.subscribers_mut()
        {
            subscribers.push(reader);
        }
        if let Some(node) = self.arena.get_mut(reader)
            && let Some(sources) = node.sources_mut()
        {
            sources.push((source, version));
        }
    }

    /// Returns whether the reader's source at `index` moved since it was read.
    ///
    /// A disposed source never counts as moved.
    fn source_changed(&self, id: NodeId, index: usize) -> bool {
        self.source_at(id, index).is_some_and(|(source, seen)| {
            self.version_of(source)
                .is_some_and(|version| version != seen)
        })
    }

    /// Pops the next effect to run, if the queue holds one.
    fn pop_effect(&mut self) -> Option<NodeId> {
        self.queue.pop_front()
    }

    /// Reports that an effect was processed by the current flush.
    fn set_ran(&mut self) {
        self.ran = true;
    }

    /// Starts a flush, unless one is already running.
    fn begin_flush(&mut self) -> bool {
        if self.flushing {
            return false;
        }
        self.flushing = true;
        self.ran = false;
        self.generation += 1;
        true
    }

    /// Returns whether any effect ran in the current flush.
    fn flush_ran(&self) -> bool {
        self.ran
    }

    /// Ends a flush, or cleans up after one that unwound.
    ///
    /// The wake state is cleared, so the next write between frames requests a
    /// redraw. A flush that unwound leaves the effects it did not reach in the
    /// queue, so they still run, in the next flush.
    fn end_flush(&mut self) {
        self.flushing = false;
        for scope in self.woken.drain(..) {
            scope.woken.set(false);
        }
    }
}

/// Guards the observer while a memo or an effect runs, restoring the previous
/// one even when the body unwinds.
struct ObserverGuard {
    previous: Option<NodeId>,
}

impl ObserverGuard {
    /// Makes `observer` the node that reads are tracked into.
    fn enter(observer: Option<NodeId>) -> ObserverGuard {
        let previous = OBSERVER.with(|cell| cell.replace(observer));
        ObserverGuard { previous }
    }
}

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        OBSERVER.with(|cell| cell.replace(self.previous));
    }
}

/// Guards the current owner while an `Owner::with` block runs.
pub(crate) struct CurrentOwnerGuard {
    previous: Option<Weak<OwnerData>>,
}

impl CurrentOwnerGuard {
    /// Makes `owner` the current one for the block the guard lives in.
    #[inline(never)]
    pub(crate) fn enter(owner: &Rc<OwnerData>) -> CurrentOwnerGuard {
        let previous = with_runtime(|runtime| {
            let previous = runtime
                .current_owner()
                .map(|current| Rc::downgrade(&current));
            runtime.current = Rc::downgrade(owner);
            previous
        });
        CurrentOwnerGuard { previous }
    }
}

impl Drop for CurrentOwnerGuard {
    #[inline(never)]
    fn drop(&mut self) {
        with_runtime(|runtime| runtime.restore_current_owner(self.previous.take()));
    }
}

/// Ends a flush when the call returns or unwinds.
struct FlushGuard;

impl Drop for FlushGuard {
    fn drop(&mut self) {
        with_runtime(|runtime| runtime.end_flush());
    }
}

/// Disposes a fresh node when creation unwinds, so an effect whose first run
/// panics with no owner around leaves nothing behind.
pub(crate) struct DisposeOnUnwind {
    /// The node to dispose.
    pub(crate) node: NodeId,
    /// Whether the node still needs disposing.
    pub(crate) armed: bool,
}

impl Drop for DisposeOnUnwind {
    fn drop(&mut self) {
        if self.armed {
            dispose(self.node);
        }
    }
}

/// Removes a node and unlinks its edges. A stale id does nothing.
///
/// The node is dropped after the runtime is released, because dropping a memo
/// or an effect drops the user's closure, and what it captured may enter the
/// runtime as it drops, as an effect handle or an owner does.
#[inline(never)]
pub(crate) fn dispose(id: NodeId) {
    let node = with_runtime(|runtime| runtime.dispose_node(id));
    drop(node);
}

/// Records a tracked read of `source` into the running observer.
///
/// The read is silently untracked when no effect or memo runs, or when the
/// source was disposed.
#[inline(never)]
pub(crate) fn track_read(source: NodeId) {
    let Some(reader) = OBSERVER.with(Cell::get) else {
        return;
    };
    with_runtime(|runtime| {
        if let Some(version) = runtime.version_of(source) {
            runtime.track(reader, source, version);
        }
    });
}

/// Returns the live owner that was current when the running effect or memo
/// was created, or `None` when no effect or memo runs.
///
/// A rerun runs with no owner current, so a read that needs the application
/// of the running code, such as the window state, finds it through here.
#[inline(never)]
pub(crate) fn observer_owner() -> Option<Rc<OwnerData>> {
    let observer = OBSERVER.with(Cell::get)?;
    with_runtime(|runtime| runtime.creation_owner(observer))
}

/// Returns the owner the running code belongs to: the creation owner of the
/// running effect or memo, or else the current owner, when it is live.
///
/// Code that needs the application of the running code, such as a window
/// command or a new channel, finds it through here, so it works the same in
/// a binding's rerun, which runs with no owner current, as in a handler.
#[inline(never)]
pub(crate) fn running_owner() -> Option<Rc<OwnerData>> {
    observer_owner().or_else(|| {
        with_runtime(|runtime| runtime.current_owner()).filter(|owner| !owner.cleaned.get())
    })
}

/// Runs the flush loop and returns whether any effect ran.
///
/// Effects queued by writes made while the loop drains run in the same call.
/// A call from inside a running flush does nothing and returns `false`.
#[inline(never)]
pub(crate) fn flush() -> bool {
    if !with_runtime(|runtime| runtime.begin_flush()) {
        return false;
    }
    let _guard = FlushGuard;
    while let Some(id) = with_runtime(|runtime| runtime.pop_effect()) {
        run_effect(id);
    }
    with_runtime(|runtime| runtime.flush_ran())
}

/// Runs one queued effect: it verifies its sources, pulling memos up to date,
/// and skips its body when nothing changed.
///
/// The sources are checked in the order the last run read them, and the
/// check stops at the first one that moved. The body then runs and reads
/// what it needs, so a memo that only a branch no longer taken reads is not
/// computed.
fn run_effect(id: NodeId) {
    if !with_runtime(|runtime| runtime.begin_effect(id)) {
        return;
    }
    let changed = sources_changed(id);
    with_runtime(|runtime| runtime.release_effect(id));
    if !changed {
        return;
    }
    let Some(cell) = with_runtime(|runtime| runtime.effect_cell(id)) else {
        return;
    };
    with_runtime(|runtime| {
        runtime.clear_sources(id);
        runtime.set_ran();
    });
    run_body(&cell, id);
}

/// Runs an effect body under tracking, as a flush step or the first run at
/// creation does.
#[inline(never)]
pub(crate) fn run_body(cell: &Rc<EffectCell>, node: NodeId) {
    let _observer = ObserverGuard::enter(Some(node));
    let mut borrow = cell
        .run
        .try_borrow_mut()
        .expect("an effect is never called while it is running");
    if let Some(run) = &mut *borrow {
        run();
    }
}

/// Pulls the reader's sources up to date in the order it read them, and
/// returns whether one of them moved, stopping at the first that did.
fn sources_changed(id: NodeId) -> bool {
    let count = with_runtime(|runtime| runtime.source_count(id));
    for index in 0..count {
        let Some((source, _)) = with_runtime(|runtime| runtime.source_at(id, index)) else {
            break;
        };
        update_source(source);
        if with_runtime(|runtime| runtime.source_changed(id, index)) {
            return true;
        }
    }
    false
}

/// Brings a source up to date before its version is compared.
fn update_source(source: NodeId) {
    if with_runtime(|runtime| runtime.is_memo(source)) {
        update_memo(source);
    }
}

/// Recomputes a memo when its value is behind its sources.
///
/// A dirty memo recomputes at once. A memo whose source may have changed
/// brings its sources up to date in the order it read them, and recomputes
/// as soon as one of them moved, without computing the ones after it.
#[inline(never)]
pub(crate) fn update_memo(id: NodeId) {
    match with_runtime(|runtime| runtime.memo_state(id)) {
        MemoState::Clean => {}
        MemoState::Dirty => recompute_memo(id),
        MemoState::Check => {
            if sources_changed(id) {
                recompute_memo(id);
            } else {
                with_runtime(|runtime| runtime.memo_set_clean(id));
            }
        }
    }
}

/// Recomputes a memo: it unsubscribes from its old sources and runs its
/// computation, which compares and stores the value and notifies on change.
fn recompute_memo(id: NodeId) {
    let Some(run) = with_runtime(|runtime| runtime.memo_run(id)) else {
        return;
    };
    with_runtime(|runtime| runtime.begin_memo(id));
    let _observer = ObserverGuard::enter(Some(id));
    run();
}

/// Runs the first computation of a fresh memo under the current observer.
#[inline(never)]
pub(crate) fn compute_new_memo(id: NodeId) {
    recompute_memo(id);
}

/// Where the scopes one write woke start in the runtime's woken list.
#[must_use = "the wakes must be fired once the runtime is released"]
pub(crate) struct Wakes(usize);

/// Runs the wake callbacks of the scopes a write woke, outside any runtime
/// borrow.
///
/// A callback may write signals and wake more scopes, which land after the
/// ones this call fires and are fired by the write that woke them.
fn fire_wakes(wakes: Wakes) {
    let end = with_runtime(|runtime| runtime.woken.len());
    for index in wakes.0..end {
        let callback = with_runtime(|runtime| {
            runtime
                .woken
                .get(index)
                .and_then(|scope| scope.wake.borrow().clone())
        });
        if let Some(callback) = callback {
            callback();
        }
    }
}
