//! Timers that run between frames, on the clock their [`App`](crate::App)
//! keeps.
//!
//! [`after`] runs a closure once, [`every`] runs it repeatedly until the
//! returned [`TimerHandle`] is dropped or the owner the timer was created
//! under is cleaned up. Both read the app's clock, not the system clock, so a
//! test that runs the app on a [`VirtualClock`] advances time by hand and
//! observes the exact timers it expects, without sleeping.
//!
//! A timer is registered under the owner that is current when it is created,
//! which is what makes the app find it: every owner chain leads to a root,
//! and the root holds the timer book. Code that runs with no current owner,
//! such as a bare test, creates no timer at all.
//!
//! ```
//! use std::time::Duration;
//!
//! use smudge_core::app::{App, AppConfig, RepaintMode};
//! use smudge_core::geometry::{Color, Size};
//! use smudge_core::reactive::{prelude::*, signal_local};
//! use smudge_core::timer::{self, VirtualClock};
//! use smudge_core::widget::panel;
//! use web_time::Instant;
//!
//! let clock = VirtualClock::new(Instant::now());
//! let (fired, set_fired) = signal_local(false);
//! let mut app = App::with_clock(
//!     AppConfig {
//!         viewport: Size {
//!             width: 100.0,
//!             height: 60.0,
//!         },
//!         scale: 1.0,
//!         clear_color: Color::WHITE,
//!         repaint: RepaintMode::Damage,
//!     },
//!     clock.clone().into(),
//!     || {
//!         timer::after(Duration::from_millis(40), move || set_fired.set(true));
//!         panel()
//!     },
//! );
//!
//! // The timer has not run yet: only the passage of time runs it.
//! clock.advance(Duration::from_millis(39));
//! app.frame();
//! assert!(!fired.get(), "the timer is not due yet");
//!
//! clock.advance(Duration::from_millis(1));
//! app.frame();
//! assert!(fired.get(), "the timer ran when its time was up");
//! ```

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use web_time::Instant;

use crate::reactive::runtime::{self, CurrentOwnerGuard, OwnerData};

/// The clock an [`App`](crate::App) and its timers read the time from.
///
/// [`App::new`](crate::App::new) runs on [`Clock::system`]. A test creates a
/// [`VirtualClock`], hands it to [`App::with_clock`](crate::App::with_clock)
/// and advances its own copy, so timers fire at exact moments without the
/// test sleeping.
#[derive(Clone, Debug)]
pub struct Clock(ClockKind);

/// Where a clock reads its time from.
#[derive(Clone, Debug)]
enum ClockKind {
    /// The operating system's monotonic time.
    System,
    /// A time that moves only when the virtual clock is advanced.
    Virtual(VirtualClock),
}

impl Clock {
    /// Creates a clock that reads the operating system's time.
    pub fn system() -> Clock {
        Clock(ClockKind::System)
    }

    /// Returns the current time of this clock.
    pub fn now(&self) -> Instant {
        match &self.0 {
            ClockKind::System => Instant::now(),
            ClockKind::Virtual(clock) => clock.now(),
        }
    }
}

impl From<VirtualClock> for Clock {
    fn from(clock: VirtualClock) -> Clock {
        Clock(ClockKind::Virtual(clock))
    }
}

/// A clock whose time moves only when it is advanced.
///
/// Clones share one time, so a test keeps a clone after handing the clock to
/// an app and advances the app's time through it.
///
/// ```
/// use std::time::Duration;
///
/// use smudge_core::timer::VirtualClock;
/// use web_time::Instant;
///
/// let start = Instant::now();
/// let clock = VirtualClock::new(start);
/// let shared = clock.clone();
/// clock.advance(Duration::from_millis(16));
/// assert_eq!(shared.now(), start + Duration::from_millis(16));
/// ```
#[derive(Clone, Debug)]
pub struct VirtualClock(Rc<Cell<Instant>>);

impl VirtualClock {
    /// Creates a virtual clock that starts at `start`.
    pub fn new(start: Instant) -> VirtualClock {
        VirtualClock(Rc::new(Cell::new(start)))
    }

    /// Returns the current time of this clock.
    pub fn now(&self) -> Instant {
        self.0.get()
    }

    /// Moves the time of this clock and of every clone forward by `by`.
    pub fn advance(&self, by: Duration) {
        self.0.set(self.0.get() + by);
    }
}

/// One registered timer, shared by the book that fires it and the handles
/// that cancel it.
struct TimerEntry {
    /// The period of a repeating timer, or `None` for a one-shot.
    period: Option<Duration>,
    /// The time the timer fires at next.
    next: Cell<Instant>,
    /// The closure to run. Taken out while it runs, so a callback that drops
    /// its own handle cannot leave the entry borrowed.
    run: RefCell<Option<Box<dyn FnMut()>>>,
    /// Whether the timer still fires. A handle drop or the fire loop clears
    /// this.
    active: Cell<bool>,
    /// The owner the timer was created under. A cleaned up owner stops its
    /// timers, even when a handle is still held elsewhere.
    ///
    /// The owner is checked rather than told: a cleanup hook per timer would
    /// keep every timer an owner ever made alive until the owner goes, and an
    /// input restarts its blink timer on every keystroke.
    owner: Weak<OwnerData>,
}

impl TimerEntry {
    /// Returns whether the timer can still fire: it was not canceled and its
    /// owner was not cleaned up.
    fn is_live(&self) -> bool {
        self.active.get()
            && self
                .owner
                .upgrade()
                .is_some_and(|owner| !owner.cleaned.get())
    }
}

/// A handle that cancels its timer when it is dropped.
///
/// Dropping the handle cancels the timer, and so does cleaning up the owner
/// the timer was created under, whichever happens first. A repeating timer
/// that should outlive the scope it was created in keeps its handle alive for
/// as long as it should run.
#[must_use = "a timer runs until its handle is dropped, so an unused handle \
              cancels it at once"]
pub struct TimerHandle {
    entry: Rc<TimerEntry>,
}

impl Drop for TimerHandle {
    fn drop(&mut self) {
        self.entry.active.set(false);
    }
}

/// The timers of one app.
///
/// The app owns the book and fires due timers at the start of every frame.
/// No borrow of the book is held while a callback runs, so a callback can
/// register and cancel timers freely.
pub(crate) struct TimerBook {
    /// The clock every timer reads.
    pub(crate) clock: Clock,
    /// The timers, in no particular order. Canceled timers are pruned after
    /// each round of firing.
    timers: Vec<Rc<TimerEntry>>,
}

impl TimerBook {
    /// Creates a book that reads `clock`.
    pub(crate) fn new(clock: Clock) -> TimerBook {
        TimerBook {
            clock,
            timers: Vec::new(),
        }
    }

    /// Returns the earliest time a live timer fires at, or `None` when the
    /// book holds none.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.timers
            .iter()
            .filter(|entry| entry.is_live())
            .map(|entry| entry.next.get())
            .min()
    }
}

/// Registers a timer that fires after `delay` and, when `period` is given,
/// again every period after that.
///
/// Returns the entry, or `None` when there is no current owner, the current
/// owner has already been cleaned up, or no app holds a timer book above the
/// owner.
fn register(
    period: Option<Duration>,
    delay: Duration,
    run: Box<dyn FnMut()>,
) -> Option<Rc<TimerEntry>> {
    let owner = runtime::with_runtime(|runtime| runtime.current_owner())?;
    if owner.cleaned.get() {
        return None;
    }
    let book = book_of(&owner)?;
    let now = book.borrow().clock.now();
    let entry = Rc::new(TimerEntry {
        period,
        next: Cell::new(now + delay),
        run: RefCell::new(Some(run)),
        active: Cell::new(true),
        owner: Rc::downgrade(&owner),
    });
    book.borrow_mut().timers.push(Rc::clone(&entry));
    Some(entry)
}

/// Runs every timer that was registered when the round began and whose time
/// has come, once per timer.
///
/// Callbacks run inside the owner they were created under, and one that
/// registers a timer due right now waits for the next round, so a callback
/// that schedules work cannot loop inside one round. A timer that fell more
/// than a period behind fires once and reschedules from the current time, so
/// a slow frame never fires a timer many times to catch up.
pub(crate) fn fire_due(book: &Rc<RefCell<TimerBook>>) {
    let round: Vec<_> = book.borrow().timers.clone();
    let mut fired: Vec<Rc<TimerEntry>> = Vec::new();
    loop {
        let now = book.borrow().clock.now();
        let Some(entry) = due_entry(&round, now, &fired) else {
            break;
        };
        fired.push(Rc::clone(&entry));
        if !run_entry(&entry) {
            entry.active.set(false);
        } else if let Some(period) = entry.period {
            entry.next.set(entry.next.get().max(now) + period);
        }
    }
    book.borrow_mut().timers.retain(|entry| entry.is_live());
}

/// Returns the timer of `timers` that fires at or before `now`, is active and
/// is not in `skip`, picking the one that is due first, or `None` when none
/// is due.
fn due_entry(
    timers: &[Rc<TimerEntry>],
    now: Instant,
    skip: &[Rc<TimerEntry>],
) -> Option<Rc<TimerEntry>> {
    let mut best: Option<Rc<TimerEntry>> = None;
    for entry in timers {
        if !entry.active.get() || skip.iter().any(|seen| Rc::ptr_eq(seen, entry)) {
            continue;
        }
        let next = entry.next.get();
        if next <= now && best.as_ref().is_none_or(|best| next < best.next.get()) {
            best = Some(Rc::clone(entry));
        }
    }
    best
}

/// Runs the callback of `entry` inside its owner, and returns whether the
/// timer may fire again.
///
/// A timer whose owner is gone or cleaned up stops, and so does one whose
/// callback is somehow already running, or that another callback of the round
/// canceled.
fn run_entry(entry: &Rc<TimerEntry>) -> bool {
    let Some(owner) = entry.owner.upgrade().filter(|owner| !owner.cleaned.get()) else {
        return false;
    };
    let Ok(mut run) = entry.run.try_borrow_mut() else {
        return false;
    };
    let Some(mut callback) = run.take() else {
        return false;
    };
    {
        let _guard = CurrentOwnerGuard::enter(&owner);
        callback();
    }
    // The entry stays alive while the callback runs, because the book, the
    // round and this call all hold a handle to it, so putting the closure
    // back cannot race a drop.
    *run = Some(callback);
    entry.period.is_some() && entry.active.get()
}

/// Returns the timer book of the app `owner` belongs to, walking up the owner
/// chain to the root that holds it.
fn book_of(owner: &Rc<OwnerData>) -> Option<Rc<RefCell<TimerBook>>> {
    let mut current = Rc::clone(owner);
    loop {
        let book = current.timer_book.borrow().as_ref().and_then(Weak::upgrade);
        if book.is_some() {
            return book;
        }
        let parent = current.parent.as_ref()?.upgrade()?;
        current = parent;
    }
}

/// Returns the current time of the clock of the app that owns the code
/// running right now, or the system time when no app does.
///
/// Handlers run inside the owner of their node, so input handlers read the
/// app's time this way, which makes double click detection and other timing
/// testable with a virtual clock.
pub fn now() -> Instant {
    let owner = runtime::with_runtime(|runtime| runtime.current_owner());
    match owner.as_ref().and_then(book_of) {
        Some(book) => book.borrow().clock.now(),
        None => Instant::now(),
    }
}

/// Runs `fun` once, after `duration` has passed on the app's clock.
///
/// The timer is created under the current owner and stops when that owner is
/// cleaned up or when `fun` has run, whichever comes first. The closure runs
/// at the start of a frame, inside the owner it was created under, so it can
/// write the signals that owner's bindings read. Code with no current owner,
/// or with no app above it, creates no timer and drops `fun`.
///
/// A duration of zero fires at the start of the next frame.
pub fn after(duration: Duration, fun: impl FnMut() + 'static) {
    register(None, duration, Box::new(fun));
}

/// Runs `fun` every `period` on the app's clock, until the returned handle is
/// dropped or the creating owner is cleaned up.
///
/// The first run comes one period after the timer was created. A timer that
/// falls behind fires once per frame and reschedules from the current time,
/// so a zero period behaves as a request to run once per frame, and every
/// frame with a due timer keeps the app awake. Keep the handle alive for as
/// long as the timer should run; see [`TimerHandle`]. When no timer can be
/// created, the handle that comes back is inert and cancels nothing.
pub fn every(period: Duration, fun: impl FnMut() + 'static) -> TimerHandle {
    let entry = register(Some(period), period, Box::new(fun));
    TimerHandle {
        entry: entry.unwrap_or_else(|| inactive_entry(period)),
    }
}

/// Builds the entry a handle holds when its timer was never registered.
///
/// The entry is inactive and has a dead owner, so the handle costs nothing
/// and dropping it changes nothing.
fn inactive_entry(period: Duration) -> Rc<TimerEntry> {
    Rc::new(TimerEntry {
        period: Some(period),
        next: Cell::new(Instant::now()),
        run: RefCell::new(None),
        active: Cell::new(false),
        owner: Weak::default(),
    })
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
