//! The `wl_data_device` client that hears the file drags of one window.
//!
//! A drop must arrive on the display the windowing library owns, because
//! the compositor routes the drag to the client of the surface under the
//! pointer. This client therefore adopts that display and binds its own
//! `wl_data_device` beside whatever the windowing library holds: both hear
//! the drags, and the windowing library's, which has none, ignores them.
//!
//! A reading thread follows the shared socket and wakes the loop for every
//! read. The windowing library reads the socket whenever it turns
//! readable, but it skips its dispatch iteration, whose about-to-wait
//! polls this client, when its own queue heard nothing of the bytes: a
//! drag's events would sit until the pointer returned.
//!
//! The compositor names the dragged files only at the drop, so a hover
//! reports no paths and the drop carries them. One window runs per client
//! here, so a drag over any surface of the client is a drag over the
//! window, whatever surface the enter named.
//!
//! The client dispatches on the main thread, between the batches of the
//! windowing library's own loop, and asks the drag source for its file
//! list at the drop, which answers or stalls for at most one second.

use std::ffi::c_void;
use std::io::{ErrorKind, PipeReader, Read};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use smudge_core::geometry::Point;
use smudge_core::input::InputEvent;
use wayland_client::backend::{Backend, WaylandError};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_registry, wl_seat,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// The type a file manager offers its file list under.
const URI_LIST_MIME: &str = "text/uri-list";

/// How long a drop waits for the drag source to send its file list.
///
/// Reading blocks the UI thread, so a source that never answers costs at
/// most this long.
const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// How long the reading thread waits on the socket before it checks whether
/// its client still lives.
const READ_POLL: Duration = Duration::from_millis(200);

/// The size of each read from the pipe a drop receives its list through.
const READ_CHUNK: usize = 16 * 1024;

/// The most bytes a received list may grow to before it is abandoned.
///
/// A list of thousands of files stays far below this, so a source that
/// streams more is broken or hostile and no app can use what it sends.
const READ_LIMIT: usize = 1024 * 1024;

/// The highest `wl_data_device_manager` version the client binds, the
/// first whose offers can agree on a drag action with the source.
const MANAGER_VERSION: u32 = 3;

/// The highest `wl_seat` version the client binds, which is what the
/// clipboard client binds as well.
const SEAT_VERSION: u32 = 5;

/// Opens the drag-and-drop client of a Wayland window, or `None` when the
/// client cannot start, as [`WaylandDnd::open`] documents.
pub(crate) fn open_for_window(
    window: &impl HasDisplayHandle,
    wake: impl Fn() + Send + Sync + 'static,
) -> Option<WaylandDnd> {
    let RawDisplayHandle::Wayland(handle) = window.display_handle().ok()?.as_raw() else {
        return None;
    };
    WaylandDnd::open(handle.display.as_ptr(), wake)
}

/// The drag-and-drop client of one window.
pub(crate) struct WaylandDnd {
    /// The connection the windowing library owns, kept here to flush the
    /// requests the client makes.
    connection: Connection,
    /// The queue the data device events land in, beside the windowing
    /// library's own.
    queue: EventQueue<DndState>,
    /// The state the data device events reach, on the main thread only.
    state: DndState,
    /// Set when the client drops, so the reading thread stops.
    stop: Arc<AtomicBool>,
    /// The reading thread, joined on drop. Open fails when the thread
    /// cannot spawn, so this is always some.
    reader: Option<JoinHandle<()>>,
}

impl WaylandDnd {
    /// Adopts the display the window points at and binds the data device.
    ///
    /// Spawns the reading thread that follows the socket, whose `wake` runs
    /// every time it brought bytes in, so the loop dispatches the drags
    /// while they fly instead of when they end. Returns `None` when the
    /// compositor offers no data device manager of at least
    /// [`MANAGER_VERSION`], no seat at all, or the reading thread cannot
    /// spawn, which leaves the window without file drops.
    pub(crate) fn open(
        display: *mut c_void,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Option<WaylandDnd> {
        // SAFETY: the display is the one behind the window's raw display
        // handle, which the windowing library keeps for the whole run of
        // the window, as long as this client lives. The connection shares
        // the display without owning it, so dropping it never closes what
        // the windowing library owns. This is the one foreign-display call
        // the platform needs, because a drop reaches no other connection.
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init(&connection).ok()?;
        let handle = queue.handle();
        let registry = globals.registry();
        let mut state = DndState::default();
        globals.contents().with_list(|list| {
            for global in list {
                match global.interface.as_str() {
                    "wl_data_device_manager" if global.version >= MANAGER_VERSION => {
                        state.manager =
                            Some(registry.bind(global.name, MANAGER_VERSION, &handle, ()));
                    }
                    "wl_seat" if global.version >= 1 => {
                        let version = global.version.min(SEAT_VERSION);
                        state.seat = Some(registry.bind(global.name, version, &handle, ()));
                    }
                    _ => {}
                }
            }
        });
        if !make_device(&mut state, &handle) {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let connection = connection.clone();
            let stop = Arc::clone(&stop);
            // A client without its reader strands every drag, so a spawn
            // that fails leaves the window without drops instead.
            match std::thread::Builder::new()
                .name(String::from("smudge-dnd"))
                .spawn(move || read_socket(connection, wake, stop))
            {
                Ok(reader) => reader,
                Err(_) => {
                    // The device the client bound must be released like
                    // Drop releases it, or the compositor keeps a bound
                    // data device for a client that holds none.
                    if let Some(device) = state.device.take() {
                        device.release();
                    }
                    return None;
                }
            }
        };
        Some(WaylandDnd {
            connection,
            queue,
            state,
            stop,
            reader: Some(reader),
        })
    }

    /// Dispatches the drags the compositor queued and returns the input
    /// events they make.
    ///
    /// The reading thread fills this queue from the socket the windowing
    /// library owns; dispatching what is queued never waits.
    pub(crate) fn poll(&mut self) -> Vec<InputEvent> {
        if self.queue.dispatch_pending(&mut self.state).is_err() {
            // A dispatch that failed means the connection broke, which ends
            // every further drag. What queued before the failure still
            // reaches the app.
            self.state.offer = None;
        }
        let _ = self.connection.flush();
        std::mem::take(&mut self.state.events)
    }
}

impl Drop for WaylandDnd {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(drag) = self.state.offer.take() {
            drag.offer.destroy();
        }
        if let Some(device) = self.state.device.take() {
            device.release();
        }
    }
}

/// Follows the socket the windowing library owns and wakes the loop for
/// every read, so the drags dispatch while they fly.
///
/// The windowing library reads the socket whenever it turns readable, but
/// it skips the iteration that would dispatch this client's queue when its
/// own queue heard nothing, so this thread reads and wakes in its place.
/// The read itself is libwayland's: it demuxes the bytes to every queue of
/// the display, and the loop's own dispatch moves the client's to its
/// handlers.
fn read_socket(
    connection: Connection,
    wake: impl Fn() + Send + Sync + 'static,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        let Some(guard) = connection.prepare_read() else {
            // Events already sit in the client's queue, and whichever
            // thread read them may have been the windowing library's,
            // which dispatches its own queue alone: wake the loop, whose
            // about-to-wait polls this client, before waiting again.
            wake();
            std::thread::sleep(READ_POLL);
            continue;
        };
        let fd = guard.connection_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        let timeout = Timespec::try_from(READ_POLL).expect("READ_POLL fits a Timespec");
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => continue,
            Err(Errno::INTR) => continue,
            Err(error) => {
                eprintln!("smudge: the file drag reader stopped: {error}");
                break;
            }
            Ok(_) => {}
        }
        match guard.read() {
            // A read that another thread won still brought the bytes to the
            // client's queue, so the loop hears the drag either way.
            Ok(_) => wake(),
            Err(WaylandError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                wake()
            }
            Err(error) => {
                eprintln!("smudge: the file drag reader stopped: {error}");
                break;
            }
        }
    }
}

/// Binds the data device of the seat, once both it and the manager are
/// there.
fn make_device(state: &mut DndState, handle: &QueueHandle<DndState>) -> bool {
    if state.device.is_some() {
        return true;
    }
    let (Some(manager), Some(seat)) = (&state.manager, &state.seat) else {
        return false;
    };
    state.device = Some(manager.get_data_device(seat, handle, ()));
    true
}

/// What one drag over the window carries.
struct DragOffer {
    /// The offer the files come over.
    offer: wl_data_offer::WlDataOffer,
    /// The serial of the enter that started the drag, which an accept
    /// names.
    serial: u32,
    /// Whether the drag carries a file list.
    files: bool,
}

/// The state the data device events reach.
#[derive(Default)]
struct DndState {
    /// The manager the data device comes from.
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    /// The seat the data device hears the drags of.
    seat: Option<wl_seat::WlSeat>,
    /// The data device the compositor routes the drags to.
    device: Option<wl_data_device::WlDataDevice>,
    /// The offer of the drag over the window, when there is one.
    offer: Option<DragOffer>,
    /// Where the pointer was, in window coordinates.
    position: Point,
    /// The input events the drags made, for the window to dispatch.
    events: Vec<InputEvent>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for DndState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The registry was fully read when the client opened; a global that
        // appears later, such as a seat a hotplug brings, joins no window
        // that already runs.
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for DndState {
    fn event(
        _: &mut Self,
        _: &wl_data_device_manager::WlDataDeviceManager,
        _: <wl_data_device_manager::WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for DndState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A drag needs no capabilities of its own; the compositor routes
        // the data device events whatever the seat can do.
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for DndState {
    fn event(
        state: &mut Self,
        _device: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial, x, y, id, ..
            } => {
                state.position = Point {
                    x: x as f32,
                    y: y as f32,
                };
                // A compositor that starts a new drag without a leave ends
                // the old one, and its offer must be destroyed with it.
                if let Some(old) = state.offer.take() {
                    // The app's hover of the drag that ended hears a
                    // cancellation, as if a leave had come first.
                    if old.files {
                        state.events.push(InputEvent::FileHoverCancelled);
                    }
                    old.offer.destroy();
                }
                // The compositor announces the mime types of an offer
                // before it names it here, so whether the drag carries
                // the file list is already known. An offer of none means
                // the drag carries no files, and the app hears nothing
                // until it ends.
                let files = id
                    .as_ref()
                    .and_then(|offer| offer.data::<OfferFiles>())
                    .is_some_and(|files| files.get());
                state.offer = id.map(|offer| DragOffer {
                    offer,
                    serial,
                    files,
                });
                if let Some(drag) = state.offer.as_ref().filter(|drag| drag.files) {
                    drag.offer.accept(drag.serial, Some(URI_LIST_MIME.into()));
                    if drag.offer.version() >= 3 {
                        drag.offer.set_actions(
                            wl_data_device_manager::DndAction::Copy,
                            wl_data_device_manager::DndAction::Copy,
                        );
                    }
                    // The paths are the drag source's to send, and it
                    // sends them at the drop, so the hover carries none
                    // yet.
                    state.events.push(InputEvent::FileHovered {
                        paths: Vec::new(),
                        position: state.position,
                    });
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                state.position = Point {
                    x: x as f32,
                    y: y as f32,
                };
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.offer.take() {
                    offer.offer.destroy();
                    if offer.files {
                        state.events.push(InputEvent::FileHoverCancelled);
                    }
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(offer) = state.offer.take() {
                    let files = offer.files;
                    if files {
                        let paths = receive_files(offer.offer.clone(), conn);
                        // Finish belongs after a received transfer, and a
                        // list with no local file is a transfer that never
                        // arrived, so the reported hover hears a
                        // cancellation instead of a drop.
                        if paths.is_empty() {
                            state.events.push(InputEvent::FileHoverCancelled);
                        } else {
                            // The offer inherits the device's manager version,
                            // so finish's own version-3 requirement holds.
                            offer.offer.finish();
                            state.events.push(InputEvent::FileDropped {
                                paths,
                                position: state.position,
                            });
                        }
                    }
                    offer.offer.destroy();
                }
            }
            // The clipboard's selection offer is the clipboard client's
            // to read, and this client only has to destroy what it
            // created.
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }

    // The compositor creates an offer with its data_offer event; enter,
    // drop and selection only name one that exists.
    event_created_child!(DndState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, OfferFiles::new()),
    ]);
}

/// Whether an offer advertised the file list type.
///
/// Each offer carries its own, because the compositor announces the
/// types of an offer before it names the offer in an enter, and the
/// announcements of the clipboard's selection offers must not count for
/// the drag in the air.
struct OfferFiles(AtomicBool);

impl OfferFiles {
    /// Returns a record of an offer that advertised no type yet.
    fn new() -> OfferFiles {
        OfferFiles(AtomicBool::new(false))
    }

    /// Records that the offer advertised the file list type.
    fn mark(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Returns whether the offer advertised the file list type.
    fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, OfferFiles> for DndState {
    fn event(
        _: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        files: &OfferFiles,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wl_data_offer::Event::Offer { mime_type } = event else {
            return;
        };
        if mime_type == URI_LIST_MIME {
            files.mark();
        }
    }
}

/// Asks the drag source for its file list and reads it.
///
/// The request travels over `conn`, and the source then answers or stalls
/// for at most [`READ_TIMEOUT`]. A list that never arrives or parses to
/// nothing means no drop is reported.
fn receive_files(offer: wl_data_offer::WlDataOffer, conn: &Connection) -> Vec<PathBuf> {
    let Ok((reader, writer)) = std::io::pipe() else {
        return Vec::new();
    };
    offer.receive(URI_LIST_MIME.into(), writer.as_fd());
    drop(writer);
    if conn.flush().is_err() {
        return Vec::new();
    }
    let Some(bytes) = read_to_end(reader, READ_TIMEOUT) else {
        return Vec::new();
    };
    paths_from_uri_list(&String::from_utf8_lossy(&bytes))
}

/// Reads `reader` until the writer closes it.
///
/// Returns `None` when the writer neither finishes nor closes the pipe
/// within `timeout`.
fn read_to_end(mut reader: PipeReader, timeout: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut chunk = [0; READ_CHUNK];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(left) = Timespec::try_from(left) else {
            return None;
        };
        let mut fds = [PollFd::new(&reader, PollFlags::IN)];
        match poll(&mut fds, Some(&left)) {
            Ok(0) => return None,
            Err(Errno::INTR) => continue,
            Err(_) => return None,
            Ok(_) => {}
        }
        match reader.read(&mut chunk) {
            Ok(0) => return Some(out),
            Ok(read) => {
                if out.len() + read > READ_LIMIT {
                    return None;
                }
                out.extend_from_slice(&chunk[..read]);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => continue,
            Err(_) => return None,
        }
    }
}

/// Parses the paths of a `text/uri-list`, the type a file manager offers.
///
/// Every line is one URI. Lines that name no `file` URI, that name a file
/// on another host, or that start with a comment mark yield nothing, and
/// the percent escapes of the rest are decoded. A line that ends in a
/// carriage return keeps its paths, which is the form the protocol writes.
fn paths_from_uri_list(list: &str) -> Vec<PathBuf> {
    list.lines()
        .filter_map(|line| {
            let uri = line.trim_end_matches('\r');
            if uri.is_empty() || uri.starts_with('#') {
                return None;
            }
            let rest = uri.strip_prefix("file://")?;
            // A URI with no authority names a file of this machine, and so
            // does one whose authority is exactly `localhost`. An authority
            // of another machine names a file this app cannot open.
            let path = match rest.strip_prefix("localhost") {
                Some(path) if path.starts_with('/') => path,
                None if rest.starts_with('/') => rest,
                _ => return None,
            };
            if path.is_empty() {
                return None;
            }
            Some(PathBuf::from(percent_decode(path)))
        })
        .collect()
}

/// Decodes the percent escapes of a URI into UTF-8.
///
/// A byte sequence that is not a valid escape stays as it is, so a list
/// that is not quite a list still yields what it can.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[path = "wayland_tests.rs"]
#[cfg(test)]
mod wayland_tests;
