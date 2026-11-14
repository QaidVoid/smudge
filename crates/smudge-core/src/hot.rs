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
//! in the crate that created the closure, and a patch of that crate maps the
//! instance to its new version. The wrapper passes the size and the
//! alignment of the closure's type as the creating code saw them; the shim
//! compares them with its own view before it calls, and reports a mismatch
//! when a capture changed the layout, on which the wrapper calls the code it
//! holds, whose environment it built. A change that keeps the size and the
//! alignment passes the guard, and a patch that inserts a closure before
//! another one in the same function can map a shim to the wrong closure of
//! the same shape. The layout gate of the patch builder refuses most of
//! that; what it cannot see is documented as a limit.
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move || imp::call_shared(imp::published(), &f, size, align)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move || imp::call_mut(imp::published(), &mut f, size, align)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |arg| imp::call1_shared(imp::published(), &f, size, align, arg)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |arg| imp::call1_mut(imp::published(), &mut f, size, align, arg)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |first, second| imp::call2_mut(imp::published(), &mut f, size, align, first, second)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |arg: &A| imp::call1_shared(imp::published(), &f, size, align, arg)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |arg: &A| imp::call1_mut(imp::published(), &mut f, size, align, arg)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |arg: &mut A| imp::call1_shared(imp::published(), &f, size, align, arg)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        move |arg: &A, second| imp::call2_mut(imp::published(), &mut f, size, align, arg, second)
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
        let (size, align) = (std::mem::size_of::<F>(), std::mem::align_of::<F>());
        imp::call_synced(&mut f, size, align)
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
    use std::cell::RefCell;
    use std::mem::{align_of, size_of, transmute_copy};
    use std::rc::{Rc, Weak};
    use std::sync::atomic::{AtomicPtr, Ordering};

    use subsecond::JumpTable;

    /// The signature a two-argument shim and its patch share: the closure
    /// as an erased pointer, the size and the alignment the wrapper
    /// recorded, and the two arguments, which the shim hands back when its
    /// guard fails.
    type Shim2<A, B, R> = fn(*mut (), usize, usize, A, B) -> Result<R, (A, B)>;

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
        let published = table().map_or(std::ptr::null_mut(), |table| {
            std::ptr::from_ref(table).cast_mut()
        });
        PUBLISHED.store(published, Ordering::Release);
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
    pub(super) fn call_shared<R, F: Fn() -> R>(
        table: Option<&JumpTable>,
        env: &F,
        size: usize,
        align: usize,
    ) -> R {
        let shim: fn(*const (), usize, usize) -> Result<R, ()> = shim_shared::<R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: a patch maps the shim's address only to the new
                // version of the same shim, which the patch builder
                // compiled with this signature for this process, as the
                // wire format requires of every function it maps, and
                // `find` filtered the null address out. The environment
                // crosses the boundary as an erased pointer, so the
                // patched code forms a reference to it only after its
                // guard accepted the size and the alignment the wrapper
                // recorded; `Err` sends the call back to the version the
                // wrapper holds, whose environment it built.
                let run = unsafe {
                    let patched: fn(*const (), usize, usize) -> Result<R, ()> =
                        transmute_copy(&mapped);
                    patched(std::ptr::from_ref(env).cast(), size, align)
                };
                run.unwrap_or_else(|()| env())
            }
            None => env(),
        }
    }

    /// Runs a mutating, no-argument closure through the published table.
    pub(super) fn call_mut<R, F: FnMut() -> R>(
        table: Option<&JumpTable>,
        env: &mut F,
        size: usize,
        align: usize,
    ) -> R {
        let shim: fn(*mut (), usize, usize) -> Result<R, ()> = shim_mut::<R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too.
                let run = unsafe {
                    let patched: fn(*mut (), usize, usize) -> Result<R, ()> =
                        transmute_copy(&mapped);
                    patched(std::ptr::from_mut(env).cast(), size, align)
                };
                run.unwrap_or_else(|()| env())
            }
            None => env(),
        }
    }

    /// Runs a mutating, no-argument closure on a thread patches are not
    /// applied on, which reads the table the applying thread published.
    pub(super) fn call_synced<R, F: FnMut() -> R>(env: &mut F, size: usize, align: usize) -> R {
        call_mut(published(), env, size, align)
    }

    /// Runs a shared, one-argument closure through the published table.
    pub(super) fn call1_shared<A, R, F: Fn(A) -> R>(
        table: Option<&JumpTable>,
        env: &F,
        size: usize,
        align: usize,
        arg: A,
    ) -> R {
        let shim: fn(*const (), usize, usize, A) -> Result<R, A> = shim1_shared::<A, R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too. The
                // shim hands the argument back when its guard fails, so the
                // fallback can run the code the wrapper holds.
                let run = unsafe {
                    let patched: fn(*const (), usize, usize, A) -> Result<R, A> =
                        transmute_copy(&mapped);
                    patched(std::ptr::from_ref(env).cast(), size, align, arg)
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
    pub(super) fn call1_mut<A, R, F: FnMut(A) -> R>(
        table: Option<&JumpTable>,
        env: &mut F,
        size: usize,
        align: usize,
        arg: A,
    ) -> R {
        let shim: fn(*mut (), usize, usize, A) -> Result<R, A> = shim1_mut::<A, R, F>;
        match find(table, shim as usize) {
            Some(mapped) => {
                // SAFETY: the argument in `call_shared` applies here too. The
                // shim hands the argument back when its guard fails, so the
                // fallback can run the code the wrapper holds.
                let run = unsafe {
                    let patched: fn(*mut (), usize, usize, A) -> Result<R, A> =
                        transmute_copy(&mapped);
                    patched(std::ptr::from_mut(env).cast(), size, align, arg)
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
    pub(super) fn call2_mut<A, B, R, F: FnMut(A, B) -> R>(
        table: Option<&JumpTable>,
        env: &mut F,
        size: usize,
        align: usize,
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
                    patched(std::ptr::from_mut(env).cast(), size, align, first, second)
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
    /// Calls the closure only when the size and the alignment of the
    /// closure's type, as the calling code saw them, match this code's own
    /// view. A mismatch means a capture changed its layout, and the
    /// calling code runs the version it holds instead.
    #[inline(never)]
    fn shim_shared<R, F: Fn() -> R>(env: *const (), size: usize, align: usize) -> Result<R, ()> {
        if size_of::<F>() == size && align_of::<F>() == align {
            // SAFETY: the guard passed, so the layout this code sees of the
            // closure's type is the layout the wrapper recorded, and the
            // pointer came from the wrapper's reference to a live closure
            // of that type.
            let env = unsafe { &*env.cast::<F>() };
            Ok(env())
        } else {
            Err(())
        }
    }

    /// The mutating, no-argument shim a wrapper looks up.
    #[inline(never)]
    fn shim_mut<R, F: FnMut() -> R>(env: *mut (), size: usize, align: usize) -> Result<R, ()> {
        if size_of::<F>() == size && align_of::<F>() == align {
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
    /// Calls the closure only when the size and the alignment of the
    /// closure's type, as the calling code saw them, match this code's own
    /// view, and hands the argument back otherwise, so the calling code can
    /// run the version it holds.
    #[inline(never)]
    fn shim1_shared<A, R, F: Fn(A) -> R>(
        env: *const (),
        size: usize,
        align: usize,
        arg: A,
    ) -> Result<R, A> {
        if size_of::<F>() == size && align_of::<F>() == align {
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
    fn shim1_mut<A, R, F: FnMut(A) -> R>(
        env: *mut (),
        size: usize,
        align: usize,
        arg: A,
    ) -> Result<R, A> {
        if size_of::<F>() == size && align_of::<F>() == align {
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
    fn shim2_mut<A, B, R, F: FnMut(A, B) -> R>(
        env: *mut (),
        size: usize,
        align: usize,
        first: A,
        second: B,
    ) -> Result<R, (A, B)> {
        if size_of::<F>() == size && align_of::<F>() == align {
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
    pub(super) fn shim_key_shared<R, F: Fn() -> R>(_f: &F) -> usize {
        let shim: fn(*const (), usize, usize) -> Result<R, ()> = shim_shared::<R, F>;
        shim as usize
    }

    /// The address the one-argument lookup keys a closure's shim by.
    #[cfg(test)]
    pub(super) fn shim_key1_mut<A, R, F: FnMut(A) -> R>(_f: &F) -> usize {
        let shim: fn(*mut (), usize, usize, A) -> Result<R, A> = shim1_mut::<A, R, F>;
        shim as usize
    }

    /// Runs the real no-argument shim of `env`'s type with the size and the
    /// alignment the test hands in, and reports whether the guard accepted
    /// them. For the tests, which pin the guard's behavior.
    #[cfg(test)]
    pub(super) fn guard_runs<R, F: Fn() -> R>(env: &F, size: usize, align: usize) -> bool {
        let shim: fn(*const (), usize, usize) -> Result<R, ()> = shim_shared::<R, F>;
        shim(std::ptr::from_ref(env).cast(), size, align).is_ok()
    }

    /// The same for a one-argument handler, which the shim runs when the
    /// guard accepts the layout.
    #[cfg(test)]
    pub(super) fn guard1_mut_runs<A, R, F: FnMut(A) -> R>(
        env: &mut F,
        size: usize,
        align: usize,
        arg: A,
    ) -> bool {
        let shim: fn(*mut (), usize, usize, A) -> Result<R, A> = shim1_mut::<A, R, F>;
        shim(std::ptr::from_mut(env).cast(), size, align, arg).is_ok()
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
