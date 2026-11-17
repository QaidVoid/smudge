//! The values of the signals a build created, carried across a remount.
//!
//! A hot patch remounts the root, so every signal the build created starts
//! again from the value the new code gives it. To keep what the user typed
//! or scrolled, each signal is registered with its owner under a key: the path of its owner from the build owner, the
//! source file that created it, its ordinal among the signals of that file
//! and type under that owner, and its type. An owner stands in the path for
//! the hash of its row's key when a list names it, and otherwise for its
//! place among its parent's live children without a name when it was
//! created. Two siblings that stand alike, two rows whose keys hash alike or
//! two owners that took one place at different times, carry nothing, and a
//! row carries only into a row of an equal key. A remount stashes the
//! values of the old build's signals before it tears the build down, and a
//! signal the new build creates under the same key takes the stashed value
//! over: it adopts the old signal's storage, which a handle the app kept
//! from the old build shares, and moves it onto its own live node. The
//! line is left out of the key, so an edit above a signal keeps
//! its value, and two signals of one type in one owner and file that swap
//! places swap their values.
//!
//! A value is carried only when its type kept its layout. The size and the
//! alignment must match, and the patch builder must have listed the type's
//! probe among the kept ones, since a type can change its fields at the
//! same size. A type the original binary has no probe for, one the app
//! started using after a patch, is never listed, so its values start fresh.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};

use super::runtime::{self, NodeId, OwnerData, SignalData};

/// One signal an owner registered for carrying.
pub(crate) struct Carried {
    /// The source file that created the signal.
    file: &'static str,
    /// The signal's ordinal among the carried signals of that file and type
    /// under its owner.
    ordinal: u32,
    /// The type of the value.
    type_id: TypeId,
    /// The size and the alignment of the value's type.
    layout: (usize, usize),
    /// The runtime address, in the binary the app started with, of the
    /// probe of the value's type.
    probe: usize,
    /// The signal's shared data, which the stash keeps alive.
    value: Weak<dyn Any>,
    /// The node the signal notifies through.
    node: NodeId,
}

/// The name a list gives the owners of one of its rows. A row has two:
/// the owner its build runs in, under the list's own node, and the owner of
/// the node it mounts, under the build's owner.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum OwnerName {
    /// The owner the row's build runs in, by the hash of the row's key.
    Row(u64),
    /// The owner of the node the row mounts, by the hash of the row's key.
    RowNode(u64),
    /// An owner of a row whose key hashes like another key of its list,
    /// which carries nothing, since its values could not be told from the
    /// other row's.
    Unkeyed,
}

/// How a list names the owners of one of its rows.
#[derive(Clone)]
pub(crate) enum RowKey {
    /// By the hash of the row's key, which no other key of its list shares,
    /// and the key itself.
    Hashed(u64, Rc<RowIdentity>),
    /// By nothing, since another key of its list hashes alike.
    Ambiguous,
}

/// The key of a list row, kept so a carried value crosses only to the row
/// of an equal key: two keys can hash alike without being equal.
pub(crate) struct RowIdentity {
    /// A copy of the key.
    key: Box<dyn Any>,
    /// The probe of the key's type, which must be kept for the old key to
    /// be read by the new code.
    probe: usize,
    /// Whether two keys of this one's type are equal.
    equal: fn(&dyn Any, &dyn Any) -> bool,
}

impl RowIdentity {
    /// The identity of the row of `key`.
    pub(crate) fn of<K: PartialEq + Clone + 'static>(key: &K) -> Rc<RowIdentity> {
        Rc::new(RowIdentity {
            key: Box::new(key.clone()),
            probe: crate::hot::probe_of::<K>(),
            equal: equal_keys::<K>,
        })
    }

    /// Whether the patch kept the layout of the key types of `self`, a row
    /// of the old build, and `new`, a row of the new build, which must hold
    /// before the new code reads the old key.
    fn both_kept(&self, new: &RowIdentity, kept: &HashSet<usize>) -> bool {
        (*self.key).type_id() == (*new.key).type_id()
            && kept.contains(&self.probe)
            && kept.contains(&new.probe)
    }

    /// Whether `self`, a row of the old build, has the key of `new`, a row
    /// of the new build, as the new code compares them. Call it only once
    /// [`RowIdentity::both_kept`] held for the two.
    fn equals(&self, new: &RowIdentity) -> bool {
        (new.equal)(&*self.key, &*new.key)
    }
}

/// Whether `left` and `right` are equal keys of type `K`.
fn equal_keys<K: PartialEq + 'static>(left: &dyn Any, right: &dyn Any) -> bool {
    match (left.downcast_ref::<K>(), right.downcast_ref::<K>()) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// Names `owner` as the owner of a list row, the row's own when `node` is
/// off and the owner of the node it mounts when it is on.
pub(crate) fn name_owner(owner: &OwnerData, row: &RowKey, node: bool) {
    let (name, identity) = match row {
        RowKey::Hashed(hash, identity) if node => {
            (OwnerName::RowNode(*hash), Some(Rc::clone(identity)))
        }
        RowKey::Hashed(hash, identity) => (OwnerName::Row(*hash), Some(Rc::clone(identity))),
        RowKey::Ambiguous => (OwnerName::Unkeyed, None),
    };
    owner.name.set(Some(name));
    *owner.row.borrow_mut() = identity;
    owner.uncount();
}

/// How one owner stands in a carried signal's key.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
enum Step {
    /// The name the list gave the owner.
    Named(OwnerName),
    /// The owner's place among its parent's live children without a name
    /// when it was created.
    At(u32),
}

/// What identifies a signal across two builds.
#[derive(PartialEq, Eq, Hash)]
struct Key {
    /// The steps from the build owner down to the signal's owner.
    path: Vec<Step>,
    /// The source file that created the signal.
    file: &'static str,
    /// The signal's ordinal among the carried signals of that file and type
    /// under its owner.
    ordinal: u32,
    /// The type of the value.
    type_id: TypeId,
}

/// A value the old build left, waiting for the new build's signal.
struct Stashed {
    /// The size and the alignment of the value's type.
    layout: (usize, usize),
    /// The old signal's shared data.
    value: Rc<dyn Any>,
    /// The old signal's node, which the old build's cleanup left alive so
    /// the observers that outlive the build keep their subscriptions.
    node: Kept,
    /// The keys of the list rows on the signal's path, from the build owner
    /// down, which the new signal's rows must equal.
    rows: Vec<Rc<RowIdentity>>,
}

/// A signal node no owner holds while a remount offers it, which this
/// disposes when it drops unless a new owner took it.
struct Kept(Option<NodeId>);

impl Kept {
    /// Hands the node over to a new owner.
    fn take(&mut self) -> Option<NodeId> {
        self.0.take()
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        if let Some(node) = self.0.take() {
            runtime::dispose(node);
        }
    }
}

/// The values a remount took from the old build, and the new build owner
/// the keys are counted from.
struct Stash {
    /// Which offer the stash belongs to.
    id: u64,
    /// The new build's owner.
    build: Weak<OwnerData>,
    /// The old build's values by key.
    values: HashMap<Key, Stashed>,
    /// The type probes the patch kept.
    kept: HashSet<usize>,
    /// How many values the new build claimed since the last frame ended.
    claims: usize,
}

/// The most frames a remount keeps offering the old build's values for:
/// rows that mount once the frame before measured them claim theirs a few
/// frames after the remount.
const OFFER_FRAMES: u32 = 32;

/// How many frames in a row may claim nothing before the offer ends.
const QUIET_FRAMES: u32 = 2;

thread_local! {
    /// The stashes of the remounts whose offers are open on this thread,
    /// one per app that remounted.
    static STASHES: RefCell<Vec<Stash>> = const { RefCell::new(Vec::new()) };
    /// The id the next offer takes.
    static NEXT_OFFER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many values the carrying code is dropping on this thread. A
    /// signal such a value's drop creates is neither registered nor given
    /// a value, so it never moves the ordinals of the build's own signals.
    static DISCARDING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// The mounts running on this thread, innermost last, each by the
    /// owner it mounts under, and the effect bodies that run inside them,
    /// as `None`. Only a signal the innermost mount creates under its owner
    /// claims a carried value: one an effect, a handler, a timer or another
    /// app's code creates is a new signal, not one the build rebuilt.
    static MOUNTS: RefCell<Vec<Option<Weak<OwnerData>>>> = const { RefCell::new(Vec::new()) };
}

/// Marks a mount as running under `owner` on this thread while it lives,
/// so the signals it creates under that owner may claim the values a
/// remount offers. A mount that runs inside a value's drop, an app made or
/// remounted from a drop, registers its signals as any mount does, so it
/// lifts the discarding while it runs.
pub(crate) struct Mounting {
    /// The discarding the mount lifted, which it restores.
    discarding: u32,
}

impl Mounting {
    /// Starts marking a mount under `owner` as running.
    pub(crate) fn enter(owner: &Rc<OwnerData>) -> Mounting {
        MOUNTS.with(|mounts| mounts.borrow_mut().push(Some(Rc::downgrade(owner))));
        Mounting {
            discarding: DISCARDING.with(|discarding| discarding.replace(0)),
        }
    }
}

impl Drop for Mounting {
    fn drop(&mut self) {
        MOUNTS.with(|mounts| mounts.borrow_mut().pop());
        DISCARDING.with(|discarding| discarding.set(self.discarding));
    }
}

/// Stops the signals created on this thread from claiming carried values
/// while it lives, for an effect that runs inside a mount: a signal its body
/// creates is not one the build rebuilt.
pub(crate) struct Paused;

impl Paused {
    /// Stops claiming until the guard drops.
    pub(crate) fn enter() -> Paused {
        MOUNTS.with(|mounts| mounts.borrow_mut().push(None));
        Paused
    }
}

impl Drop for Paused {
    fn drop(&mut self) {
        MOUNTS.with(|mounts| mounts.borrow_mut().pop());
    }
}

/// Whether a signal created now under `owner` may claim a carried value:
/// the innermost mount runs under `owner` or one of its ancestors.
fn may_claim(owner: &Rc<OwnerData>) -> bool {
    let Some(Some(mount)) = MOUNTS.with(|mounts| mounts.borrow().last().cloned()) else {
        return false;
    };
    let Some(mount) = mount.upgrade() else {
        return false;
    };
    let mut current = Rc::clone(owner);
    loop {
        if Rc::ptr_eq(&current, &mount) {
            return true;
        }
        let Some(parent) = current.parent.as_ref().and_then(Weak::upgrade) else {
            return false;
        };
        current = parent;
    }
}

/// The fewest registrations an owner holds before it sweeps out the ones
/// whose signals are gone.
const MIN_SWEEP: usize = 8;

/// The hash a list names the owners of the row of `key` by.
pub(crate) fn name_of<K: Hash>(key: &K) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

/// Registers the signal whose data is `data`, created in `file` under the
/// current owner, and answers the storage it goes on with: the old build's
/// signal of the same key, moved onto the new signal's node, while a
/// remount is in progress and the value may cross, or `data` itself. A
/// signal with no current owner is never carried.
///
/// This generic half is compiled in the crate that created the signal, so
/// it only reads the type and rebinds the storage: everything that reaches
/// the runtime's thread-locals sits in [`register_erased`], which a patch
/// of that crate calls instead of copying.
pub(crate) fn register<T: 'static>(
    file: &'static str,
    data: Rc<SignalData<T>>,
) -> Rc<SignalData<T>> {
    let value: Rc<dyn Any> = Rc::clone(&data) as Rc<dyn Any>;
    let Some(old) = register_erased(
        file,
        TypeId::of::<T>(),
        (std::mem::size_of::<T>(), std::mem::align_of::<T>()),
        crate::hot::probe_of::<T>(),
        data.node.get(),
        &value,
    ) else {
        return data;
    };
    match old.downcast::<SignalData<T>>() {
        // The new signal goes on with the old storage and the old node, and
        // its own initial value is dropped.
        Ok(old) => {
            drop(data);
            discard(value);
            old
        }
        Err(_) => data,
    }
}

/// Drops a value the carrying code let go of, with the signals its drop
/// creates kept out of the carrying.
#[inline(never)]
fn discard(value: Rc<dyn Any>) {
    let _discarding = Discarding::enter();
    drop(value);
}

/// Keeps the signals created on this thread out of the carrying while it
/// lives, and lets them back in when it drops, a panicking drop included.
struct Discarding;

impl Discarding {
    /// Starts keeping signals out.
    fn enter() -> Discarding {
        DISCARDING.with(|discarding| discarding.set(discarding.get() + 1));
        Discarding
    }
}

impl Drop for Discarding {
    fn drop(&mut self) {
        DISCARDING.with(|discarding| discarding.set(discarding.get().saturating_sub(1)));
    }
}

/// Registers the signal whose shared data is `value` with the current
/// owner, and answers the old build's signal of the same key whose value
/// it takes over, when a remount is in progress and the value may cross.
#[inline(never)]
fn register_erased(
    file: &'static str,
    type_id: TypeId,
    layout: (usize, usize),
    probe: usize,
    node: NodeId,
    value: &Rc<dyn Any>,
) -> Option<Rc<dyn Any>> {
    if DISCARDING.with(std::cell::Cell::get) > 0 {
        return None;
    }
    let owner = runtime::current_owner()?;
    let ordinal = {
        let mut state = owner.state.borrow_mut();
        let ordinal = match state
            .ordinals
            .iter_mut()
            .find(|(key, _)| *key == (file, type_id))
        {
            Some((_, next)) => {
                let ordinal = *next;
                *next += 1;
                ordinal
            }
            None => {
                state.ordinals.push(((file, type_id), 1));
                0
            }
        };
        // An owner that lives long, a handler's or the root, can create a
        // signal per event, so the registrations of dropped signals are
        // swept out whenever the list doubles. The ordinals keep counting,
        // which keys only ever need to be unique.
        let len = state.carried.len();
        if len >= MIN_SWEEP && len.is_power_of_two() {
            state
                .carried
                .retain(|carried| carried.value.strong_count() > 0);
        }
        state.carried.push(Carried {
            file,
            ordinal,
            type_id,
            layout,
            probe,
            value: Rc::downgrade(value),
            node,
        });
        ordinal
    };
    if !may_claim(&owner) {
        return None;
    }
    // The offer is the one whose build owner the signal's owner sits
    // under, since each app that remounted keeps its own. An app made
    // inside another's build sits under that build too, so the nearest
    // build owner is the signal's own.
    let builds: Vec<(u64, Weak<OwnerData>)> = STASHES.with(|stashes| {
        stashes
            .borrow()
            .iter()
            .map(|stash| (stash.id, stash.build.clone()))
            .collect()
    });
    let (id, (path, rows)) = builds
        .iter()
        .filter_map(|(id, build)| Some((*id, path_from(build, &owner)?)))
        .min_by_key(|(_, (path, _))| path.len())?;
    let key = Key {
        path,
        file,
        ordinal,
        type_id,
    };
    // The stash lets go of the value before anything drops it, since a
    // value's drop can run any code, a remount's own included.
    // Whether the patch kept each type in question is read while the stash
    // is borrowed; comparing the keys runs the app's own code, which may
    // create signals, so it runs after.
    let (mut old, kept, rows_kept) = STASHES.with(|stashes| {
        let mut stashes = stashes.borrow_mut();
        let stash = stashes.iter_mut().find(|stash| stash.id == id)?;
        let old = stash.values.remove(&key)?;
        stash.claims += 1;
        let rows_kept = old.rows.len() == rows.len()
            && old
                .rows
                .iter()
                .zip(&rows)
                .all(|(old_row, new_row)| old_row.both_kept(new_row, &stash.kept));
        Some((old, stash.kept.contains(&probe), rows_kept))
    })?;
    // The rows the old value sat in must be the rows the new signal sits
    // in, key for key, not only by hash.
    // The comparison is the app's code, and a signal it creates is not one
    // of the build's, so it neither registers nor claims.
    let same_rows = rows_kept && {
        let _discarding = Discarding::enter();
        old.rows
            .iter()
            .zip(&rows)
            .all(|(old_row, new_row)| old_row.equals(new_row))
    };
    // The comparison may also have cleaned up the owner, which then holds
    // nothing a value could go on with.
    let still_held = !owner.cleaned.get() && owner.state.borrow().nodes.contains(&node);
    if old.layout != layout || !kept || !same_rows || !still_held {
        discard_stashed(old);
        return None;
    }
    // The signal goes on with the old storage and the old node, whose
    // subscribers include the observers that outlived the old build, so
    // the new owner holds the old node in place of the one it was given.
    let old_node = old.node.take()?;
    {
        let mut state = owner.state.borrow_mut();
        if let Some(at) = state.nodes.iter().position(|held| *held == node) {
            state.nodes[at] = old_node;
        }
        // The registration is the one of the node the new signal was given:
        // the key comparison above ran the app's code, which may have
        // registered other signals since.
        if let Some(carried) = state
            .carried
            .iter_mut()
            .rev()
            .find(|carried| carried.node == node)
        {
            carried.value = Rc::downgrade(&old.value);
            carried.node = old_node;
        }
    }
    runtime::dispose(node);
    Some(old.value)
}

/// Drops a stashed value no signal took, with the signals its drop creates
/// kept out of the carrying.
#[inline(never)]
fn discard_stashed(stashed: Stashed) {
    let _discarding = Discarding::enter();
    drop(stashed);
}

/// How `owner` stands in a key among its parent's children.
fn step_of(owner: &OwnerData) -> Option<Step> {
    match owner.name.get() {
        Some(OwnerName::Unkeyed) => None,
        Some(name) => Some(Step::Named(name)),
        None => Some(Step::At(owner.place.get())),
    }
}

/// The steps from `build` down to `owner`, and the keys of the list rows on
/// the way, or none when `owner` is not `build` or one of its descendants.
fn path_from(
    build: &Weak<OwnerData>,
    owner: &Rc<OwnerData>,
) -> Option<(Vec<Step>, Vec<Rc<RowIdentity>>)> {
    let build = build.upgrade()?;
    let mut path = Vec::new();
    let mut rows = Vec::new();
    let mut current = Rc::clone(owner);
    while !Rc::ptr_eq(&current, &build) {
        let parent = current.parent.as_ref()?.upgrade()?;
        path.push(step_of(&current)?);
        if let Some(row) = &*current.row.borrow() {
            rows.push(Rc::clone(row));
        }
        current = parent;
    }
    path.reverse();
    rows.reverse();
    Some((path, rows))
}

/// Takes the values of the signals `build` and its descendants registered
/// whose types' probes are in `kept`, keyed by their place under `build`.
/// It runs before anything tears the build down, since a cleanup forgets
/// what each owner registered.
pub(crate) fn take(build: &Rc<OwnerData>, kept: HashSet<usize>) -> Taken {
    let mut values = HashMap::new();
    let mut pending = vec![(Rc::clone(build), Vec::new(), Vec::new())];
    while let Some((owner, path, rows)) = pending.pop() {
        // The stash keeps each node it takes alive past the cleanup, so the
        // owner lets go of it.
        let mut taken_nodes = Vec::new();
        let state = owner.state.borrow();
        for carried in &state.carried {
            if !kept.contains(&carried.probe) {
                continue;
            }
            let Some(value) = carried.value.upgrade() else {
                continue;
            };
            taken_nodes.push(carried.node);
            values.insert(
                Key {
                    path: path.clone(),
                    file: carried.file,
                    ordinal: carried.ordinal,
                    type_id: carried.type_id,
                },
                Stashed {
                    layout: carried.layout,
                    value,
                    node: Kept(Some(carried.node)),
                    rows: rows.clone(),
                },
            );
        }
        // Two rows whose keys hash alike cannot be told apart, nor two
        // owners that took one place at different times, so neither carries
        // anything rather than one taking the other's values.
        let live: Vec<Rc<OwnerData>> = state
            .children
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|child| !child.cleaned.get())
            .collect();
        let mut steps: HashMap<Step, usize> = HashMap::new();
        for child in &live {
            if let Some(step) = step_of(child) {
                *steps.entry(step).or_default() += 1;
            }
        }
        for child in live {
            let Some(step) = step_of(&child).filter(|step| steps[step] == 1) else {
                continue;
            };
            let mut below = path.clone();
            below.push(step);
            let mut rows_below = rows.clone();
            if let Some(row) = &*child.row.borrow() {
                rows_below.push(Rc::clone(row));
            }
            pending.push((child, below, rows_below));
        }
        drop(state);
        if !taken_nodes.is_empty() {
            owner
                .state
                .borrow_mut()
                .nodes
                .retain(|node| !taken_nodes.contains(node));
        }
    }
    Taken { values, kept }
}

/// The values a remount took from the old build, waiting for the new build
/// owner.
pub(crate) struct Taken {
    /// The old build's values by key.
    values: HashMap<Key, Stashed>,
    /// The type probes the patch kept.
    kept: HashSet<usize>,
}

impl Taken {
    /// Offers the values to the signals created under `build` until the
    /// returned guard drops.
    pub(crate) fn offer(self, build: &Rc<OwnerData>) -> Offered {
        let id = NEXT_OFFER.with(|next| next.replace(next.get().wrapping_add(1)));
        STASHES.with(|stashes| {
            stashes.borrow_mut().push(Stash {
                id,
                build: Rc::downgrade(build),
                values: self.values,
                kept: self.kept,
                claims: 0,
            });
        });
        Offered {
            id,
            frames: 0,
            quiet: 0,
        }
    }
}

/// Keeps the old build's values on offer while it lives. The new build
/// mounts some of its parts in the frames after the remount, a list's rows
/// and a shown branch among them, so the offer lasts until every value was
/// claimed or the frames stop claiming any. When it drops it ends the offer
/// and drops the values nobody claimed, outside the stash's borrow, since a
/// value's drop can run any code.
pub(crate) struct Offered {
    /// Which stash the offer keeps open.
    id: u64,
    /// The frames that ended since the offer began.
    frames: u32,
    /// The frames in a row that ended without a claim.
    quiet: u32,
}

impl Offered {
    /// Counts a frame that ended, and answers whether the offer goes on.
    pub(crate) fn frame_ended(&mut self) -> bool {
        let (claims, left) = STASHES.with(|stashes| {
            let mut stashes = stashes.borrow_mut();
            stashes
                .iter_mut()
                .find(|stash| stash.id == self.id)
                .map_or((0, 0), |stash| {
                    (std::mem::take(&mut stash.claims), stash.values.len())
                })
        });
        self.frames += 1;
        self.quiet = if claims == 0 { self.quiet + 1 } else { 0 };
        left > 0 && self.quiet < QUIET_FRAMES && self.frames < OFFER_FRAMES
    }
}

impl Drop for Offered {
    fn drop(&mut self) {
        let left = STASHES.with(|stashes| {
            let mut stashes = stashes.borrow_mut();
            let at = stashes.iter().position(|stash| stash.id == self.id)?;
            Some(stashes.swap_remove(at))
        });
        if let Some(left) = left {
            for (_, stashed) in left.values {
                discard_stashed(stashed);
            }
        }
    }
}
