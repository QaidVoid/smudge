//! A clipboard client for Wayland compositors that offer a data-control
//! protocol.
//!
//! The client opens its own connection to the compositor and binds
//! `ext_data_control_manager_v1` when the compositor offers it, or the older
//! `zwlr_data_control_manager_v1` otherwise. Both let a client read and set
//! the selection without a focused window, so a copy stays on the clipboard
//! after the window loses focus, for as long as the clipboard lives.
//!
//! A dispatch thread follows the selection on every seat and serves the
//! text this clipboard copied. A copy sets the selection on every seat. A
//! paste reads the first seat that has a selection, which is the only seat
//! on a single-seat desktop.

use std::io::{ErrorKind, PipeReader, Read};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use smudge_core::clipboard::Clipboard;
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_callback, wl_registry, wl_seat};
use wayland_client::{
    Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle, event_created_child,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1, ext_data_control_manager_v1, ext_data_control_offer_v1,
    ext_data_control_source_v1,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

/// The text types a copy offers and a paste accepts, best first.
///
/// A copy serves its text as UTF-8 under every one of them.
const TEXT_MIMES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
    "STRING",
];

/// How long a paste waits for its request to reach the compositor and for
/// the copying application to send its text, and how long a copy waits for
/// its request to reach the compositor.
///
/// Both block the UI thread, so a compositor or an application that never
/// answers costs at most this long.
const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// The size of each read from the pipe a paste receives its text through.
const READ_CHUNK: usize = 16 * 1024;

/// How long a copy keeps waiting for the pasting application to make room
/// in its pipe before it gives up on that paste.
///
/// Each write that makes progress starts the wait again, so a slow reader
/// still gets the whole text, while a reader that stopped reading frees the
/// thread that serves it.
const SEND_STALL: Duration = Duration::from_secs(5);

/// How long the dispatch thread waits for the compositor to read the
/// requests it could not take yet before the connection counts as failed.
const CONNECTION_STALL: Duration = Duration::from_secs(5);

/// How long a flush pauses when the socket had room and still took nothing,
/// so a connection whose full-socket error never clears is not polled in a
/// busy loop until its deadline.
const FLUSH_PAUSE: Duration = Duration::from_millis(10);

/// The highest `wl_seat` version the client binds, the first with `release`.
const SEAT_VERSION: u32 = 5;

/// The clipboard of a Wayland session, reached through a data-control
/// protocol.
///
/// Dropping it stops its dispatch thread and closes its connection, which
/// clears a selection this clipboard still owns.
pub(crate) struct WaylandClipboard {
    /// The clipboard's own connection to the compositor.
    connection: Connection,
    /// The handle of the queue the dispatch thread reads.
    queue: QueueHandle<State>,
    /// The bound data-control manager, bound once. Compositors keep this
    /// global for the whole session, so its removal is not handled.
    manager: Manager,
    /// The seats, shared with the dispatch thread.
    seats: Arc<Mutex<Vec<Seat>>>,
    /// Whether the dispatch thread still runs. Once it stops, nothing reads
    /// the compositor's events, so copy and paste give up at once.
    alive: Arc<AtomicBool>,
}

impl WaylandClipboard {
    /// Connects to the compositor named by the environment and starts the
    /// dispatch thread.
    ///
    /// Returns `None` when there is no Wayland compositor to connect to, when
    /// it offers no data-control manager, or when the connection fails while
    /// it is set up.
    pub(crate) fn new() -> Option<WaylandClipboard> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut event_queue) = registry_queue_init::<State>(&connection).ok()?;
        let queue = event_queue.handle();
        let manager = match globals.bind(&queue, 1..=1, ()) {
            Ok(manager) => Manager::Ext(manager),
            Err(_) => Manager::Wlr(globals.bind(&queue, 1..=2, ()).ok()?),
        };
        let seats = Arc::new(Mutex::new(Vec::new()));
        let mut state = State {
            manager: manager.clone(),
            seats: Arc::clone(&seats),
            stopped: false,
        };
        for global in globals.contents().clone_list() {
            if global.interface == wl_seat::WlSeat::interface().name {
                state.add_seat(globals.registry(), global.name, global.version, &queue);
            }
        }
        event_queue.roundtrip(&mut state).ok()?;
        let alive = Arc::new(AtomicBool::new(true));
        std::thread::Builder::new()
            .name("smudge-clipboard".into())
            .spawn({
                let connection = connection.clone();
                let alive = Arc::clone(&alive);
                move || {
                    dispatch(&connection, event_queue, state);
                    alive.store(false, Ordering::Release);
                }
            })
            .ok()?;
        Some(WaylandClipboard {
            connection,
            queue,
            manager,
            seats,
            alive,
        })
    }
}

impl Clipboard for WaylandClipboard {
    fn get(&mut self) -> Option<String> {
        if !self.alive.load(Ordering::Acquire) {
            return None;
        }
        let reader = {
            let seats = lock(&self.seats);
            let seat = seats
                .iter()
                .find(|seat| seat.source.is_some() || seat.offer.is_some())?;
            if let Some(source) = &seat.source {
                return source.text().map(String::from);
            }
            let offer = seat.offer.as_ref()?;
            let mime = offer.best_mime()?;
            let (reader, writer) = std::io::pipe().ok()?;
            offer.receive(&mime, &writer);
            reader
        };
        // The request carries its own duplicate of the write end, so with
        // ours dropped the pipe ends when the copying application closes its
        // copy.
        let deadline = Instant::now() + READ_TIMEOUT;
        if !flush_within(&self.connection, READ_TIMEOUT) {
            return None;
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        read_to_end(reader, left).map(decode)
    }

    fn set(&mut self, text: String) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        let text: Arc<str> = text.into();
        for seat in lock(&self.seats).iter_mut() {
            let source = self.manager.create_source(Arc::clone(&text), &self.queue);
            seat.device.set_selection(&source);
            if let Some(previous) = seat.source.replace(source) {
                previous.destroy();
            }
        }
        flush_within(&self.connection, READ_TIMEOUT);
    }
}

impl Drop for WaylandClipboard {
    fn drop(&mut self) {
        // The dispatch thread blocks until an event arrives, so the
        // compositor's answer to this sync is what wakes it to stop. A
        // compositor that never answers leaves the thread blocked, which
        // costs nothing in practice: the clipboard lives as long as its
        // window, and the process ends soon after.
        self.connection.display().sync(&self.queue, ());
        let _ = self.connection.flush();
    }
}

/// Dispatches the clipboard's events until the clipboard is dropped or the
/// connection fails.
fn dispatch(connection: &Connection, mut queue: EventQueue<State>, mut state: State) {
    while !state.stopped {
        match queue.blocking_dispatch(&mut state) {
            Ok(_) => {}
            // The compositor has not yet read the requests already sent, so
            // the flush that starts every dispatch could not write more.
            Err(DispatchError::Backend(WaylandError::Io(error)))
                if error.kind() == ErrorKind::WouldBlock =>
            {
                if !flush_within(connection, CONNECTION_STALL) {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// Flushes the requests sent on `connection`, waiting for the compositor to
/// read them while its socket is full, and returns whether all of them were
/// sent within `timeout`.
///
/// Some versions of the system library turn a full socket into a fatal
/// error that keeps reporting itself as a full socket, so a flush that
/// still cannot write after `timeout` counts as a failed connection.
fn flush_within(connection: &Connection, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut had_room = false;
    loop {
        match connection.flush() {
            Ok(()) => return true,
            Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return false,
        }
        if had_room {
            std::thread::sleep(FLUSH_PAUSE);
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        let Ok(left) = Timespec::try_from(left) else {
            return false;
        };
        let backend = connection.backend();
        let fd = backend.poll_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
        match poll(&mut fds, Some(&left)) {
            Ok(0) => return false,
            Ok(_) => had_room = true,
            Err(Errno::INTR) => had_room = false,
            Err(_) => return false,
        }
    }
}

/// Reads `pipe` until the writer closes it.
///
/// Returns `None` when the writer neither finishes nor closes the pipe
/// within `timeout`, or when reading fails.
fn read_to_end(mut pipe: PipeReader, timeout: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut chunk = [0; READ_CHUNK];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let left = Timespec::try_from(left).ok()?;
        let mut fds = [PollFd::new(&pipe, PollFlags::IN)];
        match poll(&mut fds, Some(&left)) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(_) => return None,
        }
        match pipe.read(&mut chunk) {
            Ok(0) => return Some(bytes),
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// Writes `text` to `fd` on a thread of its own, then closes it.
///
/// A reader that is slow stalls only that thread, not the dispatch thread,
/// and one that stops reading ends it after [`SEND_STALL`]. See
/// [`write_within`].
fn serve(text: Arc<str>, fd: OwnedFd) {
    let _ = std::thread::Builder::new()
        .name("smudge-clipboard-send".into())
        .spawn(move || {
            write_within(&fd, text.as_bytes(), SEND_STALL);
        });
}

/// Writes all of `bytes` to `fd`, waiting at most `stall` at a time for the
/// reader to make room, and returns whether everything was written.
///
/// The pasting application creates the pipe and picks its flags, and some
/// make the write end non-blocking, so `fd` is switched to non-blocking
/// writes either way and waits for room through `poll`.
fn write_within(fd: &OwnedFd, mut bytes: &[u8], stall: Duration) -> bool {
    if rustix::io::ioctl_fionbio(fd, true).is_err() {
        return false;
    }
    let mut deadline = Instant::now() + stall;
    while !bytes.is_empty() {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        let Ok(left) = Timespec::try_from(left) else {
            return false;
        };
        let mut fds = [PollFd::new(fd, PollFlags::OUT)];
        match poll(&mut fds, Some(&left)) {
            Ok(0) => return false,
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(_) => return false,
        }
        match rustix::io::write(fd, bytes) {
            Ok(written) if written > 0 => {
                bytes = &bytes[written..];
                deadline = Instant::now() + stall;
            }
            Ok(_) | Err(Errno::AGAIN | Errno::INTR) => {}
            Err(_) => return false,
        }
    }
    true
}

/// Decodes the bytes a paste received as text.
///
/// Invalid UTF-8 is replaced with U+FFFD rather than refusing the paste, so
/// text from an application that mislabels its encoding still arrives.
fn decode(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}

/// The best text type an offer advertised so far, the user data of every
/// offer.
///
/// It holds the type's rank in [`TEXT_MIMES`] and the type exactly as the
/// offer spelled it, because the copying application may compare the type
/// a paste asks for byte by byte against the ones it advertised.
struct BestMime(Mutex<Option<(usize, String)>>);

impl BestMime {
    /// Returns a record of an offer that advertised no type yet.
    fn new() -> BestMime {
        BestMime(Mutex::new(None))
    }

    /// Records that the offer advertised `mime`.
    fn advertise(&self, mime: &str) {
        let Some(rank) = TEXT_MIMES
            .iter()
            .position(|text| text.eq_ignore_ascii_case(mime))
        else {
            return;
        };
        let mut best = lock(&self.0);
        if best.as_ref().is_none_or(|(kept, _)| rank < *kept) {
            *best = Some((rank, mime.to_owned()));
        }
    }

    /// Returns the best text type advertised, spelled as the offer spelled
    /// it, or `None` when the offer advertised no text type.
    fn get(&self) -> Option<String> {
        lock(&self.0).as_ref().map(|(_, mime)| mime.clone())
    }
}

/// Locks `mutex`, carrying on past a poisoned lock.
///
/// Nothing panics while it holds one of the clipboard's locks, so the data
/// behind a poisoned lock is still whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One seat and its selection.
struct Seat {
    /// The registry name of the seat's global, which its removal refers to.
    name: u32,
    /// The seat.
    seat: wl_seat::WlSeat,
    /// The seat's data-control device.
    device: Device,
    /// The seat's current selection, when it has one.
    offer: Option<Offer>,
    /// The source this clipboard set as the seat's selection, while it is.
    source: Option<Source>,
}

impl Seat {
    /// Destroys the seat's protocol objects.
    fn destroy(self) {
        self.device.destroy();
        if let Some(offer) = self.offer {
            offer.destroy();
        }
        if let Some(source) = self.source {
            source.destroy();
        }
        if self.seat.version() >= SEAT_VERSION {
            self.seat.release();
        }
    }
}

/// The dispatch thread's state.
struct State {
    /// The bound data-control manager, for the devices of new seats.
    manager: Manager,
    /// The seats, shared with the clipboard.
    seats: Arc<Mutex<Vec<Seat>>>,
    /// Whether the clipboard was dropped, which stops the dispatch thread.
    stopped: bool,
}

impl State {
    /// Binds the seat global `name` and gets its data-control device.
    fn add_seat(
        &self,
        registry: &wl_registry::WlRegistry,
        name: u32,
        version: u32,
        queue: &QueueHandle<State>,
    ) {
        let seat: wl_seat::WlSeat = registry.bind(name, version.min(SEAT_VERSION), queue, ());
        let device = self.manager.get_device(&seat, name, queue);
        lock(&self.seats).push(Seat {
            name,
            seat,
            device,
            offer: None,
            source: None,
        });
    }

    /// Forgets the seat whose global is `name`, if there is one.
    fn remove_seat(&self, name: u32) {
        let mut seats = lock(&self.seats);
        if let Some(index) = seats.iter().position(|seat| seat.name == name) {
            seats.swap_remove(index).destroy();
        }
    }

    /// Records `offer` as the selection of the seat whose global is `name`.
    fn selection(&self, name: u32, offer: Option<Offer>) {
        let mut seats = lock(&self.seats);
        match seats.iter_mut().find(|seat| seat.name == name) {
            Some(seat) => {
                if let Some(previous) = std::mem::replace(&mut seat.offer, offer) {
                    previous.destroy();
                }
            }
            None => {
                if let Some(offer) = offer {
                    offer.destroy();
                }
            }
        }
    }

    /// Destroys `source`, which another selection replaced, and drops its
    /// text.
    fn cancelled(&self, source: Source) {
        for seat in lock(&self.seats).iter_mut() {
            if seat.source.as_ref() == Some(&source) {
                seat.source = None;
            }
        }
        source.destroy();
    }
}

/// The data-control manager the compositor offers.
#[derive(Clone)]
enum Manager {
    /// The standard `ext-data-control-v1` manager.
    Ext(ext_data_control_manager_v1::ExtDataControlManagerV1),
    /// The older `wlr-data-control-unstable-v1` manager.
    Wlr(zwlr_data_control_manager_v1::ZwlrDataControlManagerV1),
}

impl Manager {
    /// Gets the data-control device of `seat`, whose global is `name`.
    fn get_device(&self, seat: &wl_seat::WlSeat, name: u32, queue: &QueueHandle<State>) -> Device {
        match self {
            Manager::Ext(manager) => Device::Ext(manager.get_data_device(seat, queue, name)),
            Manager::Wlr(manager) => Device::Wlr(manager.get_data_device(seat, queue, name)),
        }
    }

    /// Creates a source that serves `text` under every accepted text type.
    fn create_source(&self, text: Arc<str>, queue: &QueueHandle<State>) -> Source {
        let source = match self {
            Manager::Ext(manager) => Source::Ext(manager.create_data_source(queue, text)),
            Manager::Wlr(manager) => Source::Wlr(manager.create_data_source(queue, text)),
        };
        for mime in TEXT_MIMES {
            match &source {
                Source::Ext(source) => source.offer(mime.into()),
                Source::Wlr(source) => source.offer(mime.into()),
            }
        }
        source
    }
}

/// A seat's data-control device.
enum Device {
    /// A device of the `ext` protocol.
    Ext(ext_data_control_device_v1::ExtDataControlDeviceV1),
    /// A device of the `wlr` protocol.
    Wlr(zwlr_data_control_device_v1::ZwlrDataControlDeviceV1),
}

impl Device {
    /// Makes `source` the seat's selection.
    fn set_selection(&self, source: &Source) {
        match (self, source) {
            (Device::Ext(device), Source::Ext(source)) => device.set_selection(Some(source)),
            (Device::Wlr(device), Source::Wlr(source)) => device.set_selection(Some(source)),
            _ => {}
        }
    }

    /// Destroys the device.
    fn destroy(&self) {
        match self {
            Device::Ext(device) => device.destroy(),
            Device::Wlr(device) => device.destroy(),
        }
    }
}

/// A selection another client, or this clipboard, offers.
enum Offer {
    /// An offer of the `ext` protocol.
    Ext(ext_data_control_offer_v1::ExtDataControlOfferV1),
    /// An offer of the `wlr` protocol.
    Wlr(zwlr_data_control_offer_v1::ZwlrDataControlOfferV1),
}

impl Offer {
    /// Returns the best text type the offer advertised, if any.
    fn best_mime(&self) -> Option<String> {
        match self {
            Offer::Ext(offer) => offer.data::<BestMime>(),
            Offer::Wlr(offer) => offer.data::<BestMime>(),
        }?
        .get()
    }

    /// Asks the offering client to write the selection as `mime` to `fd`.
    fn receive(&self, mime: &str, fd: &impl AsFd) {
        match self {
            Offer::Ext(offer) => offer.receive(mime.into(), fd.as_fd()),
            Offer::Wlr(offer) => offer.receive(mime.into(), fd.as_fd()),
        }
    }

    /// Destroys the offer.
    fn destroy(&self) {
        match self {
            Offer::Ext(offer) => offer.destroy(),
            Offer::Wlr(offer) => offer.destroy(),
        }
    }
}

/// A selection this clipboard offers.
///
/// Its user data holds the text it serves.
#[derive(PartialEq)]
enum Source {
    /// A source of the `ext` protocol.
    Ext(ext_data_control_source_v1::ExtDataControlSourceV1),
    /// A source of the `wlr` protocol.
    Wlr(zwlr_data_control_source_v1::ZwlrDataControlSourceV1),
}

impl Source {
    /// Returns the text the source serves.
    fn text(&self) -> Option<&str> {
        match self {
            Source::Ext(source) => source.data::<Arc<str>>(),
            Source::Wlr(source) => source.data::<Arc<str>>(),
        }
        .map(|text| &**text)
    }

    /// Destroys the source.
    fn destroy(&self) {
        match self {
            Source::Ext(source) => source.destroy(),
            Source::Wlr(source) => source.destroy(),
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_seat::WlSeat::interface().name => {
                state.add_seat(registry, name, version, queue);
            }
            wl_registry::Event::GlobalRemove { name } => state.remove_seat(name),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.stopped = true;
    }
}

/// Implements the dispatch of one data-control protocol, whose four
/// interfaces have the same events in both protocols.
macro_rules! dispatch_data_control {
    (
        $variant:ident,
        $manager:ident::$Manager:ident,
        $device:ident::$Device:ident,
        $offer:ident::$Offer:ident,
        $source:ident::$Source:ident $(,)?
    ) => {
        impl Dispatch<$manager::$Manager, ()> for State {
            fn event(
                _: &mut Self,
                _: &$manager::$Manager,
                _: $manager::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<$device::$Device, u32> for State {
            fn event(
                state: &mut Self,
                _: &$device::$Device,
                event: $device::Event,
                seat: &u32,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    $device::Event::Selection { id } => {
                        state.selection(*seat, id.map(Offer::$variant));
                    }
                    $device::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
                    $device::Event::Finished => state.remove_seat(*seat),
                    _ => {}
                }
            }

            event_created_child!(State, $device::$Device, [
                $device::EVT_DATA_OFFER_OPCODE => ($offer::$Offer, BestMime::new()),
            ]);
        }

        impl Dispatch<$offer::$Offer, BestMime> for State {
            fn event(
                _: &mut Self,
                _: &$offer::$Offer,
                event: $offer::Event,
                best: &BestMime,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if let $offer::Event::Offer { mime_type } = event {
                    best.advertise(&mime_type);
                }
            }
        }

        impl Dispatch<$source::$Source, Arc<str>> for State {
            fn event(
                state: &mut Self,
                source: &$source::$Source,
                event: $source::Event,
                text: &Arc<str>,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    $source::Event::Send { fd, .. } => serve(Arc::clone(text), fd),
                    $source::Event::Cancelled => state.cancelled(Source::$variant(source.clone())),
                    _ => {}
                }
            }
        }
    };
}

dispatch_data_control!(
    Ext,
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_device_v1::ExtDataControlDeviceV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1,
    ext_data_control_source_v1::ExtDataControlSourceV1,
);

dispatch_data_control!(
    Wlr,
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
);

#[cfg(test)]
#[path = "wayland_tests.rs"]
mod tests;
