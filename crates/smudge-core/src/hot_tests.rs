//! Tests for the closure lookup, the layout guard and the patch hooks.
//!
//! The lookup tests run against tables made by hand, which map one function
//! of the test binary to another, and no library is loaded. The fakes stand
//! in for a patched shim: they never touch the environment pointer, they
//! answer from their own constants, so a test can tell that the mapped
//! version ran and that a reported mismatch falls back. The guard tests
//! call the real shims of the feature, through helpers of the `imp` module,
//! with the layouts of the closure and with a deliberately wrong one.

#[cfg(all(feature = "hot-reload", target_os = "linux"))]
mod hot {
    use std::cell::{Cell, RefCell};
    use std::mem::{align_of_val, size_of_val};
    use std::rc::Rc;

    use subsecond::JumpTable;

    use super::super::imp::{
        call_shared, call_synced, call1_mut, find, guard_runs, guard1_mut_runs, shim_key_shared,
        shim_key1_mut,
    };
    use super::super::{HotPatchBook, on_hot_patch, run_hot_patch_hooks};
    use crate::reactive::Owner;

    /// A table that maps each pair's first address to its second, with the
    /// other fields of a loaded library left at their zero values.
    fn table(entries: &[(usize, usize)]) -> JumpTable {
        JumpTable {
            lib: std::path::PathBuf::new(),
            map: entries
                .iter()
                .map(|&(from, to)| (from as u64, to as u64))
                .collect(),
            aslr_reference: 0,
            new_base_address: 0,
            ifunc_count: 0,
        }
    }

    /// A patched shim that answers on its own and never touches the
    /// environment: the mapped version ran, the test sees `11` where the
    /// closure itself returns `10`.
    fn always_patched(_env: *const (), _size: usize, _align: usize) -> Result<u32, ()> {
        Ok(11)
    }

    /// A patched shim that reports a mismatch whatever it is handed, which
    /// sends the call back to the code the wrapper holds.
    fn never_patched(_env: *const (), _size: usize, _align: usize) -> Result<u32, ()> {
        Err(())
    }

    /// A patched shim of the one-argument shape, which doubles what it
    /// receives and adds one, so a test can tell that the argument reached
    /// the mapped version.
    fn always1_patched(_env: *mut (), _size: usize, _align: usize, arg: u32) -> Result<u32, u32> {
        Ok(arg * 2 + 1)
    }

    /// A patched shim of the one-argument shape that reports a mismatch and
    /// hands the argument back, which is what the fallback runs on.
    fn never1_patched(_env: *mut (), _size: usize, _align: usize, arg: u32) -> Result<u32, u32> {
        Err(arg)
    }

    /// The address the no-argument lookup goes by for `f`'s type, and `to`,
    /// a function with the shim's signature, to map it to.
    fn shared_pair<F: Fn() -> u32>(
        _f: &F,
        to: fn(*const (), usize, usize) -> Result<u32, ()>,
    ) -> (usize, usize) {
        (shim_key_shared(_f), to as usize)
    }

    /// The same for a one-argument handler.
    fn handler_pair<F: FnMut(u32) -> u32>(
        _f: &F,
        to: fn(*mut (), usize, usize, u32) -> Result<u32, u32>,
    ) -> (usize, usize) {
        (shim_key1_mut(_f), to as usize)
    }

    #[test]
    fn a_mapped_shim_runs_the_patched_version() {
        let closure = || 10_u32;
        let (from, to) = shared_pair(&closure, always_patched);
        let mapped = table(&[(from, to)]);
        let (size, align) = (size_of_val(&closure), align_of_val(&closure));

        let ran = call_shared(Some(&mapped), &closure, size, align);

        assert_eq!(ran, 11, "the mapped shim ran and answered on its own");
    }

    #[test]
    fn a_shim_the_table_does_not_map_runs_the_original() {
        let closure = || 10_u32;
        let (size, align) = (size_of_val(&closure), align_of_val(&closure));

        let ran = call_shared(None, &closure, size, align);
        assert_eq!(ran, 10, "no patch ran the closure as it is");

        let ran = call_shared(Some(&table(&[])), &closure, size, align);
        assert_eq!(ran, 10, "an empty table ran the closure as it is");
    }

    #[test]
    fn a_shim_that_reports_a_mismatch_runs_the_original() {
        let closure = || 10_u32;
        let (from, to) = shared_pair(&closure, never_patched);
        let mapped = table(&[(from, to)]);
        let (size, align) = (size_of_val(&closure), align_of_val(&closure));

        let ran = call_shared(Some(&mapped), &closure, size, align);

        assert_eq!(ran, 10, "the reported mismatch fell back to the original");
    }

    #[test]
    fn the_guard_runs_a_closure_of_the_layout_it_recorded() {
        let runs = Rc::new(Cell::new(0_u32));
        let count = Rc::clone(&runs);
        let closure = move || count.set(count.get() + 1);

        let ran = guard_runs(&closure, size_of_val(&closure), align_of_val(&closure));

        assert!(ran, "the guard accepted the layout the wrapper recorded");
        assert_eq!(runs.get(), 1, "the shim ran the closure once");
    }

    #[test]
    fn the_guard_rejects_a_size_mismatch() {
        let runs = Rc::new(Cell::new(0_u32));
        let count = Rc::clone(&runs);
        let closure = move || count.set(count.get() + 1);

        let ran = guard_runs(&closure, size_of_val(&closure) + 8, align_of_val(&closure));

        assert!(!ran, "the guard rejected the larger size");
        assert_eq!(runs.get(), 0, "the mismatched shim ran nothing");
    }

    #[test]
    fn the_guard_rejects_an_alignment_mismatch() {
        let runs = Rc::new(Cell::new(0_u32));
        let count = Rc::clone(&runs);
        let closure = move || count.set(count.get() + 1);

        let ran = guard_runs(&closure, size_of_val(&closure), align_of_val(&closure) * 2);

        assert!(!ran, "the guard rejected the stricter alignment");
        assert_eq!(runs.get(), 0, "the mismatched shim ran nothing");
    }

    #[test]
    fn a_mapped_handler_runs_the_patched_version() {
        let mut handler = |arg: u32| arg * 2;
        let (from, to) = handler_pair(&handler, always1_patched);
        let mapped = table(&[(from, to)]);
        let (size, align) = (size_of_val(&handler), align_of_val(&handler));

        let ran = call1_mut(Some(&mapped), &mut handler, size, align, 20);

        assert_eq!(ran, 41, "the mapped shim doubled the argument it received");
    }

    #[test]
    fn a_handler_the_table_does_not_map_runs_the_original() {
        let mut handler = |arg: u32| arg * 2;
        let (size, align) = (size_of_val(&handler), align_of_val(&handler));

        let ran = call1_mut(None, &mut handler, size, align, 20);

        assert_eq!(ran, 40, "no patch ran the handler as it is");
    }

    #[test]
    fn a_handler_that_reports_a_mismatch_keeps_its_argument() {
        let seen = Cell::new(0_u32);
        let mut handler = |arg: u32| {
            seen.set(arg);
            arg * 2
        };
        let (from, to) = handler_pair(&handler, never1_patched);
        let mapped = table(&[(from, to)]);
        let (size, align) = (size_of_val(&handler), align_of_val(&handler));

        let ran = call1_mut(Some(&mapped), &mut handler, size, align, 20);

        assert_eq!(ran, 40, "the mismatch fell back to the original handler");
        assert_eq!(
            seen.get(),
            20,
            "the fallback received the handed-back argument"
        );
    }

    #[test]
    fn the_guard_of_a_handler_runs_it_and_keeps_it_mutable() {
        let mut handler = |arg: u32| arg * 2;
        let (size, align) = (size_of_val(&handler), align_of_val(&handler));

        let ran = guard1_mut_runs(&mut handler, size, align, 20);
        assert!(ran, "the guard accepted the layout the wrapper recorded");

        let ran = guard1_mut_runs(&mut handler, size + 8, align, 20);
        assert!(!ran, "the guard rejected the larger size");

        let ran = guard1_mut_runs(&mut handler, size, align, 20);
        assert!(ran, "the handler still runs after a rejected round");
    }

    #[test]
    fn the_synced_lookup_runs_the_original_while_no_patch_is_installed() {
        let mut body = || 6_u32;
        let (size, align) = (size_of_val(&body), align_of_val(&body));

        let ran = call_synced(&mut body, size, align);

        assert_eq!(ran, 6, "no table was published, so the body ran as it is");
    }

    #[test]
    fn a_null_address_in_the_table_maps_to_nothing() {
        let key = 0x1000;

        assert_eq!(find(Some(&table(&[(key, 0)])), key), None);
        assert_eq!(find(Some(&table(&[(key, 0x2000)])), key), Some(0x2000));
    }

    /// Wires `book` to `root` the way the app does, so registrations from
    /// the owner chain under `root` find it.
    fn with_book(root: &Owner, book: &Rc<RefCell<HotPatchBook>>) {
        root.data()
            .hot_patch_book
            .replace(Some(Rc::downgrade(book)));
    }

    #[test]
    fn a_hook_runs_once_per_round() {
        let root = Owner::new();
        let book = Rc::new(RefCell::new(HotPatchBook::new()));
        with_book(&root, &book);
        let runs = Rc::new(Cell::new(0_u32));
        let count = Rc::clone(&runs);
        root.with(|| on_hot_patch(move || count.set(count.get() + 1)));

        run_hot_patch_hooks(&book);
        run_hot_patch_hooks(&book);

        assert_eq!(runs.get(), 2, "the hook ran once per round");
    }

    #[test]
    fn a_hook_of_a_cleaned_owner_does_not_run() {
        let root = Owner::new();
        let book = Rc::new(RefCell::new(HotPatchBook::new()));
        with_book(&root, &book);
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        let build = root.child();
        build.with(|| on_hot_patch(move || flag.set(true)));
        build.cleanup();

        run_hot_patch_hooks(&book);

        assert!(!ran.get(), "the build's hook went with the build");
        assert!(
            book.borrow().entries.is_empty(),
            "the dead hook was dropped after the round"
        );
    }

    #[test]
    fn a_hook_with_no_owner_is_dropped() {
        let book = Rc::new(RefCell::new(HotPatchBook::new()));

        on_hot_patch(|| {});

        assert!(
            book.borrow().entries.is_empty(),
            "code that belongs to no owner registers nothing"
        );
    }
}

#[cfg(not(all(feature = "hot-reload", target_os = "linux")))]
mod plain {
    use std::mem::{align_of, size_of, size_of_val};

    /// Checks that wrapping keeps the closure's layout, with `F` the type
    /// of the closure the test passes in.
    fn check_size<F: Fn() + 'static>(closure: F) {
        let wrapped = super::super::wrap(closure);

        assert_eq!(
            size_of_val(&wrapped),
            size_of::<F>(),
            "the feature off, the wrapper is the closure"
        );
        assert_eq!(
            align_of_val(&wrapped),
            align_of::<F>(),
            "the feature off, the wrapper aligns like the closure"
        );
    }

    #[test]
    fn a_wrapped_closure_has_the_size_of_the_closure_itself() {
        let count = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let inner = std::rc::Rc::clone(&count);
        check_size(move || inner.set(inner.get() + 1));
    }
}
