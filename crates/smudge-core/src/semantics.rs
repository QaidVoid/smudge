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
//! An accessibility backend reads the projected tree through the app and the
//! changes since the last frame; this module defines the vocabulary the two
//! sides exchange. It stays backend-neutral: mapping it onto a specific
//! accessibility protocol is a backend's job.

use crate::geometry::Rect;
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
    /// A bar that shows how far a task has come. Its progress is
    /// [`Semantics::range`]. Without a range it reads as busy, with no known
    /// progress.
    ProgressBar,
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
    /// The node's absolute rectangle in logical pixels, as the last layout
    /// pass left it.
    pub bounds: Rect,
    /// Whether the node can take the keyboard focus.
    pub focusable: bool,
    /// Whether the node is disabled.
    pub disabled: bool,
    /// Whether the node holds the keyboard focus.
    pub focused: bool,
    /// Whether a press on the node runs a handler, so the backend can offer
    /// the press action.
    pub pressable: bool,
}

/// The projected nodes that changed since the last frame.
///
/// The backend rebuilds the nodes the changed ids project; `reset` says the
/// projection must be rebuilt from scratch instead, which happens when too
/// many changes queued in one frame. The ids name widgets that are alive,
/// visible and projected at the moment this value is taken, and each appears
/// once; a widget that a frame removes or hides drops out on its own, and the
/// change to its parent's child list is what takes it off the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct A11yChanges {
    /// The widgets whose projected nodes must be rebuilt, children included.
    pub changed: Vec<WidgetId>,
    /// Whether the list of projected nodes with no projected ancestor may
    /// have changed, which the backend's own root holds as its children.
    pub roots_changed: bool,
    /// Whether the whole projection must be rebuilt from scratch.
    pub reset: bool,
}
