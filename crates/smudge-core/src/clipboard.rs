//! The clipboard an application's widgets copy text through.
//!
//! An [`App`](crate::App) holds one clipboard, installed with
//! [`App::set_clipboard`](crate::App::set_clipboard). The windowing layer
//! installs one that reaches the system clipboard; a test installs a
//! [`MemoryClipboard`], which keeps its text in memory and shares it through
//! [`MemoryClipboard::clone`]. Widget code reaches whichever one is installed
//! with [`with`], which finds the clipboard of the app the running code
//! belongs to, the same way timers find their clock.
//!
//! ```
//! use smudge_core::clipboard::{Clipboard, MemoryClipboard};
//!
//! let mut clipboard = MemoryClipboard::default();
//! clipboard.set(String::from("copied"));
//! assert_eq!(clipboard.get().as_deref(), Some("copied"));
//! ```

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::reactive::runtime;

/// The clipboard text is copied and pasted through.
///
/// A clipboard holds at most one piece of text. Putting text on it replaces
/// what it held; reading it leaves the text there.
pub trait Clipboard {
    /// Returns the text on the clipboard, or `None` when it holds none or
    /// cannot be read.
    fn get(&mut self) -> Option<String>;

    /// Puts `text` on the clipboard, replacing what it held.
    ///
    /// Implementations may drop the text when the operating system refuses
    /// the write, for example while another application holds the clipboard.
    fn set(&mut self, text: String);
}

/// A clipboard that keeps its text in memory.
///
/// A test installs one through
/// [`App::set_clipboard`](crate::App::set_clipboard) and keeps a clone, which
/// shares the same text, so the test can seed the clipboard before a paste or
/// read it back after a copy. This is also the clipboard of a platform with
/// no system clipboard to reach.
///
/// # Examples
///
/// ```
/// use smudge_core::clipboard::{Clipboard, MemoryClipboard};
///
/// let mut clipboard = MemoryClipboard::default();
/// let mut clone = clipboard.clone();
///
/// clipboard.set(String::from("text"));
/// assert_eq!(clone.get().as_deref(), Some("text"));
/// ```
#[derive(Clone, Default)]
pub struct MemoryClipboard {
    /// The text the clipboard holds, shared with every clone.
    text: Rc<RefCell<Option<String>>>,
}

impl Clipboard for MemoryClipboard {
    fn get(&mut self) -> Option<String> {
        self.text.borrow().clone()
    }

    fn set(&mut self, text: String) {
        *self.text.borrow_mut() = Some(text);
    }
}

/// Runs `f` with the clipboard of the app that owns the code running now.
///
/// Handlers run inside the owner of their node, so an input's copy and paste
/// keys reach the app's clipboard this way. Returns `None` when the running
/// code belongs to no app, when the app has no clipboard installed, or when
/// an outer `with` call holds the clipboard already. The caller should then
/// do nothing.
pub fn with<R>(f: impl FnOnce(&mut dyn Clipboard) -> R) -> Option<R> {
    let owner = runtime::with_runtime(|runtime| runtime.current_owner())?;
    let slot = clipboard_of(&owner)?;
    let mut slot = slot.try_borrow_mut().ok()?;
    Some(f(&mut **slot))
}

/// The shared slot one app's clipboard lives in, held strongly by the app and
/// weakly by every owner of its tree.
pub(crate) type Slot = Rc<RefCell<Box<dyn Clipboard>>>;

/// A weak view of a clipboard slot, as an owner holds it.
pub(crate) type WeakSlot = Weak<RefCell<Box<dyn Clipboard>>>;

/// Returns the clipboard of the app `owner` belongs to, walking up the owner
/// chain to the root that holds it.
fn clipboard_of(owner: &Rc<runtime::OwnerData>) -> Option<Slot> {
    let mut current = Rc::clone(owner);
    loop {
        let slot = current.clipboard.borrow().as_ref().and_then(Weak::upgrade);
        if slot.is_some() {
            return slot;
        }
        let parent = current.parent.as_ref()?.upgrade()?;
        current = parent;
    }
}

/// Stores `clipboard` as the clipboard of the app whose root owner is
/// `root`, replacing any clipboard it had.
///
/// The app keeps the strong reference, so the clipboard lives as long as the
/// app does; owners hold only weak references.
pub(crate) fn install(root: &Rc<runtime::OwnerData>, clipboard: Box<dyn Clipboard>) -> Slot {
    let slot: Slot = Rc::new(RefCell::new(clipboard));
    root.clipboard.replace(Some(Rc::downgrade(&slot)));
    slot
}
#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
