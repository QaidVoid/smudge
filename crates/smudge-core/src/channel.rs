//! Messages from other threads to the application's user interface thread.
//!
//! Signals live on the thread that runs the reactive runtime, so a network
//! reply, a line of subprocess output or a file notification cannot write
//! one. [`channel`] hands the other side a [`UiSender`]: `send` puts the
//! value into a queue that crosses threads and, on the first value of a
//! burst, asks the window for the frame that applies it. The frame drains
//! the queue before anything else and runs the handler on the application's
//! thread, inside the owner the channel was created under, so the handler
//! writes signals like any other application code.
//!
//! The receiving side lives as long as that owner: a row that unmounts, a
//! view that hides or an application that closes stops its channels, and
//! every later `send` returns the value in a [`Closed`]. A handler that
//! panics closes its channel the same way. Creating a channel
//! where no application is above the current owner is a programming error:
//! debug builds assert, and the sender that comes back delivers nothing.
//!
//! # Examples
//!
//! A build closure creates a channel and keeps its sender somewhere the rest
//! of the application, or another thread, can reach:
//!
//! ```
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! use smudge_core::app::{App, AppConfig, RepaintMode};
//! use smudge_core::channel;
//! use smudge_core::geometry::{Color, Size};
//! use smudge_core::reactive::{prelude::*, signal_local};
//! use smudge_core::widget::panel;
//!
//! let (total, set_total) = signal_local(0u32);
//! let holder = Rc::new(RefCell::new(None));
//! let slot = Rc::clone(&holder);
//! let mut app = App::new(
//!     AppConfig {
//!         viewport: Size {
//!             width: 120.0,
//!             height: 60.0,
//!         },
//!         scale: 1.0,
//!         clear_color: Color::WHITE,
//!         repaint: RepaintMode::Damage,
//!     },
//!     move || {
//!         *slot.borrow_mut() = Some(channel::channel(move |add: u32| {
//!             set_total.update(|total| *total += add);
//!         }));
//!         panel()
//!     },
//! );
//! app.frame();
//!
//! let sender = holder
//!     .borrow()
//!     .clone()
//!     .expect("the build closure stored its sender");
//! sender.send(3).unwrap();
//! sender.send(4).unwrap();
//! app.frame();
//! assert_eq!(total.get(), 7, "both values arrived, in send order");
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use crate::reactive::Owner;
use crate::reactive::runtime::{self, CurrentOwnerGuard, OwnerData};

/// The callback that asks the platform for the frame that drains a channel.
type RemoteWaker = Arc<dyn Fn() + Send + Sync>;

/// Why a poisoned queue lock is an invariant break: the lock is held only to
/// push, pop, close and read the length, and none of those runs user code.
const QUEUE_POISONED: &str = "a channel queue lock is poisoned, although it \
     is never held while user code runs";

/// Why a poisoned waker lock is an invariant break, for the same reason.
const WAKER_POISONED: &str = "a channel waker lock is poisoned, although it \
     is never held while user code runs";

/// Opens a channel from any thread to the application's user interface.
///
/// The channel must be created where an application is above the current
/// owner, which is the case inside a build closure, inside a `show` child or
/// inside a `for_each` row. The receiving side is tied to the owner that is
/// current when `channel` runs: the handler runs inside it, and cleaning it
/// up stops the channel. With no application above the current owner the
/// call is a programming error: debug builds panic, and release builds get a
/// sender whose every `send` returns [`Closed`].
///
/// The handler runs on the application's thread at the start of the next
/// frame, once per value, in send order. A frame delivers at most the values
/// the queue held when the drain reached it, so a handler that sends to its own
/// channel defers the new value to the following frame instead of starving
/// the frame.
///
/// # Examples
///
/// Send from another thread and apply the value with the next frame. The
/// thread signals when the value is queued, so the frame after that is the
/// one that applies it:
///
/// ```
/// use std::sync::mpsc;
///
/// use smudge_core::app::{App, AppConfig, RepaintMode};
/// use smudge_core::channel;
/// use smudge_core::geometry::{Color, Size};
/// use smudge_core::reactive::{prelude::*, signal_local};
/// use smudge_core::widget::panel;
///
/// let (seen, set_seen) = signal_local(0u32);
/// let (done, queued) = mpsc::channel();
/// let mut app = App::new(
///     AppConfig {
///         viewport: Size {
///             width: 120.0,
///             height: 60.0,
///         },
///         scale: 1.0,
///         clear_color: Color::WHITE,
///         repaint: RepaintMode::Damage,
///     },
///     move || {
///         let sender = channel::channel(move |value: u32| set_seen.set(value));
///         std::thread::spawn(move || {
///             sender.send(7).unwrap();
///             done.send(()).unwrap();
///         });
///         panel()
///     },
/// );
///
/// queued
///     .recv()
///     .expect("the thread sends one value and then signals");
/// app.frame();
/// assert_eq!(seen.get(), 7);
/// ```
pub fn channel<T: Send + 'static>(handler: impl FnMut(T) + 'static) -> UiSender<T> {
    let owner = runtime::with_runtime(|runtime| runtime.current_owner())
        .filter(|owner| !owner.cleaned.get());
    let book = owner.as_ref().and_then(book_of);
    let Some((owner, book)) = owner.zip(book) else {
        debug_assert!(
            false,
            "channel() ran where no application owner is current, so no \
             frame will ever run its handler and the sender it returns \
             delivers nothing. Create channels inside an application, for \
             example in a build closure"
        );
        return UiSender {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::Closed),
                waker: Mutex::new(None),
            }),
        };
    };

    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue::Open(VecDeque::new())),
        waker: Mutex::new(book.borrow().waker.clone()),
    });

    let mut handler = Box::new(handler) as Box<dyn FnMut(T)>;
    let draining = Arc::clone(&shared);
    let run: Drain = Box::new(move |bound: usize| {
        for _ in 0..bound {
            let value = {
                let mut queue = draining.queue.lock().expect(QUEUE_POISONED);
                match &mut *queue {
                    Queue::Open(values) => values.pop_front(),
                    Queue::Closed => None,
                }
            };
            let Some(value) = value else {
                break;
            };
            handler(value);
        }
    });

    let id = {
        let mut book = book.borrow_mut();
        let id = book.next_id;
        book.next_id += 1;
        book.receivers.push(Rc::new(Receiver {
            id,
            owner: Rc::downgrade(&owner),
            run: RefCell::new(Some(run)),
            channel: Arc::clone(&shared) as Arc<dyn ErasedChannel>,
        }));
        id
    };

    let weak_book = Rc::downgrade(&book);
    let closing: Arc<dyn ErasedChannel> = shared.clone();
    Owner::on_cleanup(move || {
        closing.close();
        if let Some(book) = weak_book.upgrade() {
            book.borrow_mut().receivers.retain(|other| other.id != id);
        }
    });

    UiSender { shared }
}

/// The sending half of a [`channel`].
///
/// Cloning is cheap and every clone sends into the same queue. A sender is
/// [`Send`] and [`Sync`] when the value type is, so a thread that owns one
/// can hand clones to its workers.
#[must_use = "a channel delivers only through its sender, so a dropped \
              sender delivers nothing"]
pub struct UiSender<T> {
    /// The queue and the waker, shared with the receiving application.
    shared: Arc<Shared<T>>,
}

impl<T> Clone for UiSender<T> {
    fn clone(&self) -> Self {
        UiSender {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> std::fmt::Debug for UiSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("UiSender")
    }
}

impl<T: Send> UiSender<T> {
    /// Hands `value` to the application's next frame.
    ///
    /// Only the first value of a burst asks the window for a frame, so a
    /// sender that writes in a loop costs one wake up per burst. A value
    /// that lands while a frame is draining the channel waits for the frame
    /// after it, which the drain asks for.
    ///
    /// # Errors
    ///
    /// Returns the value in a [`Closed`] when the receiving side is gone:
    /// the owner that created the channel was cleaned up, the application
    /// was dropped, or the handler panicked. Nothing is queued then.
    ///
    /// # Panics
    ///
    /// Panics when the queue's lock is poisoned, which only a panic in
    /// another sender at the same moment can cause: the lock is never held
    /// while user code runs.
    pub fn send(&self, value: T) -> Result<(), Closed<T>> {
        let first = {
            let mut queue = self.shared.queue.lock().expect(QUEUE_POISONED);
            match &mut *queue {
                Queue::Closed => return Err(Closed { value }),
                Queue::Open(values) => {
                    let first = values.is_empty();
                    values.push_back(value);
                    first
                }
            }
        };
        if first {
            self.shared.wake();
        }
        Ok(())
    }
}

/// The value a send could not deliver.
///
/// The receiving side was gone: the owner that created the channel was
/// cleaned up, the application was dropped, or the handler panicked. The
/// value comes back, so the sender can keep it, send it elsewhere or drop it.
#[derive(Debug)]
pub struct Closed<T> {
    /// The value that was not delivered.
    pub value: T,
}

/// What a channel's queue holds.
enum Queue<T> {
    /// The channel delivers, and holds these values for the next frame.
    Open(VecDeque<T>),
    /// The receiving side is gone; every send comes back.
    Closed,
}

/// The state of one channel that crosses threads.
///
/// The queue and the waker are all that leaves the application's thread. The
/// handler and every other channel handle stay on it.
struct Shared<T> {
    /// The values waiting for the next frame, and whether the channel still
    /// delivers.
    queue: Mutex<Queue<T>>,
    /// The callback that asks the platform for the draining frame. The
    /// application's platform sets it; a headless app has none, and its
    /// next `frame` call finds the values on its own.
    waker: Mutex<Option<RemoteWaker>>,
}

/// The parts of a channel the book uses without knowing the value type.
trait ErasedChannel {
    /// Returns how many values wait to be delivered.
    fn pending(&self) -> usize;

    /// Stops the channel and drops the values that were never delivered.
    fn close(&self);

    /// Replaces the callback that a first value wakes.
    fn set_waker(&self, waker: RemoteWaker);

    /// Runs the waker, without holding the queue lock, for values that a
    /// drain in progress left behind.
    fn wake(&self);
}

impl<T> ErasedChannel for Shared<T> {
    fn pending(&self) -> usize {
        let queue = self.queue.lock().expect(QUEUE_POISONED);
        match &*queue {
            Queue::Open(values) => values.len(),
            Queue::Closed => 0,
        }
    }

    fn close(&self) {
        // Replacing the queue drops the values that were never delivered.
        *self.queue.lock().expect(QUEUE_POISONED) = Queue::Closed;
    }

    fn set_waker(&self, waker: RemoteWaker) {
        *self.waker.lock().expect(WAKER_POISONED) = Some(waker);
    }

    fn wake(&self) {
        let waker = self.waker.lock().expect(WAKER_POISONED).clone();
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// One channel's receiving side, as the book holds it.
struct Receiver {
    /// The receiver's id, which the cleanup that removes it matches.
    id: u64,
    /// The owner the channel was created under. The handler runs inside it,
    /// and its cleanup removes the receiver and closes the channel.
    owner: Weak<OwnerData>,
    /// Delivers up to the bound it is given. Taken out while it runs, so a
    /// drain that reenters cannot borrow it twice.
    run: RefCell<Option<Drain>>,
    /// The channel's shared state, for the book's checks and the close.
    channel: Arc<dyn ErasedChannel>,
}

/// Delivers up to a bound of one channel's queued values.
type Drain = Box<dyn FnMut(usize)>;

/// The receiving sides of one application's channels.
///
/// The application owns the book and drains it at the start of every frame.
/// A channel registers its receiver here and the cleanup of the owner that
/// created it removes it, so the book holds only channels whose owner still
/// exists.
#[derive(Default)]
pub(crate) struct ChannelBook {
    /// The waker that channels created from now on start with.
    waker: Option<RemoteWaker>,
    /// The id the next receiver gets.
    next_id: u64,
    /// The receivers, in creation order, so a frame drains the channels in
    /// the order the application created them.
    receivers: Vec<Rc<Receiver>>,
}

/// Drains every channel of the application at the start of a frame.
///
/// Each receiver whose owner still exists delivers the values the queue held
/// when the drain reached it, in send order, inside that owner. A value that
/// lands while the drain runs waits for the next frame, and the drain asks
/// for that frame through the waker once it ends.
pub(crate) fn drain_all(book: &Rc<RefCell<ChannelBook>>) {
    let round: Vec<_> = book.borrow().receivers.clone();
    for receiver in round {
        let Some(owner) = receiver
            .owner
            .upgrade()
            .filter(|owner| !owner.cleaned.get())
        else {
            continue;
        };
        let bound = receiver.channel.pending();
        if bound == 0 {
            continue;
        }
        let Ok(mut slot) = receiver.run.try_borrow_mut() else {
            continue;
        };
        let Some(mut run) = slot.take() else {
            continue;
        };
        drop(slot);
        {
            let _guard = CurrentOwnerGuard::enter(&owner);
            let closes_on_unwind = CloseOnUnwind(&*receiver.channel);
            run(bound);
            std::mem::forget(closes_on_unwind);
        }
        // The round and the book hold the receiver while the closure runs,
        // so putting the closure back cannot race its drop.
        *receiver.run.borrow_mut() = Some(run);
        if receiver.channel.pending() > 0 {
            receiver.channel.wake();
        }
    }
}

/// Closes a channel when its handler unwinds out of a drain.
///
/// The drain takes the handler out while it runs and puts it back only when
/// it returns, so a handler that panicked is gone. Closing the channel makes
/// later sends come back as [`Closed`] and leaves no value queued that no
/// frame can deliver, which would otherwise keep the app asking for frames.
struct CloseOnUnwind<'a>(&'a dyn ErasedChannel);

impl Drop for CloseOnUnwind<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Returns whether any channel of the application holds an undelivered
/// value.
pub(crate) fn has_pending(book: &Rc<RefCell<ChannelBook>>) -> bool {
    book.borrow()
        .receivers
        .iter()
        .any(|receiver| receiver.channel.pending() > 0)
}

/// Sets the waker of the application's channels, the ones it has and the
/// ones created later.
pub(crate) fn install_waker(book: &Rc<RefCell<ChannelBook>>, waker: RemoteWaker) {
    let mut borrowed = book.borrow_mut();
    for receiver in &borrowed.receivers {
        receiver.channel.set_waker(Arc::clone(&waker));
    }
    borrowed.waker = Some(waker);
}

/// Returns the channel book of the application `owner` belongs to, walking
/// up the owner chain to the root that holds it.
fn book_of(owner: &Rc<OwnerData>) -> Option<Rc<RefCell<ChannelBook>>> {
    let mut current = Rc::clone(owner);
    loop {
        let book = current
            .channel_book
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade);
        if book.is_some() {
            return book;
        }
        let parent = current.parent.as_ref()?.upgrade()?;
        current = parent;
    }
}

#[cfg(test)]
#[path = "channel_tests.rs"]
mod tests;
