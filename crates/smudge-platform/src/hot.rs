//! The receiver that brings hot patches to a running app, and the lookup
//! that finds the patched version of a view.
//!
//! A patch builder that runs the app sets [`HOT_SOCKET_ENVIRONMENT`] to the
//! path of a Unix stream socket it listens on. The receiver connects to it,
//! introduces the app, and hands every jump table the builder sends to the
//! thread that runs the app, which applies it and answers. Without the
//! variable, nothing here runs: no connection and no thread.
//!
//! A patch is installed in [`Patch::apply`] and the jump table is read in
//! [`current`], and both run only on the thread that runs the event loop. The
//! table's store and load therefore never race, and a patch library is
//! loaded only while no app code runs.

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
/// [`crate::run_view`] with the `hot-reload` feature on Linux connects to it
/// and applies the patches it receives. Without the variable the app never
/// looks for a builder.
///
/// # Wire format
///
/// This is version 1 of the format. Both sides send frames: one byte for the
/// frame's kind, the payload's length in bytes as a little-endian `u32`, and
/// then the payload. Every number inside a payload is little-endian too.
///
/// | Kind | Sent by | Payload |
/// |---|---|---|
/// | 1, hello | the app | the format version as a `u32`, the process id as a `u32`, the runtime address of `main` as a `u64`, and then the build id, the value of [`HOT_BUILD_ENVIRONMENT`], to the end |
/// | 2, patch | the builder | the address of `main` in the binary as linked, as a `u64`, the address of `main` in the patch library as linked, as a `u64`, the entry count as a `u64`, that many pairs of `u64` mapping the address of a function in the binary as linked to the address of its new version in the patch library as linked, and then the path of the patch library to the end |
/// | 3, applied | the app | empty |
/// | 4, failed | the app | why the patch was not applied, in UTF-8 |
///
/// The app sends one hello right after it connects. It answers every patch,
/// in order, with applied or failed. The app skips a frame of a kind it does
/// not know. It stops reading when the builder closes the socket, or when a
/// frame's header is broken or announces a payload over 64 MiB, because the
/// stream can no longer be followed.
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
/// check it: symbol names do not record signatures. When the view's own
/// signature changed, the builder reports that the change needs a restart
/// instead of sending the patch.
///
/// A builder must not send a patch that changes the layout of a type whose
/// values outlive the build: the state type and every type its values reach,
/// directly or through signals, `Rc`, `Box` or collections, and the types of
/// values created in `main`. Those values were laid out by the old code, and
/// the app's own guard sees only the outermost state type.
///
/// Each patch replaces the one before it. Its library and its table hold
/// every function changed since the app started, not only those changed
/// since the last patch, and always the crate that defines the view and the
/// crate that calls `run_view`, where the state guard is compiled, because
/// installing a table drops the previous one.
///
/// A patch gets fresh, empty copies of the thread-locals defined in the
/// crates it rebuilds. It must therefore never rebuild a crate whose
/// thread-locals the running app relies on, which always includes the core
/// and the text crate of this library: their reactive runtime and text
/// system would start empty in the patched code. A thread-local of the app's
/// own rebuilt crates restarts from its initial value in the patched code.
/// Likewise, every function and static the patch did not change resolves to
/// the running process's own instance: a patch library must not carry a
/// private copy of a crate it did not rebuild, such as a second reactive
/// runtime, because its code would then read that copy's empty state.
///
/// Only the thread that runs the event loop reads the table: the window
/// looks up the view there, and the app must not look up a hot function on
/// another thread. The table is published without synchronization for
/// other threads, so a read elsewhere could see it half written. The app's binary must export
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
const VERSION: u32 = 1;

/// The kind of the frame that introduces the app.
const HELLO: u8 = 1;

/// The kind of the frame that carries a jump table.
const PATCH: u8 = 2;

/// The kind of the frame that reports a patch as applied.
const APPLIED: u8 = 3;

/// The kind of the frame that reports a patch as refused.
const FAILED: u8 = 4;

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
    /// Where the outcome goes: `Ok` for applied, or why it was not.
    answer: mpsc::Sender<Result<(), String>>,
}

impl Patch {
    /// Loads the patch library, installs the table, runs `then` once the
    /// table is installed, and answers the builder with the outcome.
    ///
    /// Call it only on the thread that runs the event loop, the one thread
    /// that reads the table through [`current`]. `then` rebuilds the root
    /// through the patched view, and its error is the answer when it fails.
    pub(crate) fn apply(self, then: impl FnOnce() -> Result<(), String>) {
        let outcome = install(self.table).and_then(|()| then());
        let _ = self.answer.send(outcome);
    }

    /// Answers the builder that the patch was not applied, for `reason`,
    /// without loading anything.
    pub(crate) fn refuse(self, reason: &str) {
        let _ = self.answer.send(Err(String::from(reason)));
    }
}

/// Connects to the patch builder the environment names, and returns the
/// receiver's thread.
///
/// `deliver` runs on the receiver's thread for every patch that decodes. It
/// must hand the patch to the thread that runs the event loop, which applies
/// or refuses it. The receiver waits for that answer before it reads the
/// next frame. Returns `None` when [`HOT_SOCKET_ENVIRONMENT`] is unset, and
/// when the socket cannot be reached, which it reports on standard error.
pub(crate) fn start(deliver: impl Fn(Patch) + Send + 'static) -> Option<JoinHandle<()>> {
    start_from(
        std::env::var_os(HOT_SOCKET_ENVIRONMENT),
        std::env::var_os(HOT_BUILD_ENVIRONMENT),
        subsecond::aslr_reference(),
        deliver,
    )
}

/// Connects to `socket` like [`start`], with the values of the environment
/// variables and the runtime address of `main` passed in.
///
/// An address of 0 means the binary does not export `main` in its dynamic
/// symbol table, so no patch could be placed against it. Hot reload is then
/// off, and the app says so instead of connecting.
fn start_from(
    socket: Option<OsString>,
    build: Option<OsString>,
    main: usize,
    deliver: impl Fn(Patch) + Send + 'static,
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
    match connect(&socket, build.as_encoded_bytes(), main, deliver) {
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
    deliver: impl Fn(Patch) + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    let mut stream = UnixStream::connect(socket)?;
    let mut hello = Vec::with_capacity(16 + build.len());
    hello.extend_from_slice(&VERSION.to_le_bytes());
    hello.extend_from_slice(&std::process::id().to_le_bytes());
    hello.extend_from_slice(&(main as u64).to_le_bytes());
    hello.extend_from_slice(build);
    write_frame(&mut stream, HELLO, &hello)?;
    std::thread::Builder::new()
        .name(String::from("smudge-hot-reload"))
        .spawn(move || receive(stream, deliver))
}

/// Delivers every patch that arrives on `stream` and answers each one, until
/// the builder closes the socket or the stream breaks.
fn receive(mut stream: UnixStream, deliver: impl Fn(Patch)) {
    loop {
        let (kind, payload) = match read_frame(&mut stream) {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(error) => {
                eprintln!("smudge: hot reload stopped: {error}");
                return;
            }
        };
        if kind != PATCH {
            continue;
        }
        let outcome = decode_patch(&payload).and_then(|table| {
            let (answer, answered) = mpsc::channel();
            deliver(Patch { table, answer });
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
    // requires. This runs on the thread that runs the event loop, the only
    // thread that reads the table, as the wire format requires of the app,
    // so the
    // relaxed store that publishes it is ordered before every later read by
    // program order, no read holds the previous table across this call, and
    // the library's loading runs while no app code does. A view whose state
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

/// Reads the jump table a patch frame carries.
fn decode_patch(payload: &[u8]) -> Result<JumpTable, String> {
    let mut rest = payload;
    let aslr_reference = take_u64(&mut rest)?;
    let new_base_address = take_u64(&mut rest)?;
    let count = take_u64(&mut rest)?;
    let map = (0..count)
        .map(|_| Ok((take_u64(&mut rest)?, take_u64(&mut rest)?)))
        .collect::<Result<_, String>>()?;
    if rest.is_empty() {
        return Err(String::from("the patch names no library"));
    }
    Ok(JumpTable {
        lib: PathBuf::from(OsString::from_vec(rest.to_vec())),
        map,
        aslr_reference,
        new_base_address,
        ifunc_count: 0,
    })
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
pub(crate) type Shape = (usize, usize, &'static str);

/// Returns the shape of `S`, as this instance was compiled.
///
/// This is the probe of the state guard. [`current`] is generic over the
/// state and `run_view` instantiates it with the app's state type, so this
/// function's instance for that type is compiled in the crate that calls
/// `run_view`, which is not always the crate that defines the view. A patch
/// that rebuilds that crate maps the instance to its new
/// version like any other function, and the new version reports the state
/// type as the new code sees it. When the new code no longer uses the type,
/// the instance is gone from the patch, and the table maps it to nothing.
#[inline(never)]
fn state_shape<S>() -> Shape {
    (
        std::mem::size_of::<S>(),
        std::mem::align_of::<S>(),
        std::any::type_name::<S>(),
    )
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
    // symbol. The guard in `patched` refuses a view whose state type changed
    // its size, its alignment or its name, or was replaced by another type.
    // It sees only that outermost type: a change that keeps all three, such
    // as reordered fields of one size, or a change to a type the state
    // reaches through a signal or a pointer, passes the guard. Running the
    // view after such a change, or after a patch from a builder that breaks
    // the signature or the layout rule of the wire format, is undefined
    // behavior, which `run_view` documents as a change that needs a
    // restart.
    unsafe { patched(table, view, state_shape::<S>) }
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
             calls `run_view`. The window keeps its view until a restart"
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
