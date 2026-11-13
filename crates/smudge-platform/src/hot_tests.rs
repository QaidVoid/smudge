//! Tests for the hot reload receiver's wire format, the view lookup and the
//! state guard.
//!
//! Each wire test plays the patch builder on a socket of its own, and the
//! test's thread plays the thread that runs the app. No test installs a
//! patch: a library that cannot be loaded is refused before anything is
//! installed, so the process keeps running its original code. The lookup
//! and the guard are checked against tables made by hand.

use std::cell::Cell;
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::{AnyElement, IntoElement};
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::px;
use smudge_core::widget::panel;

use super::{
    APPLIED, FAILED, HELLO, MAX_PAYLOAD, PATCH, Patch, Shape, current, decode_patch, patched,
    read_frame, start_from, state_shape, write_frame,
};
use subsecond::JumpTable;

/// A socket path of its own for each test, removed when dropped.
struct Socket(PathBuf);

impl Socket {
    /// Binds a fresh socket path and returns it with its listener.
    fn bind() -> (Socket, UnixListener) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "smudge-hot-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("the test socket binds");
        (Socket(path), listener)
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The runtime address of `main` the tests report, since a test binary does
/// not export `main` and would report 0.
const MAIN: usize = 0x7f00_1000;

/// Starts a receiver on a fresh socket, as an app under a builder would,
/// and returns the builder's end of the connection, the receiver's thread
/// and the patches it delivers to the app's thread.
fn connected(build: &str) -> (Socket, UnixStream, JoinHandle<()>, mpsc::Receiver<Patch>) {
    let (socket, listener) = Socket::bind();
    let (deliver, delivered) = mpsc::channel();
    let receiver = start_from(
        Some(OsString::from(&socket.0)),
        Some(OsString::from(build)),
        MAIN,
        move |patch| {
            let _ = deliver.send(patch);
        },
    )
    .expect("a reachable socket starts the receiver");
    let (stream, _) = listener.accept().expect("the receiver connects");
    (socket, stream, receiver, delivered)
}

/// Sends a patch with a library that cannot load, and returns the patch
/// the app's thread receives for it.
fn send_patch(builder: &mut UnixStream, delivered: &mpsc::Receiver<Patch>, lib: &str) -> Patch {
    let payload = patch_payload(0x1000, 0x2000, &[(0x1100, 0x2100)], lib);
    write_frame(builder, PATCH, &payload).expect("the patch is sent");
    delivered.recv().expect("the receiver delivers the patch")
}

/// Encodes a patch frame's payload the way a builder does.
fn patch_payload(
    aslr_reference: u64,
    new_base_address: u64,
    entries: &[(u64, u64)],
    lib: &str,
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&aslr_reference.to_le_bytes());
    payload.extend_from_slice(&new_base_address.to_le_bytes());
    payload.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for (old, new) in entries {
        payload.extend_from_slice(&old.to_le_bytes());
        payload.extend_from_slice(&new.to_le_bytes());
    }
    payload.extend_from_slice(lib.as_bytes());
    payload
}

/// Reads the next frame the app sent, which must exist.
fn next_frame(stream: &mut UnixStream) -> (u8, Vec<u8>) {
    read_frame(stream)
        .expect("the app's frame reads")
        .expect("the app sent a frame")
}

/// Splits a hello payload into its version, process id, address of `main`
/// and build id.
fn hello_fields(payload: &[u8]) -> (u32, u32, u64, Vec<u8>) {
    let (version, rest) = payload.split_first_chunk::<4>().expect("a version");
    let (pid, rest) = rest.split_first_chunk::<4>().expect("a process id");
    let (main, build) = rest.split_first_chunk::<8>().expect("an address of main");
    (
        u32::from_le_bytes(*version),
        u32::from_le_bytes(*pid),
        u64::from_le_bytes(*main),
        build.to_vec(),
    )
}

#[test]
fn without_the_socket_variable_nothing_starts() {
    let receiver = start_from(None, Some(OsString::from("7")), MAIN, |_| {});

    assert!(receiver.is_none(), "no socket must start no thread");
}

#[test]
fn an_unreachable_socket_starts_nothing() {
    let (socket, listener) = Socket::bind();
    let path = OsString::from(&socket.0);
    drop(listener);
    drop(socket);

    let receiver = start_from(Some(path), None, MAIN, |_| {});

    assert!(
        receiver.is_none(),
        "a socket nobody listens on must leave the app running without hot reload"
    );
}

#[test]
fn the_hello_reports_the_version_the_process_main_and_the_build() {
    let (_socket, mut builder, receiver, _) = connected("build-42");

    let (kind, payload) = next_frame(&mut builder);
    assert_eq!(kind, HELLO, "the first frame introduces the app");
    let (version, pid, main, build) = hello_fields(&payload);
    assert_eq!(version, 1);
    assert_eq!(pid, std::process::id());
    assert_eq!(main, MAIN as u64);
    assert_eq!(build, b"build-42");

    drop(builder);
    receiver
        .join()
        .expect("the receiver ends quietly when the builder closes the socket");
}

#[test]
fn a_patch_whose_library_cannot_load_is_refused_and_remounts_nothing() {
    let (_socket, mut builder, receiver, delivered) = connected("1");
    next_frame(&mut builder);

    let patch = send_patch(
        &mut builder,
        &delivered,
        "/nonexistent/smudge-hot/patch-1.so",
    );
    let remounted = Cell::new(false);
    patch.apply(|| {
        remounted.set(true);
        Ok(())
    });

    let (kind, reason) = next_frame(&mut builder);
    assert_eq!(kind, FAILED, "a library that cannot load is refused");
    let reason = String::from_utf8(reason).expect("the reason is UTF-8");
    assert!(
        reason.contains("patch-1.so"),
        "the reason names the library: {reason}"
    );
    assert!(!remounted.get(), "a refused patch remounts nothing");

    drop(builder);
    receiver.join().expect("the receiver ends quietly");
}

#[test]
fn a_patch_the_app_refuses_is_answered_with_the_reason() {
    let (_socket, mut builder, receiver, delivered) = connected("1");
    next_frame(&mut builder);

    send_patch(&mut builder, &delivered, "/lib.so").refuse("the app is closing");

    let (kind, reason) = next_frame(&mut builder);
    assert_eq!(kind, FAILED);
    assert_eq!(reason, b"the app is closing");

    drop(builder);
    receiver.join().expect("the receiver ends quietly");
}

#[test]
fn a_patch_the_app_drops_unanswered_is_answered_as_failed() {
    let (_socket, mut builder, receiver, delivered) = connected("1");
    next_frame(&mut builder);

    drop(send_patch(&mut builder, &delivered, "/lib.so"));

    let (kind, reason) = next_frame(&mut builder);
    assert_eq!(
        kind, FAILED,
        "a patch a closed loop never took is not applied"
    );
    assert_eq!(reason, b"the app closed before it applied the patch");

    drop(builder);
    receiver.join().expect("the receiver ends quietly");
}

#[test]
fn a_malformed_patch_is_refused_without_reaching_the_app_and_the_next_frame_is_still_read() {
    let (_socket, mut builder, receiver, delivered) = connected("1");
    next_frame(&mut builder);

    let mut cut = patch_payload(0x1000, 0x2000, &[(0x1100, 0x2100)], "");
    cut.truncate(30);
    write_frame(&mut builder, PATCH, &cut).expect("the cut patch is sent");
    let (kind, reason) = next_frame(&mut builder);
    assert_eq!(kind, FAILED);
    assert_eq!(reason, b"the patch ends early");

    let unnamed = patch_payload(0x1000, 0x2000, &[], "");
    write_frame(&mut builder, PATCH, &unnamed).expect("the unnamed patch is sent");
    let (kind, reason) = next_frame(&mut builder);
    assert_eq!(kind, FAILED, "the stream stays in step after a bad patch");
    assert_eq!(reason, b"the patch names no library");

    drop(builder);
    receiver.join().expect("the receiver ends quietly");
    assert!(
        delivered.try_recv().is_err(),
        "a patch that does not decode never reaches the app"
    );
}

#[test]
fn a_frame_of_an_unknown_kind_is_skipped() {
    let (_socket, mut builder, receiver, _) = connected("1");
    next_frame(&mut builder);

    write_frame(&mut builder, 9, b"from a newer builder").expect("the frame is sent");
    write_frame(&mut builder, APPLIED, &[]).expect("the frame is sent");
    write_frame(&mut builder, PATCH, &patch_payload(0, 0, &[], "")).expect("the patch is sent");

    let (kind, reason) = next_frame(&mut builder);
    assert_eq!(kind, FAILED, "only the patch is answered");
    assert_eq!(reason, b"the patch names no library");

    drop(builder);
    receiver.join().expect("the receiver ends quietly");
}

#[test]
fn an_oversized_frame_ends_the_receiver_without_reading_it() {
    let (_socket, mut builder, receiver, _) = connected("1");
    next_frame(&mut builder);

    let mut header = vec![PATCH];
    header.extend_from_slice(&(MAX_PAYLOAD + 1).to_le_bytes());
    std::io::Write::write_all(&mut builder, &header).expect("the header is sent");

    receiver
        .join()
        .expect("the receiver stops instead of allocating the announced payload");
    let mut rest = Vec::new();
    builder
        .read_to_end(&mut rest)
        .expect("the app's end is closed");
    assert!(rest.is_empty(), "nothing is answered");
}

#[test]
fn a_patch_frame_decodes_into_the_jump_table_it_describes() {
    let payload = patch_payload(
        0x5_5000,
        0x7_0000,
        &[(0x5_5100, 0x7_0100), (0x5_5200, 0x7_0200)],
        "/target/smudge/patch-3.so",
    );

    let table = decode_patch(&payload).expect("a whole patch decodes");

    assert_eq!(table.lib, PathBuf::from("/target/smudge/patch-3.so"));
    assert_eq!(table.aslr_reference, 0x5_5000);
    assert_eq!(table.new_base_address, 0x7_0000);
    assert_eq!(table.map.len(), 2);
    assert_eq!(table.map.get(&0x5_5100), Some(&0x7_0100));
    assert_eq!(table.map.get(&0x5_5200), Some(&0x7_0200));
}

#[test]
fn an_entry_count_beyond_the_payload_is_refused() {
    let mut payload = patch_payload(0, 0, &[(1, 2)], "/lib.so");
    payload[16..24].copy_from_slice(&u64::MAX.to_le_bytes());

    assert_eq!(
        decode_patch(&payload).map(|_| ()),
        Err(String::from("the patch ends early"))
    );
}

/// The state a test app keeps outside its build.
struct State {
    /// The height every view binds its panel to.
    height: RwSignal<f32>,
}

/// The view as the binary was built.
fn view_before(state: &State) -> AnyElement {
    let height = state.height.clone();
    panel()
        .child(panel().width(px(30.0)).height(move || px(height.get())))
        .into_any()
}

/// The view as a patch rebuilt it, with a new width.
fn view_after(state: &State) -> AnyElement {
    let height = state.height.clone();
    panel()
        .child(panel().width(px(70.0)).height(move || px(height.get())))
        .into_any()
}

/// Returns the width and the height of the inner panel of `app`.
fn inner_size(app: &App) -> (f32, f32) {
    let mut sizes = Vec::new();
    app.visit_nodes(|_, _, rect| sizes.push((rect.x1 - rect.x0, rect.y1 - rect.y0)));
    sizes[1]
}

/// The same state type under another name.
struct Renamed {
    /// The field `State` has.
    _height: RwSignal<f32>,
}

/// A table that maps each pair's first function address to its second.
fn table(entries: &[(usize, usize)]) -> JumpTable {
    JumpTable {
        lib: PathBuf::new(),
        map: entries
            .iter()
            .map(|&(old, new)| (old as u64, new as u64))
            .collect(),
        aslr_reference: 0,
        new_base_address: 0,
        ifunc_count: 0,
    }
}

/// The address of a view, as a table keys it.
fn at(view: fn(&State) -> AnyElement) -> usize {
    view as usize
}

/// The address of a probe, as a table keys it.
fn probe_at(probe: fn() -> Shape) -> usize {
    probe as usize
}

/// The original probe for `State`, as `current` uses it.
fn probe() -> fn() -> Shape {
    state_shape::<State>
}

/// A new version of the probe that reports the original shape.
fn same_shape() -> Shape {
    state_shape::<State>()
}

/// A new version of the probe that reports a larger state.
fn grown() -> Shape {
    let (size, align, name) = state_shape::<State>();
    (size + 8, align, name)
}

/// A new version of the probe that reports a more strictly aligned state.
fn realigned() -> Shape {
    let (size, align, name) = state_shape::<State>();
    (size, align * 2, name)
}

/// Returns the view `table` leads to from `view_before`, with the probe of
/// `State`.
fn lookup(table: &JumpTable) -> Result<fn(&State) -> AnyElement, String> {
    // SAFETY: the tables of these tests map the view only to a view of the
    // same signature, and the probe only to functions of its signature.
    unsafe { patched(Some(table), view_before, probe()) }
}

#[test]
fn with_no_patch_applied_the_view_is_the_original() {
    let view: fn(&State) -> AnyElement = view_before;

    let current = current(view).expect("no patch keeps the view");

    assert!(std::ptr::fn_addr_eq(current, view));
}

#[test]
fn a_patch_that_keeps_the_state_shape_runs_the_patched_view() {
    let table = table(&[
        (at(view_before), at(view_after)),
        (probe_at(probe()), probe_at(same_shape)),
    ]);

    let view = lookup(&table).expect("an unchanged state lets the patched view run");

    assert!(std::ptr::fn_addr_eq(
        view,
        view_after as fn(&State) -> AnyElement
    ));
}

#[test]
fn a_patch_that_changes_the_state_size_or_alignment_keeps_the_view() {
    for changed in [grown as fn() -> Shape, realigned] {
        let table = table(&[
            (at(view_before), at(view_after)),
            (probe_at(probe()), probe_at(changed)),
        ]);

        let reason = lookup(&table).expect_err("a new state layout must not reach the view");

        assert!(
            reason.contains("restart"),
            "the reason asks for a restart: {reason}"
        );
    }
}

#[test]
fn a_patch_that_renames_the_state_type_keeps_the_view() {
    let table = table(&[
        (at(view_before), at(view_after)),
        (probe_at(probe()), probe_at(state_shape::<Renamed>)),
    ]);

    let reason = lookup(&table).expect_err("a renamed state type must not reach the view");

    assert!(
        reason.contains("Renamed"),
        "the reason names the new type: {reason}"
    );
}

#[test]
fn a_patch_built_with_another_state_type_keeps_the_view() {
    let table = table(&[(at(view_before), at(view_after))]);

    let reason = lookup(&table).expect_err("a view whose state probe is gone must not run");

    assert!(
        reason.contains("restart"),
        "the reason asks for a restart: {reason}"
    );
}

#[test]
fn a_patch_that_leaves_the_view_alone_keeps_the_original_whatever_the_state() {
    let table = table(&[(probe_at(probe()), probe_at(grown))]);

    let view = lookup(&table).expect("the original view reads the original state");

    assert!(std::ptr::fn_addr_eq(
        view,
        view_before as fn(&State) -> AnyElement
    ));
}

#[test]
fn a_remount_through_the_patched_view_shows_the_new_code_and_keeps_the_state() {
    let original: fn(&State) -> AnyElement = view_before;
    let table = table(&[
        (at(view_before), at(view_after)),
        (probe_at(probe()), probe_at(same_shape)),
    ]);
    let state = State {
        height: RwSignal::new(10.0),
    };
    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 120.0,
                height: 80.0,
            },
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        || original(&state),
    );
    app.frame();
    assert_eq!(inner_size(&app), (30.0, 10.0));
    state.height.set(25.0);

    let view = lookup(&table).expect("an unchanged state lets the patched view run");
    app.remount(|| view(&state));
    app.frame();

    assert_eq!(
        inner_size(&app),
        (70.0, 25.0),
        "the new width comes from the patched view, the height from the kept state"
    );
}

#[test]
fn a_binary_without_an_exported_main_does_not_connect() {
    let (socket, listener) = Socket::bind();
    listener
        .set_nonblocking(true)
        .expect("the listener turns non-blocking");

    let receiver = start_from(Some(OsString::from(&socket.0)), None, 0, |_| {});

    assert!(receiver.is_none(), "no main to place patches against");
    assert!(
        listener.accept().is_err(),
        "the app never connected to the builder"
    );
}
