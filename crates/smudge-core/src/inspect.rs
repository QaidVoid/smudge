//! What devtools read of an app's widget tree: every node with its depth,
//! the node under a point, and the details of one node.
//!
//! The module exists with the `devtools` feature only. The tree stays
//! private: these are copies taken when asked, so reading them never changes
//! the app.
//!
//! ```
//! use smudge_core::app::{App, AppConfig, RepaintMode};
//! use smudge_core::geometry::{Color, Point, Size};
//! use smudge_core::style::px;
//! use smudge_core::widget::panel;
//!
//! let mut app = App::new(
//!     AppConfig {
//!         viewport: Size {
//!             width: 100.0,
//!             height: 50.0,
//!         },
//!         scale: 1.0,
//!         clear_color: Color::WHITE,
//!         repaint: RepaintMode::Damage,
//!     },
//!     || panel().child(panel().width(px(10.0)).height(px(10.0))),
//! );
//! app.frame();
//!
//! let picked = app.node_at(Point { x: 5.0, y: 5.0 }).expect("a node is there");
//! let details = app.node_details(picked).expect("the node is live");
//! assert_eq!(details.boxes.border.width(), 10.0);
//! assert!(details.created_at.is_some());
//! ```

use std::panic::Location;

use crate::geometry::Rect;
use crate::semantics::SemanticsNode;
use crate::style::Style;
use crate::tree::{Dirty, WidgetId};

/// One node of the tree, as [`App::visit_tree`](crate::app::App::visit_tree)
/// reports it.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TreeEntry {
    /// The node.
    pub id: WidgetId,
    /// How deep the node sits, with the root at zero.
    pub depth: usize,
    /// The full name of the node's widget type.
    pub type_name: &'static str,
    /// The node's rectangle from the last layout pass, in logical pixels in
    /// window coordinates.
    pub rect: Rect,
    /// Whether the node, or one of its ancestors, is hidden, so it is laid
    /// out as empty and paints nothing.
    pub hidden: bool,
}

/// The interaction states a node is in, which decide the style variants
/// that apply to it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct States {
    /// Whether the pointer is over the node or one of its descendants.
    pub hovered: bool,
    /// Whether a press that began on the node is held.
    pub pressed: bool,
    /// Whether the node holds the keyboard focus.
    pub focused: bool,
    /// Whether the node is disabled.
    pub disabled: bool,
    /// Whether the node holds a value the app rejects.
    pub invalid: bool,
    /// Whether a drag whose payload the node accepts is held over it.
    pub drop: bool,
}

/// A node's four boxes from the last layout pass, in logical pixels in
/// window coordinates, from the outside in.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BoxModel {
    /// The node's rectangle grown by its margin.
    pub margin: Rect,
    /// The node's rectangle, the outer edge of its border.
    pub border: Rect,
    /// The node's rectangle inside its border, the outer edge of its
    /// padding.
    pub padding: Rect,
    /// The node's rectangle inside its border and padding, where its
    /// content goes.
    pub content: Rect,
}

/// What the inspector shows of one node, copied when asked.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NodeDetails {
    /// The node.
    pub id: WidgetId,
    /// The full name of the node's widget type.
    pub type_name: &'static str,
    /// Where the element of the node was created: the call of
    /// [`Element::new`](crate::element::Element::new), or of the first
    /// widget constructor on the way to it that does not pass its caller
    /// on. `None` for a node that was not built from an element.
    pub created_at: Option<&'static Location<'static>>,
    /// The style the element's builders and bindings wrote.
    pub base: Style,
    /// The style layout and paint read: the base style with the variants of
    /// the active states applied, and the window's size for the top node of
    /// an overlay that covers the window.
    pub resolved: Style,
    /// The interaction states the node is in.
    pub states: States,
    /// How much of the node is out of date.
    pub dirty: Dirty,
    /// The node's boxes from the last layout pass.
    pub boxes: BoxModel,
    /// The window rectangle the node's last paint covered, before clipping,
    /// or `None` when it has not painted.
    pub painted: Option<Rect>,
    /// How many paint operations the node's last paint produced.
    pub ops: usize,
    /// The node as the accessibility projection shows it, or `None` when it
    /// is not projected.
    pub semantics: Option<SemanticsNode>,
    /// Whether the node, or one of its ancestors, is hidden.
    pub hidden: bool,
    /// The bindings and effects the node's owner holds.
    pub bindings: Vec<BindingInfo>,
}

/// One binding or effect, as the inspector shows it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BindingInfo {
    /// Where the effect was created, when its constructor passed its caller
    /// on. Element bindings, which the library creates, have none.
    pub defined_at: Option<&'static Location<'static>>,
    /// How many times its body ran.
    pub runs: u64,
    /// The signals and memos its last run read.
    pub sources: Vec<SourceInfo>,
    /// The write that last woke it, or `None` when no write has since it
    /// was created.
    pub woken_by: Option<WriteInfo>,
}

/// A signal or memo an effect read.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceInfo {
    /// Whether it is a signal or a memo.
    pub kind: SourceKind,
    /// Where it was created.
    pub defined_at: Option<&'static Location<'static>>,
}

/// What an effect read.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceKind {
    /// A signal.
    Signal,
    /// A memo.
    Memo,
}

/// A write that woke an effect.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WriteInfo {
    /// Where the written signal was created.
    pub source_defined_at: Option<&'static Location<'static>>,
    /// Where the write happened: the call of `set` or `update`.
    pub written_at: &'static Location<'static>,
}

/// How often one effect ran, for the busiest effects list.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EffectRuns {
    /// Where the effect was created, when known.
    pub defined_at: Option<&'static Location<'static>>,
    /// How many times its body ran.
    pub runs: u64,
}

/// A cycle that a frame cut short: a binding that kept waking itself, or
/// updates that kept waking bindings.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CycleInfo {
    /// Where the effect that ran too often was created, when known.
    pub effect_defined_at: Option<&'static Location<'static>>,
    /// Where the write that last woke it happened, when known.
    pub written_at: Option<&'static Location<'static>>,
}

/// One owner and what it holds, as the owner tree shows it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OwnerEntry {
    /// How deep the owner sits, with the app's root owner at zero.
    pub depth: usize,
    /// How many signals it holds.
    pub signals: usize,
    /// How many memos it holds.
    pub memos: usize,
    /// How many effects it holds.
    pub effects: usize,
}

/// Where the keyboard focus is and where Tab takes it, as
/// [`App::focus_info`](crate::app::App::focus_info) reports it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FocusInfo {
    /// The node that holds the focus.
    pub focused: Option<WidgetId>,
    /// Whether keyboard navigation mode is on, which shows focus rings.
    pub keyboard: bool,
    /// The nodes Tab visits, in order, each with its rectangle.
    pub order: Vec<(WidgetId, Rect)>,
    /// The rectangle of the focus trap that holds the keyboard, if one does.
    pub trap: Option<Rect>,
}

/// What the pointer is over and what it holds, as
/// [`App::pointer_info`](crate::app::App::pointer_info) reports it.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PointerInfo {
    /// The hovered node and its enabled ancestors, deepest first.
    pub hovered: Vec<Rect>,
    /// The node a held press began on, which takes the pointer until the
    /// press ends.
    pub pressed: Option<Rect>,
    /// Every visible, enabled node that runs a handler when pressed.
    pub pressable: Vec<Rect>,
}

/// A node that assistive technology cannot describe well.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct A11yWarning {
    /// The node.
    pub id: WidgetId,
    /// Its rectangle.
    pub rect: Rect,
    /// What is missing.
    pub kind: A11yWarningKind,
}

/// What an [`A11yWarning`] is about.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum A11yWarningKind {
    /// The node takes the keyboard focus but has no name, so a screen
    /// reader announces nothing useful when it is focused.
    FocusableWithoutName,
    /// The node runs a handler when pressed but reports no role, so assistive
    /// technology does not know it can be pressed.
    PressableWithoutRole,
    /// A text input with no name, so a screen reader cannot say what it is
    /// for.
    TextInputWithoutName,
}
