//! The accessibility projection: the roles and names widgets report and the
//! tree the [`App`](crate::app::App) exposes to an accessibility backend.
//!
//! A widget opts into the projection by returning [`Semantics`] from
//! [`Widget::semantics`](crate::widget::Widget::semantics), and an element can
//! override the role, the label, the checked state and the value range with
//! the [`role`](crate::element::Element::role),
//! [`accessible_label`](crate::element::Element::accessible_label),
//! [`accessible_checked`](crate::element::Element::accessible_checked) and
//! [`accessible_range`](crate::element::Element::accessible_range) builders.
//! An [`Announcer`] attached to an element has a screen reader speak
//! messages no widget shows. An accessibility backend reads the projected
//! tree through the app and the changes since the last frame; this module
//! defines the vocabulary the two sides exchange. It stays backend-neutral:
//! mapping it onto a specific accessibility protocol is a backend's job.

use std::cell::RefCell;
use std::rc::Rc;

use crate::geometry::Rect;
use crate::reactive::RwSignal;
use crate::reactive::prelude::*;
use crate::tree::WidgetId;

/// The kind of control a projected node represents.
///
/// The set covers the widgets this release projects. A widget without a role
/// here is left out of the projection, and its children attach to the nearest
/// projected ancestor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// The window the tree fills. The backend adds this as the root.
    Window,
    /// A plain container that groups its children.
    Group,
    /// A control a press activates.
    Button,
    /// Static text.
    Label,
    /// A single-line editable text field.
    TextInput,
    /// A single-line field for a password, whose value and text show one
    /// bullet per character typed.
    PasswordInput,
    /// A multi-line editable text field.
    MultilineTextInput,
    /// A viewport that clips and scrolls its content.
    ScrollView,
    /// A container whose children are list items.
    List,
    /// One entry of a list.
    ListItem,
    /// A divider between two panes that moves to resize them.
    Splitter,
    /// A box that is checked, unchecked or mixed, which a press toggles. Its
    /// state is [`Semantics::checked`], without which a screen reader
    /// announces no state. One without a press handler reads as read only.
    CheckBox,
    /// A switch that is on or off, which a press toggles. Its state is
    /// [`Semantics::checked`], without which a screen reader announces no
    /// state. One without a press handler reads as read only.
    Switch,
    /// One choice of a set of which exactly one is chosen. Its state is
    /// [`Semantics::checked`], without which a screen reader announces no
    /// state. One without a press handler reads as read only.
    RadioButton,
    /// A link that a press follows. One with no name of its own is named by
    /// its text.
    Link,
    /// A heading over the content after it. Its level is
    /// [`Semantics::level`], and one with no name of its own is named by its
    /// text.
    Heading,
    /// A bar that shows how far a task has come. Its progress is
    /// [`Semantics::range`]. Without a range it reads as busy, with no known
    /// progress.
    ProgressBar,
    /// A bar of menus across the top of a window or a view.
    MenuBar,
    /// A list of choices that opens from a menu bar, a button or a right
    /// press, and closes once one is chosen.
    Menu,
    /// One choice of a menu, or a menu of a menu bar.
    MenuItem,
    /// A choice of a menu that is checked or not, which choosing toggles.
    /// Its state is [`Semantics::checked`].
    MenuItemCheckBox,
    /// A field that shows one choice and opens a list of the others. Its
    /// choice is its [`Semantics::value`].
    ComboBox,
    /// A list of choices to pick from.
    ListBox,
    /// One choice of a list box. Whether it is the one picked is
    /// [`Semantics::selected`].
    ListBoxOption,
    /// A short text that describes the node it shows over.
    Tooltip,
    /// A control that picks a number in a range by dragging a thumb along
    /// a track. Its number is [`Semantics::range`].
    Slider,
    /// A field that holds a number in a range, which steps up and down. Its
    /// number is [`Semantics::range`].
    SpinButton,
    /// A row of tabs, each of which shows one panel.
    TabList,
    /// One tab of a tab list. Whether its panel shows is
    /// [`Semantics::selected`].
    Tab,
    /// The panel a tab shows.
    TabPanel,
    /// A dialog that holds the keyboard over the rest of the window. It
    /// reports as modal, and while it shows a screen reader reaches nothing
    /// behind it.
    Dialog,
}

/// Whether a checkbox or a switch is checked.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Checked {
    /// Checked, or on.
    True,
    /// Not checked, or off.
    False,
    /// Neither, as a checkbox over a set of boxes some of which are checked.
    Mixed,
}

impl From<bool> for Checked {
    fn from(checked: bool) -> Checked {
        if checked {
            Checked::True
        } else {
            Checked::False
        }
    }
}

/// A number and the range it moves in, such as a progress bar's progress.
///
/// The three numbers are finite and `min` is at most `max`. A range that is
/// not reads as no known value, and a value outside its range reads as the
/// nearer end.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ValueRange {
    /// The current value, which lies between `min` and `max`.
    pub value: f64,
    /// The least the value can be.
    pub min: f64,
    /// The most the value can be.
    pub max: f64,
}

/// The accessibility description a widget reports about itself.
///
/// Return one from [`Widget::semantics`](crate::widget::Widget::semantics) to
/// place the widget in the projection. [`Semantics::new`] leaves everything
/// but the role unset.
#[derive(Clone, Debug, PartialEq)]
pub struct Semantics {
    /// The kind of control the widget is.
    pub role: Role,
    /// The accessible name: what a screen reader announces first.
    pub label: Option<String>,
    /// The current value: for a text field its content, for a label its text.
    pub value: Option<String>,
    /// Whether a checkbox or a switch is checked, or `None` for a control
    /// that is neither.
    pub checked: Option<Checked>,
    /// The number a control shows and the range it moves in, such as a
    /// progress bar's progress, or `None` for a control without one.
    pub range: Option<ValueRange>,
    /// The node's place from zero among the items of the set around it,
    /// such as a row of a list, or `None` for a node that is no item of a
    /// set. A screen reader announces it with [`Semantics::set_size`] of the
    /// nearest ancestor that has one, so place 39 in a set of 3,000 is
    /// announced as "item 40 of 3,000".
    pub position: Option<usize>,
    /// How many items the set the node holds has, the ones not on screen
    /// included, such as the rows of a list, or `None` for a node that holds
    /// no set.
    pub set_size: Option<usize>,
    /// The level of a heading, from 1 for the top level down, or `None` for
    /// a node that is no heading.
    pub level: Option<usize>,
    /// The widget's text, a run per stretch of a line, in reading order, so
    /// a screen reader can read it by character, word and line. Each run's
    /// bounds are relative to the widget's content box, where its text is
    /// laid out. Empty for a widget that shows no text.
    pub text_runs: Vec<TextRun>,
    /// The selection in the widget's text, or the caret when its ends
    /// meet, or `None` for a widget with no selection.
    pub text_selection: Option<TextSelection>,
    /// The byte range of each link in the widget's text, in order and apart
    /// from one another, which a screen reader reaches as a link of its own
    /// and presses through [`Widget::link_action`](crate::widget::Widget::link_action)
    /// by its index here.
    pub links: Vec<std::ops::Range<usize>>,
    /// A longer description than the name, such as a tooltip's text, which
    /// a screen reader announces after the name and the role, or `None`.
    pub description: Option<String>,
    /// Whether the node is the picked choice of the set around it, such as
    /// an option of a list box, or `None` for a node that is no choice.
    pub selected: Option<bool>,
    /// Whether what the node opens, such as a menu or a list, is open, or
    /// `None` for a node that opens nothing.
    pub expanded: Option<bool>,
    /// Whether the node holds a value the app rejects.
    pub invalid: bool,
}

/// A selection in a widget's text, between two places in the text its
/// [`TextRun`]s hold together.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextSelection {
    /// The end the selection was made from, which stays put as it grows.
    pub anchor: TextOffset,
    /// The end that moves as the selection grows, where the caret is.
    pub focus: TextOffset,
}

/// A place in a widget's text.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextOffset {
    /// The byte offset into the text, on a character boundary. The length
    /// of the text names its end.
    pub byte: usize,
    /// Whether the place sits at the end of the line before `byte` rather
    /// than at the start of the line after it, which tells the two apart
    /// where a line wraps: a caret moved to the end of a wrapped line sits
    /// there.
    pub ends_line: bool,
}

/// The most characters one [`TextRun`] holds, since a screen reader indexes
/// the characters of a run with a byte. A longer line splits into several
/// runs.
pub const TEXT_RUN_CAP: usize = 255;

/// One stretch of a line of a widget's text, as a screen reader reads it.
///
/// A line is one run, or several when it changes direction or font or is
/// longer than [`TEXT_RUN_CAP`] characters, the later ones going on with
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// Where the run's text starts in the widget's text, in bytes.
    pub start: usize,
    /// The run's text. A line that ends in a hard line break ends in its
    /// line feed, which counts as one character.
    pub text: String,
    /// The length in bytes of each character of `text`, where a character
    /// is the smallest stretch a caret steps over. They add up to the
    /// length of `text`, and there are at most [`TEXT_RUN_CAP`] of them.
    pub character_lengths: Vec<u8>,
    /// The index of each character that starts a word, in order. A word
    /// takes the spaces after it, so a word starts at a character that is
    /// no space after one that is.
    pub word_starts: Vec<u8>,
    /// Where each character starts, from the edge the run starts at, in
    /// logical pixels: from the left edge, or from the right edge for a run
    /// that reads right to left.
    pub character_positions: Vec<f32>,
    /// The width of each character, in logical pixels.
    pub character_widths: Vec<f32>,
    /// The rectangle the run covers, in logical pixels.
    pub bounds: Rect,
    /// Whether the run reads right to left.
    pub right_to_left: bool,
    /// Whether the run goes on with the line of the run before it, as the
    /// stretches of a line that changes direction or is too long for one
    /// run do, so a screen reader reads them as one line.
    pub continues_line: bool,
}

impl Semantics {
    /// Creates a description with the role and nothing else.
    pub fn new(role: Role) -> Semantics {
        Semantics {
            role,
            label: None,
            value: None,
            checked: None,
            range: None,
            position: None,
            set_size: None,
            level: None,
            text_runs: Vec::new(),
            text_selection: None,
            links: Vec::new(),
            description: None,
            selected: None,
            expanded: None,
            invalid: false,
        }
    }
}

/// One node of the projected tree, in the form a backend reads.
///
/// `id`, `parent` and `children` name projected nodes only, so the backend can
/// build a tree whose nodes all carry a role. `children` is the list the
/// projection reports: the children of an unprojected widget attach to the
/// nearest projected ancestor.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticsNode {
    /// The id of the widget this node projects.
    pub id: WidgetId,
    /// The id of the nearest projected ancestor, or `None` for the root.
    pub parent: Option<WidgetId>,
    /// The ids of the projected children, in paint order.
    pub children: Vec<WidgetId>,
    /// The kind of control the node represents.
    pub role: Role,
    /// The accessible name.
    pub label: Option<String>,
    /// The current value.
    pub value: Option<String>,
    /// Whether a checkbox or a switch is checked.
    pub checked: Option<Checked>,
    /// The number the node shows and the range it moves in.
    pub range: Option<ValueRange>,
    /// The node's place from zero among the items of the set around it.
    pub position: Option<usize>,
    /// How many items the set the node holds has.
    pub set_size: Option<usize>,
    /// The level of a heading, from 1 for the top level down.
    pub level: Option<usize>,
    /// The node's text, a run per stretch of a line, with each run's bounds
    /// absolute, in logical pixels.
    pub text_runs: Vec<TextRun>,
    /// The selection in the node's text, or the caret when its ends meet.
    pub text_selection: Option<TextSelection>,
    /// The byte range of each link in the node's text, in order.
    pub links: Vec<std::ops::Range<usize>>,
    /// The node's description, which a screen reader announces after its
    /// name and role.
    pub description: Option<String>,
    /// Whether the node is the picked choice of the set around it, or
    /// `None` for a node that is no choice.
    pub selected: Option<bool>,
    /// Whether what the node opens is open, or `None` for a node that opens
    /// nothing.
    pub expanded: Option<bool>,
    /// The node's absolute rectangle in logical pixels, as the last layout
    /// pass left it.
    pub bounds: Rect,
    /// Whether the node can take the keyboard focus.
    pub focusable: bool,
    /// Whether the node is disabled.
    pub disabled: bool,
    /// Whether the node holds a value the app rejects, which
    /// [`Element::invalid`](crate::element::Element::invalid) or its widget
    /// says.
    pub invalid: bool,
    /// Whether the node holds the keyboard focus.
    pub focused: bool,
    /// Whether a press on the node runs a handler, so the backend can offer
    /// the press action.
    pub pressable: bool,
}

/// The projected nodes that changed since the last frame, and the messages
/// announced since then.
///
/// The backend rebuilds the nodes the changed ids project; `reset` says the
/// projection must be rebuilt from scratch instead, which happens when too
/// many changes queued in one frame. The ids name widgets that are alive,
/// visible and projected at the moment this value is taken, and each appears
/// once; a widget that a frame removes or hides drops out on its own, and the
/// change to its parent's child list is what takes it off the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct A11yChanges {
    /// The widgets whose projected nodes must be rebuilt, children included.
    pub changed: Vec<WidgetId>,
    /// Whether the list of projected nodes with no projected ancestor may
    /// have changed, which the backend's own root holds as its children.
    pub roots_changed: bool,
    /// Whether the whole projection must be rebuilt from scratch.
    pub reset: bool,
    /// The messages to speak, oldest first, which arrive even with `reset`.
    /// The newest [`ANNOUNCEMENT_CAP`] are kept when more queue between two
    /// takes.
    pub announcements: Vec<Announcement>,
}

/// How many announcements wait for a screen reader at most. Past it, the
/// oldest drop, since a reader that falls this far behind could not speak
/// them in time anyway.
pub const ANNOUNCEMENT_CAP: usize = 32;

/// How urgently a screen reader speaks an announcement.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Politeness {
    /// Spoken once the reader finishes what it is saying, such as a reply
    /// that streamed in or a note that a save finished.
    Polite,
    /// Spoken at once, cutting off what the reader is saying, such as an
    /// error that needs attention now.
    Assertive,
}

/// A message for a screen reader to speak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    /// What the reader speaks.
    pub message: String,
    /// How urgently it speaks it.
    pub politeness: Politeness,
}

/// Has a screen reader speak messages that no widget shows, such as "Saved"
/// or a block of a reply that just finished streaming.
///
/// Attach it to one element with
/// [`Element::announcer`](crate::element::Element::announcer), then call
/// [`Announcer::announce`] from anywhere: a press handler, an effect or a
/// task. Each message is spoken once, and the messages of one politeness
/// in the order they were announced, even when several arrive in one frame.
/// Cloning the handle is cheap, and every clone queues into the same
/// announcer.
///
/// Make the announcer under an owner that lives as long as the element it
/// is attached to, such as the one the element is built under: once its
/// owner is cleaned up, an announcement no longer wakes the element, and
/// waits for something else to.
///
/// ```
/// use smudge_core::element::Element;
/// use smudge_core::semantics::{Announcer, Politeness};
/// use smudge_core::widget::panel;
///
/// let announcer = Announcer::new();
/// let saved = announcer.clone();
/// let view = panel()
///     .announcer(&announcer)
///     .on_press(move || saved.announce("Saved", Politeness::Polite));
/// ```
#[derive(Clone)]
pub struct Announcer {
    /// The queue every clone shares.
    shared: Rc<AnnouncerQueue>,
}

/// The messages an announcer holds until its element takes them.
struct AnnouncerQueue {
    /// The messages not yet taken, oldest first, at most
    /// [`ANNOUNCEMENT_CAP`] of them.
    messages: RefCell<Vec<Announcement>>,
    /// Counts the announcements, which wakes the element that takes them.
    posted: RwSignal<u64>,
}

impl Default for Announcer {
    fn default() -> Self {
        Announcer::new()
    }
}

impl Announcer {
    /// Creates an announcer with nothing queued.
    pub fn new() -> Announcer {
        Announcer {
            shared: Rc::new(AnnouncerQueue {
                messages: RefCell::new(Vec::new()),
                posted: RwSignal::new(0),
            }),
        }
    }

    /// Queues `message` for a screen reader to speak with `politeness`.
    ///
    /// An empty message is ignored. A message announced while the element
    /// the announcer is attached to is hidden or fading out is dropped, and
    /// one announced before it mounts is spoken once it does. Only the
    /// newest [`ANNOUNCEMENT_CAP`] messages wait for the element to take
    /// them.
    pub fn announce(&self, message: impl Into<String>, politeness: Politeness) {
        let message = message.into();
        if message.is_empty() {
            return;
        }
        push_capped(
            &mut self.shared.messages.borrow_mut(),
            Announcement {
                message,
                politeness,
            },
        );
        self.shared
            .posted
            .update(|count| *count = count.wrapping_add(1));
    }

    /// Takes every queued message, and subscribes the current effect to
    /// the next.
    pub(crate) fn take(&self) -> Vec<Announcement> {
        self.shared.posted.with(|_| ());
        std::mem::take(&mut *self.shared.messages.borrow_mut())
    }
}

/// Appends `announcement` to `queue`, dropping the oldest past
/// [`ANNOUNCEMENT_CAP`].
pub(crate) fn push_capped(queue: &mut Vec<Announcement>, announcement: Announcement) {
    if queue.len() >= ANNOUNCEMENT_CAP {
        queue.remove(0);
    }
    queue.push(announcement);
}
