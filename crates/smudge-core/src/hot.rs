//! Hot reload for the closures an application stores in the tree, and for
//! the loop bodies of the threads it spawns itself.
//!
//! A patch replaces the code of the functions an application crate compiled,
//! but a closure the tree has already stored keeps the code it was built
//! with, because only a call through the patch's jump table reaches the new
//! version. Every entry where Smudge erases a user closure's type calls one
//! of the [`wrap`](crate::hot::wrap) functions here, so the stored closure carries the address
//! of a small generic shim and reaches the patched version of that shim on
//! every call. [`call`](crate::hot::call) does the same for a loop body on a thread the app
//! spawned, and [`on_hot_patch`](crate::hot::on_hot_patch) registers a hook the window runs after it
//! applied a patch and remounted, so an app can restart the workers it
//! created outside the build.
//!
//! # How the lookup works
//!
//! The shim is generic over the closure's type, so its instance is compiled
//! in the crate that created the closure, and a patch of that crate maps
//! the instance to its new version. The calling code passes the [`TypeId`]
//! of the closure's type as it was compiled; the shim compares it with the
//! id of its own closure type before it calls, and reports a mismatch on
//! which the calling code runs the version it holds, whose environment it
//! built. A closure's type id names the closure, by its place among the
//! closures of the function that defines it, and the types of its captures,
//! so a capture that changes its type reports a mismatch, while an edit of
//! the body alone keeps the id. A closure inserted before another one of
//! the same function, or removed from before it, renumbers it, so another
//! closure with captures of the same types can take over its id. The patch
//! builder therefore leaves every closure of a function whose closures
//! changed in number out of the table, and a closure the app holds since
//! before the patch keeps its old body until a restart. Two closures that
//! swap places, or a closure removed while another is added, keep the count
//! and can still exchange bodies. Remounting rebuilds the closures of the
//! tree, so either case reaches only the ones kept outside it, such as
//! those of a configure step, a timer or a worker. The id does not cover
//! the layout of a captured type whose own fields change: the patch builder
//! must leave such a shim out, which it does by comparing the debug info of
//! each shim's closure type, as the wire format requires of the signatures
//! of every function it maps.
//!
//! [`TypeId`]: std::any::TypeId
//!
//! A patch is applied on the thread that runs the event loop, which
//! publishes the installed table with release ordering once it is in
//! place. Every lookup, of a wrapped closure as much as of a loop body
//! through [`call`](crate::hot::call), reads that published pointer with
//! acquire ordering, so a call on any thread that sees the table also sees
//! the whole table that came with the patch, and code on the installing
//! thread needs no edge of its own.
//!
//! Closures created by patched code run their own shim and never see a
//! later patch, because the table is keyed by the addresses of the binary
//! the app started with. Remounting, which is the default, rebuilds them.

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
use crate::reactive::runtime::OwnerData;
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
use std::rc::Rc;

/// Wraps a closure with no argument that the tree stores behind
/// `Rc<dyn Fn()>`, so a hot patch replaces its body.
///
/// This is the form of a binding's value, a memo's computation, a theme
/// binding and the closures of [`show`](crate::element::show),
/// [`for_each`](crate::element::for_each) and the virtual lists. With the
/// `hot-reload` feature off, the closure comes back unchanged.
#[inline(always)]
pub fn wrap<R, F: Fn() -> R + 'static>(f: F) -> impl Fn() -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        move || imp::call_shared(imp::published(), &f)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with no argument that the tree stores behind
/// `Box<dyn FnMut()>`, so a hot patch replaces its body.
///
/// This is the form of a timer's callback and an effect's body. With the
/// `hot-reload` feature off, the closure comes back unchanged.
#[inline(always)]
pub fn wrap_mut<R, F: FnMut() -> R + 'static>(f: F) -> impl FnMut() -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        let mut f = f;
        move || imp::call_mut(imp::published(), &mut f)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with one argument that the tree stores behind
/// `Rc<dyn Fn(A)>`, so a hot patch replaces its body.
///
/// The argument may be a value or a reference, whichever the closure takes.
/// This is the form of
/// [`style_children`](crate::element::Element::style_children) and of the
/// keyed lists' keys and rows. With the `hot-reload` feature off, the
/// closure comes back unchanged.
#[inline(always)]
pub fn wrap1<A, R, F: Fn(A) -> R + 'static>(f: F) -> impl Fn(A) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        move |arg| imp::call1_shared(imp::published(), &f, arg)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with one argument that the tree stores behind
/// `Box<dyn FnMut(A)>`, so a hot patch replaces its body.
///
/// The argument may be a value or a reference, whichever the closure takes.
/// This is the form of every input handler, a channel's handler and the
/// tick callback. With the `hot-reload` feature off, the closure comes back
/// unchanged.
#[inline(always)]
pub fn wrap1_mut<A, R, F: FnMut(A) -> R + 'static>(f: F) -> impl FnMut(A) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        let mut f = f;
        move |arg| imp::call1_mut(imp::published(), &mut f, arg)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with two arguments that the tree stores behind
/// `Box<dyn FnMut(A, B)>`, so a hot patch replaces its body.
///
/// This is the form of the input method's preedit handler. With the
/// `hot-reload` feature off, the closure comes back unchanged.
#[inline(always)]
pub fn wrap2_mut<A, B, R, F: FnMut(A, B) -> R + 'static>(f: F) -> impl FnMut(A, B) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        let mut f = f;
        move |first, second| imp::call2_mut(imp::published(), &mut f, first, second)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with one argument by reference that the tree stores
/// behind `Rc<dyn Fn(&A)>`, so a hot patch replaces its body.
///
/// The argument may be a reference to any type. With the `hot-reload`
/// feature off, the closure comes back unchanged.
#[inline(always)]
pub fn wrap1_ref<A: ?Sized, R, F: for<'a> Fn(&'a A) -> R + 'static>(
    f: F,
) -> impl for<'a> Fn(&'a A) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        move |arg: &A| imp::call1_shared(imp::published(), &f, arg)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with one argument by reference that the tree stores
/// behind `Box<dyn FnMut(&A)>`, so a hot patch replaces its body.
///
/// This is the form of the commit handler of the input method and the key
/// handler. With the `hot-reload` feature off, the closure comes back
/// unchanged.
#[inline(always)]
pub fn wrap1_ref_mut<A: ?Sized, R, F: for<'a> FnMut(&'a A) -> R + 'static>(
    f: F,
) -> impl for<'a> FnMut(&'a A) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        let mut f = f;
        move |arg: &A| imp::call1_mut(imp::published(), &mut f, arg)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with one argument by mutable reference that the tree
/// stores behind `Rc<dyn Fn(&mut A)>`, so a hot patch replaces its body.
///
/// This is the form of
/// [`style_children`](crate::element::Element::style_children). With the
/// `hot-reload` feature off, the closure comes back unchanged.
#[inline(always)]
pub fn wrap1_mut_ref<A: ?Sized, R, F: for<'a> Fn(&'a mut A) -> R + 'static>(
    f: F,
) -> impl for<'a> Fn(&'a mut A) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        move |arg: &mut A| imp::call1_shared(imp::published(), &f, arg)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Wraps a closure with a first argument by reference and a second by value
/// that the tree stores behind `Box<dyn FnMut(&A, B)>`, so a hot patch
/// replaces its body.
///
/// This is the form of the input method's preedit handler. With the
/// `hot-reload` feature off, the closure comes back unchanged.
#[inline(always)]
pub fn wrap2_ref_mut<A: ?Sized, B, R, F: for<'a> FnMut(&'a A, B) -> R + 'static>(
    f: F,
) -> impl for<'a> FnMut(&'a A, B) -> R + 'static {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        let mut f = f;
        move |arg: &A, second| imp::call2_mut(imp::published(), &mut f, arg, second)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f
    }
}

/// Runs one step of a loop on a thread the application spawned itself,
/// through the newest version a patch installed.
///
/// A thread that was running before the app started cannot be rebuilt, so
/// its loop calls this with its body and picks up the patched code on the
/// next iteration, with every capture it already holds. The body runs on
/// the calling thread, which is not the thread patches are applied on, so
/// the call reads the table that the applying thread published with release
/// ordering, and runs the code it holds while no table was published to it
/// yet.
///
/// With the `hot-reload` feature off, the body runs as it is.
pub fn call<R, F: FnMut() -> R + 'static>(mut f: F) -> R {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        imp::call_synced(&mut f)
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        f()
    }
}

/// Registers a hook the window runs on the thread that runs the event loop,
/// after it applied a patch and remounted.
///
/// An app that spawned workers outside the build restarts them in the hook.
/// The hook belongs to the owner that is current when it is registered,
/// like a timer: a hook registered inside the build goes with the build,
/// which is cleaned up and rebuilt on every patch, so the remount itself
/// adds it no second time. A hook registered later in the app's own scope,
/// for example in a timer callback, runs after every patch. Registration
/// needs an owner that is current and reaches the app's book of hooks,
/// which the app wires to its root owner as it starts: with no owner
/// current, or before the app has started, the hook is dropped, as a timer
/// in the same places creates none.
///
/// With the `hot-reload` feature off, the hook is dropped.
pub fn on_hot_patch<F: FnMut() + 'static>(f: F) {
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    {
        let Some(owner) = crate::reactive::runtime::current_owner()
            .or_else(crate::reactive::runtime::observer_owner)
        else {
            return;
        };
        if owner.cleaned.get() {
            return;
        }
        imp::register(Box::new(wrap_mut(f)), &owner);
    }

    #[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
    {
        drop(f)
    }
}

/// Tells the calls on the app's threads that a new patch was installed.
///
/// This is for the receiver that applies patches, which runs on the thread
/// that runs the event loop: it publishes the installed table to the calls
/// of [`call`], with release ordering, so a call on another thread that
/// sees the table also sees everything that was written into it. Without
/// it, a call on another thread would have no edge to the install, because
/// subsecond hands the table out with relaxed ordering.
///
/// # Safety
///
/// The caller must run on the thread that applied the patch and call this
/// right after the install. A call on any other thread reads the table
/// with relaxed ordering, so publishing what it read could hand the
/// readers a table whose contents they would see with no edge to the
/// writes that filled it in.
///
/// The body holds no unsafe code; the lint allows the declaration, whose
/// contract is the point of the function.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[allow(unsafe_code)]
pub unsafe fn note_patch_installed() {
    imp::note_installed();
}

/// The size, the alignment and the name of a type, as the code compiled with
/// it sees the type.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
pub type Shape = (usize, usize, &'static str);

/// Returns the shape of `T`, as this instance was compiled.
///
/// Every instance is the probe of its type. The window announces the
/// instance for the app's state type to the patch builder, which finds
/// every other instance in the binary from it, compares the type each one
/// names with the patch, and lists the ones that kept their layout. The
/// window's state guard calls the patched instance for the state type, and
/// a remount carries a signal's value into the new build only when the
/// probe of its type is listed, which [`note_kept_types`] records.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[inline(never)]
pub fn shape<T>() -> Shape {
    (
        std::mem::size_of::<T>(),
        std::mem::align_of::<T>(),
        std::any::type_name::<T>(),
    )
}

/// The runtime address of the probe of `T` that the code calling this
/// reaches, mapped to the binary the app started with when a patch
/// replaced it, which is what a remount records for a signal of type `T`
/// and checks against the type probes a patch kept. A type the app first
/// used after a patch has its probe only in a patch library, and its
/// address there matches no probe a patch keeps.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
pub fn probe_of<T>() -> usize {
    original_address(shape::<T> as fn() -> Shape as usize)
}

/// Records the type probes a patch kept in the layout the app's live values
/// hold, by their runtime addresses in the binary the app started with, as
/// the patch builder listed them, for the next remount on this thread.
///
/// The window calls this on the thread that applies patches, right after
/// [`note_patch_installed`] and before it remounts. That remount carries
/// the values of the signals of exactly those types into the new build, as
/// [`App::remount`](crate::app::App::remount) describes, and every later
/// one starts its signals fresh until the next patch. When the remount does
/// not run, [`forget_kept_types`] drops the list.
///
/// # Safety
///
/// Every probe listed must name a type whose layout the patched code reads
/// exactly as the values the current build holds were laid out, which only
/// the patch builder can tell. A value of a type listed wrongly is read in
/// another layout by the new build, which is undefined behavior.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[allow(unsafe_code)]
#[inline(never)]
pub unsafe fn note_kept_types(probes: std::collections::HashSet<usize>) {
    imp::note_kept(probes);
}

/// Drops the type probes [`note_kept_types`] recorded, so a remount that
/// does not follow a patch carries nothing.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[inline(never)]
pub fn forget_kept_types() {
    drop(imp::take_kept());
}

/// The type probes the last patch kept, which the remount that follows it
/// takes, or none when no patch came since the last remount.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
pub(crate) fn take_kept() -> Option<std::collections::HashSet<usize>> {
    imp::take_kept()
}

/// The address in the binary the app started with of the function at
/// `address`: the function the installed table maps to `address`, or
/// `address` itself when the table maps nothing to it. Code a patch
/// compiled runs at the address its library was loaded at, and this finds
/// the original it replaces.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[inline(never)]
pub(crate) fn original_address(address: usize) -> usize {
    imp::original(address)
}

/// Runs every live hook that [`on_hot_patch`] registered, inside the owners
/// the hooks belong to.
///
/// This is for the window, on the thread that runs the event loop, after it
/// applied a patch and remounted. A build's own hooks are dead by then, and
/// the hooks the new build registered while rebuilding are the ones this
/// runs, so a hook runs once per patch.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
pub(crate) fn run_hot_patch_hooks(book: &std::cell::RefCell<HotPatchBook>) {
    imp::run_hooks(book);
}

/// The hot patch hooks of one app, held by the root owner like the timer
/// book.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
pub(crate) struct HotPatchBook {
    /// The hooks, in no particular order. A hook whose owner was cleaned up
    /// is dropped after the next round ran.
    entries: Vec<Rc<HotPatchEntry>>,
}

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
impl HotPatchBook {
    /// An empty book, for the app that wires it to its root owner.
    pub(crate) fn new() -> HotPatchBook {
        HotPatchBook {
            entries: Vec::new(),
        }
    }
}

/// One registered hook, shared by the book that runs it and its owner.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
struct HotPatchEntry {
    /// The hook to run. Taken out while it runs, so a hook that registers a
    /// hook cannot leave the entry borrowed.
    run: std::cell::RefCell<Option<Box<dyn FnMut()>>>,
    /// The owner the hook was registered under. A cleaned up owner's hook
    /// is dead, which is what keeps a build's hooks from outliving it.
    owner: std::rc::Weak<OwnerData>,
}

/// The shim, the jump table lookup and the hooks of the feature, whose
/// address casts are the only unsafe code in this crate.
///
/// The generic `call*` functions and the generic shims are instantiated in
/// the crate that created the closure, which is what a patch maps. They
/// reach the table only through `published`, which is neither generic nor
/// inlined, so the patched code reads the runtime's own copy of the
/// published table and never a fresh one. `table` itself is for the
/// publication, which runs right after an install.
#[cfg(all(feature = "hot-reload", target_os = "linux"))]
#[allow(unsafe_code)]
mod imp {
    use std::any::TypeId;
    use std::cell::RefCell;
    use std::mem::transmute_copy;
    use std::rc::{Rc, Weak};
    use std::sync::atomic::{AtomicPtr, Ordering};

    use subsecond::JumpTable;

    /// The signature a two-argument shim and its patch share: the closure
    /// as an erased pointer, the type id of the closure as the calling code
    /// was compiled, and the two arguments, which the shim hands back when
    /// its guard fails.
    type Shim2<A, B, R> = fn(*mut (), TypeId, A, B) -> Result<R, (A, B)>;

    use super::{HotPatchBook, HotPatchEntry};
    use crate::reactive::runtime::{CurrentOwnerGuard, OwnerData};

    /// The table the thread that applies patches published for the app's
    /// own threads, or null while none was.
    static PUBLISHED: AtomicPtr<JumpTable> = AtomicPtr::new(std::ptr::null_mut());

    /// Publishes the installed table to the calls on the app's own threads.
    /// Runs on the thread that applies patches, right after the table was
    /// installed: the release store of the pointer hands every write that
    /// filled the table in to whoever acquires it.
    pub(super) fn note_installed() {
        let installed = table();
        let published = installed.map_or(std::ptr::null_mut(), |table| {
            std::ptr::from_ref(table).cast_mut()
        });
        PUBLISHED.store(published, Ordering::Release);
        ORIGINALS.with(|originals| {
            let mut originals = originals.borrow_mut();
            originals.clear();
            if let Some(table) = installed {
                for (old, new) in &table.map {
                    // Two originals a folding link gave one replacement
                    // cannot be told apart, so that replacement maps to no
                    // original, and a probe on it vouches for nothing.
                    originals
                        .entry(*new as usize)
                        .and_modify(|original| *original = usize::MAX)
                        .or_insert(*old as usize);
                }
            }
        });
    }

    thread_local! {
        /// The original address of every function the installed table maps,
        /// by the address of its patched version, which only the thread
        /// that applies patches keeps.
        static ORIGINALS: RefCell<std::collections::HashMap<usize, usize>> =
            RefCell::new(std::collections::HashMap::new());
        /// The type probes the last patch kept in the running layout,
        /// until the remount that follows it takes them.
        static KEPT: RefCell<Option<std::collections::HashSet<usize>>> =
            const { RefCell::new(None) };
    }

    /// Records the type probes the last patch kept.
    pub(super) fn note_kept(probes: std::collections::HashSet<usize>) {
        KEPT.with(|kept| *kept.borrow_mut() = Some(probes));
    }

    /// Takes the type probes the last patch kept.
    pub(super) fn take_kept() -> Option<std::collections::HashSet<usize>> {
        KEPT.with(|kept| kept.borrow_mut().take())
    }

    /// The original address of the function at `address`.
    #[inline(never)]
    pub(super) fn original(address: usize) -> usize {
        ORIGINALS.with(|originals| originals.borrow().get(&address).copied().unwrap_or(address))
    }

    /// Reads the jump table of the running process, which only the thread
    /// that applies patches does, right after an install.
    ///
    /// Subsecond publishes this pointer with relaxed ordering, which is why
    /// every reader goes through [`published`] instead: a relaxed load on
    /// another thread could see the pointer with no edge to the writes that
    /// filled the table in. On the installing thread the install itself
    /// happens before the read, so the relaxed load needs no edge.
    #[inline(never)]
    pub(super) fn table() -> Option<&'static JumpTable> {
        // SAFETY: the pinned implementation of `get_jump_table` leaks every
        // installed table and never writes a table again after its install,
        // so the `&'static` it hands out holds for the rest of the process
        // and `note_installed` may keep the pointer it forms from this
        // reference. The only caller is `note_installed`, which runs on the
        // thread that applied the patch, where the install happened before
        // this read and the relaxed load needs no edge.
        unsafe { subsecond::get_jump_table() }
    }

    /// The table the installing thread published, or `None` while no patch
    /// was installed yet.
    ///
    /// This is the reader of [`note_installed`]'s publication, so a caller
    /// on any thread that sees the table also sees everything that was
    /// written into it, which is what makes a lookup sound off the thread
    /// that applies patches.
    #[inline(never)]
    pub(super) fn published() -> Option<&'static JumpTable> {
        // SAFETY: the pointer was stored by `note_installed` on the thread
        // that applies patches, and only ever names a table subsecond handed
        // out, which stays valid for the rest of the process and is never
        // written again after its install. A lookup may hold an earlier
        // table across a later install, which is safe because a table is
        // immutable once installed and its library stays loaded, so the
        // code it maps keeps running.
        unsafe { PUBLISHED.load(Ordering::Acquire).as_ref() }
    }

    /// Looks `address` up in `table`, whose keys and values are the
    /// addresses of the binary as it was linked.
    pub(super) fn find(table: Option<&JumpTable>, address: usize) -> Option<usize> {
        table
            .and_then(|table| table.map.get(&(address as u64)))
            .copied()
            .map(|mapped| mapped as usize)
            .filter(|&mapped| mapped != 0)
    }

    /// Runs a shared, no-argument closure through the published table.
    pub(super) fn call_shared<R, F: Fn() -> R + 'static>(table: Option<&JumpTable>, env: &F) -> R {
        let shim: fn(*const (), TypeId) -> Result<R, ()> = shim_shared::<R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: a patch maps the shim's address only to the new
                // version of the same shim, which the patch builder
                // compiled with this signature for this process, as the
                // wire format requires of every function it maps, and
                // `find` filtered the null address out. The environment
                // crosses the boundary as an erased pointer, so the
                // patched code forms a reference to it only after its
                // guard found the closure type it was compiled for to be
                // the one this code was compiled for; `Err` sends the call
                // back to the version the wrapper holds, whose environment
                // it built.
                let run = unsafe {
                    let patched: fn(*const (), TypeId) -> Result<R, ()> = transmute_copy(&mapped);
                    patched(std::ptr::from_ref(env).cast(), TypeId::of::<F>())
                };
                run.unwrap_or_else(|()| env())
            }
            None => env(),
        }
    }

    /// Runs a mutating, no-argument closure through the published table.
    pub(super) fn call_mut<R, F: FnMut() -> R + 'static>(
        table: Option<&JumpTable>,
        env: &mut F,
    ) -> R {
        let shim: fn(*mut (), TypeId) -> Result<R, ()> = shim_mut::<R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too.
                let run = unsafe {
                    let patched: fn(*mut (), TypeId) -> Result<R, ()> = transmute_copy(&mapped);
                    patched(std::ptr::from_mut(env).cast(), TypeId::of::<F>())
                };
                run.unwrap_or_else(|()| env())
            }
            None => env(),
        }
    }

    /// Runs a mutating, no-argument closure on a thread patches are not
    /// applied on, which reads the table the applying thread published.
    pub(super) fn call_synced<R, F: FnMut() -> R + 'static>(env: &mut F) -> R {
        call_mut(published(), env)
    }

    /// Runs a shared, one-argument closure through the published table.
    pub(super) fn call1_shared<A, R, F: Fn(A) -> R + 'static>(
        table: Option<&JumpTable>,
        env: &F,
        arg: A,
    ) -> R {
        let shim: fn(*const (), TypeId, A) -> Result<R, A> = shim1_shared::<A, R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too. The
                // shim hands the argument back when its guard fails, so the
                // fallback can run the code the wrapper holds.
                let run = unsafe {
                    let patched: fn(*const (), TypeId, A) -> Result<R, A> = transmute_copy(&mapped);
                    patched(std::ptr::from_ref(env).cast(), TypeId::of::<F>(), arg)
                };
                match run {
                    Ok(result) => result,
                    Err(arg) => env(arg),
                }
            }
            None => env(arg),
        }
    }

    /// Runs a mutating, one-argument closure through the published table.
    pub(super) fn call1_mut<A, R, F: FnMut(A) -> R + 'static>(
        table: Option<&JumpTable>,
        env: &mut F,
        arg: A,
    ) -> R {
        let shim: fn(*mut (), TypeId, A) -> Result<R, A> = shim1_mut::<A, R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too. The
                // shim hands the argument back when its guard fails, so the
                // fallback can run the code the wrapper holds.
                let run = unsafe {
                    let patched: fn(*mut (), TypeId, A) -> Result<R, A> = transmute_copy(&mapped);
                    patched(std::ptr::from_mut(env).cast(), TypeId::of::<F>(), arg)
                };
                match run {
                    Ok(result) => result,
                    Err(arg) => env(arg),
                }
            }
            None => env(arg),
        }
    }

    /// Runs a mutating, two-argument closure through the published table.
    pub(super) fn call2_mut<A, B, R, F: FnMut(A, B) -> R + 'static>(
        table: Option<&JumpTable>,
        env: &mut F,
        first: A,
        second: B,
    ) -> R {
        let shim: Shim2<A, B, R> = shim2_mut::<A, B, R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too. The
                // shim hands the arguments back when its guard fails, so the
                // fallback can run the code the wrapper holds.
                let run = unsafe {
                    let patched: Shim2<A, B, R> = transmute_copy(&mapped);
                    patched(
                        std::ptr::from_mut(env).cast(),
                        TypeId::of::<F>(),
                        first,
                        second,
                    )
                };
                match run {
                    Ok(result) => result,
                    Err((first, second)) => env(first, second),
                }
            }
            None => env(first, second),
        }
    }

    /// The shared, no-argument shim a wrapper looks up.
    ///
    /// Calls the closure only when the closure type the calling code was
    /// compiled for is the one this code was compiled for. A mismatch means
    /// a capture changed its type or the closure is another one, and the
    /// calling code runs the version it holds instead.
    #[inline(never)]
    fn shim_shared<R, F: Fn() -> R + 'static>(env: *const (), id: TypeId) -> Result<R, ()> {
        if TypeId::of::<F>() == id {
            // SAFETY: the guard passed, so this code and the caller name one
            // closure type with the same captures, whose layout the patch
            // builder checked when it mapped this shim, and the pointer came
            // from the caller's reference to a live closure of that type.
            let env = unsafe { &*env.cast::<F>() };
            Ok(env())
        } else {
            Err(())
        }
    }

    /// The mutating, no-argument shim a wrapper looks up.
    #[inline(never)]
    fn shim_mut<R, F: FnMut() -> R + 'static>(env: *mut (), id: TypeId) -> Result<R, ()> {
        if TypeId::of::<F>() == id {
            // SAFETY: the guard passed, and the pointer came from the
            // wrapper's mutable reference to a live closure of this type.
            let env = unsafe { &mut *env.cast::<F>() };
            Ok(env())
        } else {
            Err(())
        }
    }

    /// The shared, one-argument shim a wrapper looks up.
    ///
    /// Calls the closure only when the closure type the calling code was
    /// compiled for is the one this code was compiled for, and hands the
    /// argument back otherwise, so the calling code can run the version it
    /// holds.
    #[inline(never)]
    fn shim1_shared<A, R, F: Fn(A) -> R + 'static>(
        env: *const (),
        id: TypeId,
        arg: A,
    ) -> Result<R, A> {
        if TypeId::of::<F>() == id {
            // SAFETY: the guard passed, and the pointer came from the
            // wrapper's reference to a live closure of this type.
            let env = unsafe { &*env.cast::<F>() };
            Ok(env(arg))
        } else {
            Err(arg)
        }
    }

    /// The mutating, one-argument shim a wrapper looks up.
    #[inline(never)]
    fn shim1_mut<A, R, F: FnMut(A) -> R + 'static>(
        env: *mut (),
        id: TypeId,
        arg: A,
    ) -> Result<R, A> {
        if TypeId::of::<F>() == id {
            // SAFETY: the guard passed, and the pointer came from the
            // wrapper's mutable reference to a live closure of this type.
            let env = unsafe { &mut *env.cast::<F>() };
            Ok(env(arg))
        } else {
            Err(arg)
        }
    }

    /// The mutating, two-argument shim a wrapper looks up.
    #[inline(never)]
    fn shim2_mut<A, B, R, F: FnMut(A, B) -> R + 'static>(
        env: *mut (),
        id: TypeId,
        first: A,
        second: B,
    ) -> Result<R, (A, B)> {
        if TypeId::of::<F>() == id {
            // SAFETY: the guard passed, and the pointer came from the
            // wrapper's mutable reference to a live closure of this type.
            let env = unsafe { &mut *env.cast::<F>() };
            Ok(env(first, second))
        } else {
            Err((first, second))
        }
    }

    /// The address the no-argument lookup keys a closure's shim by. For the
    /// tests, which build tables by hand from it.
    #[cfg(test)]
    pub(super) fn shim_key_shared<R, F: Fn() -> R + 'static>(_f: &F) -> usize {
        let shim: fn(*const (), TypeId) -> Result<R, ()> = shim_shared::<R, F>;
        shim as usize
    }

    /// The address the one-argument lookup keys a closure's shim by.
    #[cfg(test)]
    pub(super) fn shim_key1_mut<A, R, F: FnMut(A) -> R + 'static>(_f: &F) -> usize {
        let shim: fn(*mut (), TypeId, A) -> Result<R, A> = shim1_mut::<A, R, F>;
        shim as usize
    }

    /// Runs the real no-argument shim of `env`'s type with the type id the
    /// test hands in, and reports whether the guard accepted it. For the
    /// tests, which pin the guard's behavior.
    #[cfg(test)]
    pub(super) fn guard_runs<R, F: Fn() -> R + 'static>(env: &F, id: TypeId) -> bool {
        let shim: fn(*const (), TypeId) -> Result<R, ()> = shim_shared::<R, F>;
        shim(std::ptr::from_ref(env).cast(), id).is_ok()
    }

    /// The same for a one-argument handler, which the shim runs when the
    /// guard accepts the type id.
    #[cfg(test)]
    pub(super) fn guard1_mut_runs<A, R, F: FnMut(A) -> R + 'static>(
        env: &mut F,
        id: TypeId,
        arg: A,
    ) -> bool {
        let shim: fn(*mut (), TypeId, A) -> Result<R, A> = shim1_mut::<A, R, F>;
        shim(std::ptr::from_mut(env).cast(), id, arg).is_ok()
    }

    /// Registers `run` under `owner`, in the book of the app the owner
    /// belongs to.
    pub(super) fn register(run: Box<dyn FnMut()>, owner: &Rc<OwnerData>) {
        let mut current = Rc::clone(owner);
        let book = loop {
            let held = current
                .hot_patch_book
                .borrow()
                .as_ref()
                .and_then(Weak::upgrade);
            if let Some(book) = held {
                break book;
            }
            let Some(parent) = current.parent.as_ref().and_then(Weak::upgrade) else {
                return;
            };
            current = parent;
        };
        book.borrow_mut().entries.push(Rc::new(HotPatchEntry {
            run: RefCell::new(Some(run)),
            owner: Rc::downgrade(owner),
        }));
    }

    /// Runs the live hooks of `book`, inside the owners they belong to.
    ///
    /// The entries are snapshotted first, so a hook that registers another
    /// hook while it runs cannot see the book borrowed: the new hook waits
    /// for the next round, which is the once-per-patch rule anyway.
    pub(super) fn run_hooks(book: &RefCell<HotPatchBook>) {
        let entries = book.borrow().entries.clone();
        for entry in &entries {
            let Some(owner) = entry.owner.upgrade().filter(|owner| !owner.cleaned.get()) else {
                continue;
            };
            let Ok(mut run) = entry.run.try_borrow_mut() else {
                continue;
            };
            let Some(mut hook) = run.take() else {
                continue;
            };
            let _guard = CurrentOwnerGuard::enter(&owner);
            hook();
            *run = Some(hook);
        }
        book.borrow_mut().entries.retain(|entry| {
            entry
                .owner
                .upgrade()
                .is_some_and(|owner| !owner.cleaned.get())
        });
    }
}

#[cfg(test)]
#[path = "hot_tests.rs"]
mod tests;
