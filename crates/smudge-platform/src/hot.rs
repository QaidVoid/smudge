//! The receiver that brings hot patches to a running app, and the lookup
//! that finds the patched version of a view.
//!
//! A patch builder that runs the app sets [`HOT_SOCKET_ENVIRONMENT`] to the
//! path of a Unix stream socket it listens on. The receiver connects to it,
//! introduces the app, and hands every jump table the builder sends to the
//! thread that runs the app, which applies it and answers. Without the
//! variable, nothing here runs: no connection and no thread.
//!
//! A patch is installed in [`Patch::apply`], on the thread that runs the
//! event loop, between two events, and [`current`] reads the table on that
//! same thread, so the window's own store and load never race. The
//! application's other threads may call through `smudge_core::hot` while a
//! patch is applied: they read the table the last patch published, which
//! is never rewritten after its install and whose library stays loaded, so
//! their calls are safe across an install.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use subsecond::JumpTable;

/// The environment variable that names the socket a patch builder listens
/// on.
///
/// The builder sets it to the path of a Unix stream socket, usually under
/// the target directory, before it starts the app. An app started through
/// [`crate::run_view`] or [`crate::run_view_with`] with the `hot-reload`
/// feature on Linux connects to it and applies the patches it receives.
/// Without the variable the app never looks for a builder.
///
/// # Wire format
///
/// This is version 2 of the format. Both sides send frames: one byte for the
/// frame's kind, the payload's length in bytes as a little-endian `u32`, and
/// then the payload. Every number inside a payload is little-endian too.
///
/// | Kind | Sent by | Payload |
/// |---|---|---|
/// | 1, hello | the app | the format version as a `u32`, the process id as a `u32`, the runtime address of `main` as a `u64`, the runtime address of the view as a `u64`, the runtime address of the state probe as a `u64`, and then the build id, the value of [`HOT_BUILD_ENVIRONMENT`], to the end |
/// | 2, patch | the builder | the address of `main` in the binary as linked, as a `u64`, the address of `main` in the patch library as linked, as a `u64`, the entry count as a `u64`, that many pairs of `u64` mapping the address of a function in the binary as linked to the address of its new version in the patch library as linked, the count of kept type probes as a `u64`, that many addresses of type probes in the binary as linked, as `u64`, and then the path of the patch library to the end |
/// | 3, applied | the app | empty |
/// | 4, failed | the app | why the patch was not applied, in UTF-8 |
/// | 5, building | the builder | empty |
/// | 6, build failed | the builder | why the patch could not be built, such as the compiler's message, in UTF-8 |
///
/// The app sends one hello right after it connects. It answers every patch, in
/// order, with applied or failed. The builder may send building when it starts
/// building a patch, and then build failed when the build fails, for the app to
/// show. Neither needs an answer. The app skips a frame of a kind it does not
/// know. It stops reading when the builder closes the socket, or when a frame's
/// header is broken or announces a payload over 64 MiB, because the stream can
/// no longer be followed.
///
/// Each patch frame is built against the hello of the connection it is sent
/// on: its library was compiled from the crates of the binary that process
/// runs, and its addresses refer to that binary as linked. A builder must
/// not send a patch to a process it did not start, nor a patch built for
/// another process, because the app trusts every patch to fit its own code.
/// A patch library must export `main`, which the app uses to find where the
/// library was loaded. A patch maps every function of each crate it rebuilt,
/// changed or not, so the app can tell from the table which functions the
/// new code no longer has, as the state guard of [`crate::run_view`] needs.
/// A function the new code no longer has is left out of the table, never
/// sent with the address 0, because the app adds the library's load address
/// to every entry. A builder must also leave out every function whose
/// signature, its parameter and return types, changed, because the app calls
/// a mapped function with the signature it has in the binary and cannot
/// check it: symbol names do not record signatures. A type probe, described
/// below, is the one exception: it takes nothing and returns the same tuple
/// whatever type it names, so it is mapped even when only that type
/// changed. When the view's own signature changed, the builder reports that
/// the change needs a restart instead of sending the patch.
///
/// The hello tells the builder which function is the view and which is the
/// state probe, by their runtime addresses, which the builder rebases
/// against the address of `main` like every other runtime address. The
/// state probe is the instance of a function generic over the state type
/// that `run_view` compiles for the app's state, whose debug information
/// names the state type as its type parameter. The builder reads the state
/// type there to keep the layout rule below. Every other instance of the
/// same function in the binary probes the type it is instantiated with, and
/// each patch lists the type probes whose types it kept in the layout the
/// app's live values hold. The remount after the patch carries the value of
/// a signal the old build created into the new build only when its type's
/// probe is listed, so a builder lists a probe only when it compared the
/// type and found that layout unchanged.
///
/// A builder must not send a patch that changes the layout of the state
/// type or of any type its values reach, directly or through signals, `Rc`,
/// `Box` or collections. Those values were laid out by the old code, and the
/// app's own guard sees only the outermost state type. Values created in
/// `main` or in the `configure` step of `run_view_with` outlive the build
/// too, but no probe names their types, so a change to them needs a
/// restart, which `run_view_with` documents.
///
/// Each patch replaces the one before it. Its library and its table hold
/// every function changed since the app started, not only those changed
/// since the last patch, and always the crate that defines the view and the
/// crate that calls `run_view` or `run_view_with`, where the state guard is
/// compiled, because
/// installing a table replaces the active one without merging entries,
/// while every earlier table and library stays allocated, which is what
/// keeps the calls of the application's workers valid across a patch.
///
/// A patch gets fresh, empty copies of the thread-locals defined in the
/// crates it rebuilds. It must therefore never rebuild a crate whose
/// thread-locals the running app relies on, which always includes the core
/// and the text crate of this library: their reactive runtime and text
/// system would start empty in the patched code. A thread-local of the app's
/// own rebuilt crates restarts from its initial value in the patched code.
/// A builder should give the patched code the running app's own copy of a
/// writable static of a rebuilt crate wherever it can: one the code reaches
/// through a full address, such as a public static of a library, which the
/// loader can point at the running app's copy, and whose type keeps its
/// layout. A static the code reaches by a 32-bit distance, such as any
/// static of the app crate, cannot be shared that way, because the patch
/// library is loaded too far from the running app, so the patched code gets
/// a copy at its initial value, which is why a builder should refuse to
/// rebuild a library crate whose writable statics it cannot share and say
/// so for the app's own crate. An immutable static is the patch's own, so
/// an edit of it shows, at another address than the running app's copy.
/// Every function and static of a crate the patch did not rebuild resolves
/// to the running process's own instance: a patch library must not carry a
/// private copy of a crate it did not rebuild, such as a second reactive
/// runtime, because its code would then read that copy's empty state.
///
/// The thread that runs the event loop applies every patch, and its read
/// of the table right after an install is ordered by program order. The
/// application's own threads reach the table only through the publication
/// of `smudge_core::hot`, whose release store on the event loop's thread
/// and acquire load on theirs order every read after the writes that
/// filled the table in. A table is never rewritten after its install and
/// every loaded library stays loaded, so concurrent readers need no other
/// synchronization. The app's binary must export
/// `main` in its dynamic symbol table, which the builder arranges when it
/// links. Without it the app sends no hello and leaves hot reload off.
///
/// Every patch names a library file of its own, at a path no earlier patch
/// used, and the file stays unchanged for as long as the app runs. The app
/// keeps every library it loaded, because code from it can still be
/// running, and loading a path a second time hands back the library already
/// loaded, whose addresses the new table would not fit.
///
/// The app trusts its builder completely: a patch runs native code in the
/// app. The socket is therefore one the builder created for the app it
/// started, and the app does not defend against a builder that sends
/// hostile frames, beyond the size limit above.
///
/// The app applies a patch on the thread that runs its window, between two
/// events, and answers applied once it has rebuilt its root through the
/// patched view. It answers failed, without loading anything, when the patch
/// does not decode, when its window is not open yet and when it is closing.
/// It also answers failed after the library was loaded and the table
/// installed, when the patch changed the type of the state the view reads:
/// the window then keeps the view it has, and the change needs a restart.
/// The app writes the reason of every failed answer to standard error too.
pub const HOT_SOCKET_ENVIRONMENT: &str = "SMUDGE_HOT_SOCKET";

/// The environment variable that carries the id of the build a patch
/// builder started.
///
/// The app reads it as bytes and sends it back in its hello, unchanged, so
/// the builder can tell the app it started from an older one. An unset
/// variable sends an empty id.
pub const HOT_BUILD_ENVIRONMENT: &str = "SMUDGE_HOT_BUILD";

/// The version of the wire format the hello announces.
const VERSION: u32 = 2;

/// The kind of the frame that introduces the app.
const HELLO: u8 = 1;

/// The kind of the frame that carries a jump table.
const PATCH: u8 = 2;

/// The kind of the frame that reports a patch as applied.
const APPLIED: u8 = 3;

/// The kind of the frame that reports a patch as refused.
const FAILED: u8 = 4;

/// The kind of the frame that tells the app a patch is being built.
const BUILDING: u8 = 5;

/// The kind of the frame that tells the app a patch could not be built.
const BUILD_FAILED: u8 = 6;

/// What the builder says about a patch before it sends one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Build {
    /// It started building a patch.
    Started,
    /// The patch could not be built, for this reason.
    Failed(String),
}

/// The largest payload the receiver accepts, so a broken header cannot make
/// it allocate without bound.
const MAX_PAYLOAD: u32 = 64 << 20;

/// A jump table on its way from the receiver to the thread that runs the
/// app, with the channel its answer goes back on.
///
/// A patch that is dropped unanswered, for example in an event the closing
/// event loop never delivered, is answered as failed.
pub(crate) struct Patch {
    /// The table the builder sent, as linked.
    table: JumpTable,
    /// The type probes whose types the patch kept in the running layout,
    /// as linked.
    kept: Vec<u64>,
    /// Where the outcome goes: `Ok` for applied, or why it was not.
    answer: mpsc::Sender<Result<(), String>>,
}

impl Patch {
    /// Loads the patch library, installs the table, runs `then` once the
    /// table is installed, answers the builder with the outcome and returns
    /// it, for the devtools to show.
    ///
    /// Call it only on the thread that runs the event loop, the one thread
    /// that reads the table through [`current`]. `then` rebuilds the root
    /// through the patched view, and its error is the answer when it fails.
    pub(crate) fn apply(self, then: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        // The probes are linked addresses, which move by the same distance
        // as `main` did.
        let linked_main = self.table.aslr_reference;
        let runtime_main = subsecond::aslr_reference() as u64;
        let kept: std::collections::HashSet<usize> = self
            .kept
            .iter()
            .map(|probe| probe.wrapping_sub(linked_main).wrapping_add(runtime_main) as usize)
            .collect();
        let outcome = install(self.table).and_then(|()| {
            note_patch_installed();
            // SAFETY: the builder lists a type probe only when it compared
            // the type the probe names with the patch and found the layout
            // the app's live values hold, as the wire format requires.
            unsafe { smudge_core::hot::note_kept_types(kept) };
            let remounted = then();
            // A view the guard refused leaves the build unmounted, and the
            // next remount, one the app asks for itself, carries nothing.
            smudge_core::hot::forget_kept_types();
            remounted
        });
        let _ = self.answer.send(outcome.clone());
        outcome
    }

    /// Answers the builder that the patch was not applied, for `reason`,
    /// without loading anything.
    pub(crate) fn refuse(self, reason: &str) {
        let _ = self.answer.send(Err(String::from(reason)));
    }
}

/// Publishes the table the last patch installed to the calls of
/// [`smudge_core::hot::call`], so a step of a loop on a thread the app
/// spawned itself picks the patch up.
///
/// [`Patch::apply`] calls this on the thread that runs the event loop,
/// right after it installed the table there.
pub(crate) fn note_patch_installed() {
    // SAFETY: `Patch::apply` runs on the thread that runs the event loop
    // and calls this right after `install` put the table in place on that
    // thread, which is the contract the core's publication asks of its
    // caller.
    unsafe { smudge_core::hot::note_patch_installed() }
}

/// Connects to the patch builder the environment names, and returns the
/// receiver's thread.
///
/// `deliver` runs on the receiver's thread for every patch that decodes. It
/// must hand the patch to the thread that runs the event loop, which applies or
/// refuses it. `report` runs there too, for every building and build failed
/// frame, which the window shows in the devtools. `view` is the runtime address
/// of the view that `run_view` or `run_view_with` was given, which the hello
/// announces so the builder can attribute a signature change to the view, and
/// `probe` is the runtime address of the state probe [`state_probe`] answers,
/// which the hello announces so the builder can check the state's layout. The
/// receiver waits for that answer before it reads the next frame. Returns
/// `None` when [`HOT_SOCKET_ENVIRONMENT`] is unset, and when the socket cannot
/// be reached, which it reports on standard error.
pub(crate) fn start(
    deliver: impl Fn(Patch) + Send + 'static,
    report: impl Fn(Build) + Send + 'static,
    view: usize,
    probe: usize,
) -> Option<JoinHandle<()>> {
    start_from(
        std::env::var_os(HOT_SOCKET_ENVIRONMENT),
        std::env::var_os(HOT_BUILD_ENVIRONMENT),
        subsecond::aslr_reference(),
        (view, probe),
        (deliver, report),
    )
}

/// Connects to `socket` like [`start`], with the values of the environment
/// variables, the runtime address of `main` and the runtime addresses of the
/// view and of the state probe passed in.
///
/// An address of `main` of 0 means the binary does not export `main` in its
/// dynamic symbol table, so no patch could be placed against it. Hot reload
/// is then off, and the app says so instead of connecting.
fn start_from(
    socket: Option<OsString>,
    build: Option<OsString>,
    main: usize,
    (view, probe): (usize, usize),
    (deliver, report): (
        impl Fn(Patch) + Send + 'static,
        impl Fn(Build) + Send + 'static,
    ),
) -> Option<JoinHandle<()>> {
    let socket = PathBuf::from(socket?);
    if main == 0 {
        eprintln!(
            "smudge: hot reload is off, because this binary does not export `main` in its \
             dynamic symbol table, which the patch builder must arrange when it links"
        );
        return None;
    }
    let build = build.unwrap_or_default();
    match connect(
        &socket,
        build.as_encoded_bytes(),
        main,
        (view, probe),
        deliver,
        report,
    ) {
        Ok(receiver) => Some(receiver),
        Err(error) => {
            eprintln!(
                "smudge: hot reload is off, because the patch builder at {} cannot be reached: {error}",
                socket.display()
            );
            None
        }
    }
}

/// Connects to `socket`, sends the hello and starts the thread that receives
/// the patches.
fn connect(
    socket: &Path,
    build: &[u8],
    main: usize,
    (view, probe): (usize, usize),
    deliver: impl Fn(Patch) + Send + 'static,
    report: impl Fn(Build) + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    let mut stream = UnixStream::connect(socket)?;
    let mut hello = Vec::with_capacity(32 + build.len());
    hello.extend_from_slice(&VERSION.to_le_bytes());
    hello.extend_from_slice(&std::process::id().to_le_bytes());
    hello.extend_from_slice(&(main as u64).to_le_bytes());
    hello.extend_from_slice(&(view as u64).to_le_bytes());
    hello.extend_from_slice(&(probe as u64).to_le_bytes());
    hello.extend_from_slice(build);
    write_frame(&mut stream, HELLO, &hello)?;
    std::thread::Builder::new()
        .name(String::from("smudge-hot-reload"))
        .spawn(move || receive(stream, deliver, report))
}

/// Delivers every patch that arrives on `stream` and answers each one, and
/// reports every building and build failed frame, until the builder closes
/// the socket or the stream breaks.
fn receive(mut stream: UnixStream, deliver: impl Fn(Patch), report: impl Fn(Build)) {
    loop {
        let (kind, payload) = match read_frame(&mut stream) {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(error) => {
                eprintln!("smudge: hot reload stopped: {error}");
                return;
            }
        };
        match kind {
            BUILDING => {
                report(Build::Started);
                continue;
            }
            BUILD_FAILED => {
                report(Build::Failed(
                    String::from_utf8_lossy(&payload).into_owned(),
                ));
                continue;
            }
            PATCH => {}
            _ => continue,
        }
        let outcome = decode_patch(&payload).and_then(|(table, kept)| {
            let (answer, answered) = mpsc::channel();
            deliver(Patch {
                table,
                kept,
                answer,
            });
            answered
                .recv()
                .unwrap_or_else(|_| Err(String::from("the app closed before it applied the patch")))
        });
        let answer = match outcome {
            Ok(()) => write_frame(&mut stream, APPLIED, &[]),
            Err(reason) => {
                eprintln!("smudge: a hot patch was not applied: {reason}");
                write_frame(&mut stream, FAILED, reason.as_bytes())
            }
        };
        if answer.is_err() {
            return;
        }
    }
}

/// Loads the patch library and installs `table`.
///
/// Runs only through [`Patch::apply`], on the thread that runs the event
/// loop.
fn install(table: JumpTable) -> Result<(), String> {
    // SAFETY: the table came over the connection of a patch builder that
    // started this very process, and the wire format requires that it was
    // built against the hello of that connection, as
    // `HOT_SOCKET_ENVIRONMENT` documents. The library was compiled from
    // this binary's own crates, every address refers to this binary as
    // linked, and each function maps to its new version, in a library at a
    // path no earlier patch used and left unchanged since, which is the
    // well-formed table `apply_patch` requires, and its unchanged functions
    // and statics resolve to this process's own, as the wire format also
    // requires. This runs on the thread that runs the event loop, which
    // applies every patch, so the relaxed store that publishes the table is
    // ordered before this call's read of it by program order, and no read
    // on this thread holds the previous table across this call. The
    // application's own
    // threads reach the table only through `smudge_core::hot`'s
    // publication. A reader that observes this install's publication
    // acquires the store of it on this thread, which orders its reads
    // after the writes this install made; a reader that still observes an
    // earlier publication synchronizes with that earlier install in the
    // same way. A table is never rewritten after its install and every
    // library stays loaded, so a concurrent reader races nothing here. The
    // loading runs in the event handler that applies the patch, between
    // two events, so the code the event loop itself runs never sees it
    // half done, while the application's workers keep running the code of
    // the earlier patches, whose libraries and tables this process keeps.
    // A view whose state
    // type changed is caught by the guard in `patched` before it runs,
    // except for a change that keeps the type's size, alignment and name,
    // which is undefined behavior the guard narrows but cannot rule out, as
    // `run_view` documents.
    let applied = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        subsecond::apply_patch(table)
    }));
    match applied {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(String::from(
            "applying the patch panicked, most likely because the library exports no main",
        )),
    }
}

/// Reads one frame, or returns `None` when the stream ended between frames.
fn read_frame(stream: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0; 5];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let [kind, length @ ..] = header;
    let length = u32::from_le_bytes(length);
    if length > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame announces {length} bytes, over the limit of {MAX_PAYLOAD}"),
        ));
    }
    let mut payload = vec![0; length as usize];
    stream.read_exact(&mut payload)?;
    Ok(Some((kind, payload)))
}

/// Writes one frame of `kind` carrying `payload`.
fn write_frame(stream: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the payload is too long"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

/// Reads the jump table a patch frame carries, and the type probes it
/// kept.
fn decode_patch(payload: &[u8]) -> Result<(JumpTable, Vec<u64>), String> {
    let mut rest = payload;
    let aslr_reference = take_u64(&mut rest)?;
    let new_base_address = take_u64(&mut rest)?;
    let count = take_u64(&mut rest)?;
    let map = (0..count)
        .map(|_| Ok((take_u64(&mut rest)?, take_u64(&mut rest)?)))
        .collect::<Result<_, String>>()?;
    let kept_count = take_u64(&mut rest)?;
    let kept = (0..kept_count)
        .map(|_| take_u64(&mut rest))
        .collect::<Result<_, String>>()?;
    if rest.is_empty() {
        return Err(String::from("the patch names no library"));
    }
    let table = JumpTable {
        lib: PathBuf::from(OsString::from_vec(rest.to_vec())),
        map,
        aslr_reference,
        new_base_address,
        ifunc_count: 0,
    };
    Ok((table, kept))
}

/// Takes a little-endian `u64` off the front of `rest`.
fn take_u64(rest: &mut &[u8]) -> Result<u64, String> {
    let (bytes, tail) = rest
        .split_first_chunk::<8>()
        .ok_or_else(|| String::from("the patch ends early"))?;
    *rest = tail;
    Ok(u64::from_le_bytes(*bytes))
}

/// The size, the alignment and the name of a type, as the code compiled with
/// it sees the type.
pub(crate) use smudge_core::hot::Shape;

/// Returns the runtime address of the state probe for `S`, the instance of
/// [`smudge_core::hot::shape`] that [`current`] checks, which the hello
/// announces.
///
/// [`current`] is generic over the state and `run_view` instantiates it
/// with the app's state type, so the probe's instance for that type is
/// compiled in the crate that calls `run_view`, which is not always the
/// crate that defines the view. A patch that rebuilds that crate maps the
/// instance to its new version like any other function, and the new
/// version reports the state type as the new code sees it. When the new
/// code no longer uses the type, the instance is gone from the patch, and
/// the table maps it to nothing.
pub(crate) fn state_probe<S>() -> usize {
    smudge_core::hot::shape::<S> as fn() -> Shape as usize
}

/// Returns the newest version of `view`: the version the last applied patch
/// installed, or `view` itself when no patch replaced it.
///
/// Every patch maps the functions of the binary as it was built, so `view`
/// must be the original pointer, never one this function returned. Call it
/// only on the thread that runs the event loop, where patches are applied.
///
/// # Errors
///
/// Returns why the patched view must not run when the newest patch changed
/// the state type, as [`patched`] describes. The view that built the current
/// root then stays, and the change needs a restart.
pub(crate) fn current<S, E>(view: fn(&S) -> E) -> Result<fn(&S) -> E, String> {
    // SAFETY: `get_jump_table` asks that the table not be held across a
    // patch. Patches are installed only on this thread, and the table is
    // read here and dropped before this function returns.
    let table = unsafe { subsecond::get_jump_table() };
    // SAFETY: a patch builder maps a function of the binary only to the new
    // version of the same function, compiled for this process, and leaves
    // out every function whose signature changed, as the wire format
    // requires. So a mapped probe keeps the probe's signature, and a mapped
    // view keeps the view's, but for the state type: the view's parameter
    // names the state type, whose definition can change without changing the
    // symbol. The wire format requires the builder to refuse a patch that
    // changed the layout of the state type or of any type it reaches. The
    // guard in `patched` is the second line: it refuses a view whose state
    // type changed its size, its alignment or its name, or was replaced by
    // another type. Running the view after a patch from a builder that
    // breaks the signature or the layout rule of the wire format is
    // undefined behavior, which `run_view` documents as a change that needs
    // a restart.
    unsafe { patched(table, view, smudge_core::hot::shape::<S>) }
}

/// Returns the function `table` maps `view` to, or `view` when it maps it to
/// nothing, after checking that the patch kept the shape of the state.
///
/// `probe` reports the shape of the state as the binary was built. The view's
/// new version is used only when `table` also maps `probe` to a new version
/// that reports the same size, alignment and type name. This catches a field
/// added, removed or retyped when that moves the size or the alignment, a
/// state type renamed, and a state replaced by another type, whose probe the
/// new code no longer has. It cannot catch a change that keeps the size, the
/// alignment and the name, such as two fields of one size swapped, or a field
/// changed to another type of the same size.
///
/// # Errors
///
/// Returns why the patched view must not run when `table` maps `view` but
/// the state's shape changed or `probe` is not mapped.
///
/// # Safety
///
/// Any address `table` maps `probe` to must be a function with the signature
/// of `probe`. Any address it maps `view` to must be a function with the
/// signature of `view` but for its state type, which must be `S` whenever
/// the patched probe reports the shape `probe` reports.
unsafe fn patched<S, E>(
    table: Option<&JumpTable>,
    view: fn(&S) -> E,
    probe: fn() -> Shape,
) -> Result<fn(&S) -> E, String> {
    // A real table never holds 0, because every entry is rebased by the
    // library's load address. The check only keeps a null address out of
    // the transmutes below, and the wire format says a function the new
    // code no longer has is left out, not sent as 0.
    let lookup = |address: usize| {
        table
            .and_then(|table| table.map.get(&(address as u64)))
            .copied()
            .filter(|&address| address != 0)
    };
    let Some(address) = lookup(view as usize) else {
        return Ok(view);
    };
    let (size, align, name) = probe();
    let Some(new_probe) = lookup(probe as usize) else {
        return Err(format!(
            "the patch maps the view but not the guard for the state type `{name}`: \
             either the state type changed, or the patch left out the crate that \
             calls `run_view` or `run_view_with`. The window keeps its view until a restart"
        ));
    };
    // SAFETY: the caller promises that a mapped probe has the probe's
    // signature, and the address is not null.
    let new_probe = unsafe { std::mem::transmute::<usize, fn() -> Shape>(new_probe as usize) };
    let (new_size, new_align, new_name) = new_probe();
    if (new_size, new_align, new_name) != (size, align, name) {
        return Err(format!(
            "the state type changed from `{name}` of {size} bytes aligned to \
             {align} to `{new_name}` of {new_size} bytes aligned to {new_align}, \
             so the window keeps its view until a restart"
        ));
    }
    // SAFETY: the patched probe reports the original shape, so the caller
    // promises that the address is a function with this signature, and it
    // is not null.
    Ok(unsafe { std::mem::transmute::<usize, fn(&S) -> E>(address as usize) })
}

#[cfg(test)]
#[path = "hot_tests.rs"]
mod tests;
