//! A headless app that speaks the patch wire format, for the tests of
//! `cargo smudge dev`. It announces itself on the builder's socket, applies
//! every patch it receives, and then calls its probes through the installed
//! table, printing which version of each one ran.
//!
//! The probes are called through the addresses `main` captured when the
//! binary was built, and the loop that reports never runs again after a
//! patch, so every lookup uses an address of the original binary, as the
//! wire format requires.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;

use subsecond::JumpTable;

/// The format version, matching the platform crate's hello.
const VERSION: u32 = 2;

/// The frame kinds of the wire format.
const HELLO: u8 = 1;
const PATCH: u8 = 2;
const APPLIED: u8 = 3;
const FAILED: u8 = 4;

#[inline(never)]
fn alpha() -> &'static str {
    "alpha-1"
}

// BETA-FN-START
#[inline(never)]
fn beta() -> &'static str {
    "beta-1"
}
// BETA-FN-END

#[inline(never)]
fn helper(x: u32) -> u32 {
    x + 1
}

/// The function whose address the hello announces, and whose signature a
/// test changes. It is never called, so a signature change compiles.
#[inline(never)]
fn view() -> &'static str {
    "view-1"
}

/// The state probe whose address the hello announces: its type parameter
/// is the state type the builder checks the layout of.
#[inline(never)]
fn state_shape<S>() -> usize {
    std::mem::size_of::<S>()
}

fn main() {
    let Some(socket) = std::env::var_os("SMUDGE_HOT_SOCKET") else {
        eprintln!("fixture: no patch builder socket named, exiting");
        return;
    };
    let build = std::env::var_os("SMUDGE_HOT_BUILD").unwrap_or_default();
    let mut stream = match UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("fixture: cannot reach the patch builder: {error}");
            return;
        }
    };
    let mut hello = Vec::new();
    hello.extend_from_slice(&VERSION.to_le_bytes());
    hello.extend_from_slice(&std::process::id().to_le_bytes());
    hello.extend_from_slice(&(subsecond::aslr_reference() as u64).to_le_bytes());
    hello.extend_from_slice(&(view as *const () as usize as u64).to_le_bytes());
    hello.extend_from_slice(&(state_shape::<()> as fn() -> usize as usize as u64).to_le_bytes());
    hello.extend_from_slice(build.as_bytes());
    if write_frame(&mut stream, HELLO, &hello).is_err() {
        return;
    }

    let (sender, frames) = mpsc::channel();
    let reader = stream.try_clone().expect("fixture: cannot clone the socket");
    std::thread::spawn(move || {
        let mut reader = reader;
        loop {
            match read_frame(&mut reader) {
                Ok(Some(frame)) => {
                    if sender.send(frame).is_err() {
                        return;
                    }
                }
                Ok(None) | Err(_) => return,
            }
        }
    });

    // The addresses of the probes as this binary was built. Every patch's
    // table maps these addresses, so the lookups below stay valid no matter
    // how many patches arrive.
    let alpha_address = alpha as *const () as usize;
    let beta_address = beta as *const () as usize;
    let helper_address = helper as *const () as usize;
    let view_address = view as *const () as usize;

    for (kind, payload) in frames {
        if kind != PATCH {
            continue;
        }
        match apply(&payload) {
            Ok(()) => {
                if write_frame(&mut stream, APPLIED, &[]).is_err() {
                    return;
                }
            }
            Err(reason) => {
                eprintln!("fixture: the patch was not applied: {reason}");
                if write_frame(&mut stream, FAILED, reason.as_bytes()).is_err() {
                    return;
                }
                continue;
            }
        }
        let table = unsafe { subsecond::get_jump_table() };
        let (alpha_value, alpha_hot) = through(table, alpha_address);
        let (helper_value, helper_hot) = through_helper(table, helper_address);
        let (view_value, view_hot) = through(table, view_address);
        println!(
            "fixture alpha={alpha_value} alpha_hot={alpha_hot} helper={helper_value} \
             helper_hot={helper_hot} view={view_value} view_hot={view_hot}"
        );
        // BETA-REPORT-START
        let (beta_value, beta_hot) = through(table, beta_address);
        println!("fixture beta={beta_value} beta_hot={beta_hot}");
        // BETA-REPORT-END
        let _ = io::stdout().flush();
    }
}

/// The runtime addresses of the type probes the last patch kept.
static KEPT: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

/// Whether the last patch kept the type probe at `address`.
// Only the tests that add a report line call it.
#[allow(dead_code)]
fn kept(address: usize) -> bool {
    KEPT.lock()
        .expect("fixture: the kept probes lock")
        .contains(&(address as u64))
}

/// Decodes a patch frame and installs its table.
///
/// # Errors
///
/// Returns why the patch was not applied.
fn apply(payload: &[u8]) -> Result<(), String> {
    let mut rest = payload;
    let aslr_reference = take_u64(&mut rest)?;
    let new_base_address = take_u64(&mut rest)?;
    let count = take_u64(&mut rest)?;
    let map = (0..count)
        .map(|_| Ok((take_u64(&mut rest)?, take_u64(&mut rest)?)))
        .collect::<Result<_, String>>()?;
    let kept_count = take_u64(&mut rest)?;
    let runtime_main = subsecond::aslr_reference() as u64;
    let kept: Vec<u64> = (0..kept_count)
        .map(|_| Ok(take_u64(&mut rest)?.wrapping_sub(aslr_reference).wrapping_add(runtime_main)))
        .collect::<Result<_, String>>()?;
    *KEPT.lock().expect("fixture: the kept probes lock") = kept;
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
    // SAFETY: the builder started this process and built the table against
    // its own hello, as the wire format requires: the library is compiled
    // from this binary's own crates, every old address is this binary's, and
    // every mapped function keeps its signature. The call runs on the main
    // thread, which applies every patch, and its result is answered before
    // the next frame is read.
    unsafe { subsecond::apply_patch(table) }.map_err(|error| error.to_string())
}

/// Calls the function the table maps `address` to, or the address itself
/// when nothing maps it, and answers its value with whether the table
/// carried it.
fn through(table: Option<&JumpTable>, address: usize) -> (&'static str, bool) {
    let mapped = table
        .and_then(|table| table.map.get(&(address as u64)).copied())
        .filter(|&new| new != 0);
    match mapped {
        // SAFETY: the wire format requires every entry to be the new version
        // of the function at the old address, with the same signature.
        Some(new) => {
            let run = unsafe { std::mem::transmute::<usize, fn() -> &'static str>(new as usize) };
            (run(), true)
        }
        // SAFETY: `address` came from a function pointer of this binary, and
        // is turned back into one.
        None => {
            let run = unsafe { std::mem::transmute::<usize, fn() -> &'static str>(address) };
            (run(), false)
        }
    }
}

/// Like [`through`], for a probe that takes and returns a `u32`.
fn through_helper(table: Option<&JumpTable>, address: usize) -> (u32, bool) {
    let mapped = table
        .and_then(|table| table.map.get(&(address as u64)).copied())
        .filter(|&new| new != 0);
    match mapped {
        // SAFETY: the entry maps the helper to its new version, which keeps
        // the signature the running binary calls it with, since a helper
        // whose signature changed is never in the table.
        Some(new) => {
            let run = unsafe { std::mem::transmute::<usize, fn(u32) -> u32>(new as usize) };
            (run(1), true)
        }
        // SAFETY: `address` came from a function pointer of this binary.
        None => {
            let run = unsafe { std::mem::transmute::<usize, fn(u32) -> u32>(address) };
            (run(1), false)
        }
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
    let mut payload = vec![0; length as usize];
    stream.read_exact(&mut payload)?;
    Ok(Some((kind, payload)))
}

/// Writes one frame of `kind` carrying `payload`.
fn write_frame(stream: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

/// Takes a little-endian `u64` off the front of `rest`.
fn take_u64(rest: &mut &[u8]) -> Result<u64, String> {
    if rest.len() < 8 {
        return Err(String::from("the patch ends early"));
    }
    let (bytes, tail) = rest.split_at(8);
    *rest = tail;
    let mut value = [0; 8];
    value.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(value))
}
