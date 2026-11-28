//! Pointer routing: hit testing, hover and press.
//!
//! [`App::handle_event`](crate::app::App::handle_event) hands the pointer
//! events it receives to the tree, which finds the node under the pointer and
//! keeps the interaction bits current. That state lives on the nodes instead
//! of in signals: a change marks the node for style re-resolution, which is
//! where the style variants read it. Application code that needs a hover
//! change in its own logic writes its own signal from
//! [`on_hover`](crate::element::Element::on_hover). Keyboard focus and key
//! routing live beside it in [`super::focus`].

use std::any::Any;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::geometry::Point;
use crate::geometry::Rect;
use crate::input::{ButtonEvent, Drag, Handled, Key, KeyEvent, PointerButton};
use crate::reactive::{RwSignal, prelude::*};

use super::{WidgetId, WidgetTree};

/// The interaction state of one node, kept as bits.
///
/// The routing sets and clears the bits as the pointer moves, buttons are
/// pressed and focus moves. A change marks the node for style re-resolution,
/// so a style that depends on the state of a node is resolved again before it
/// paints.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Interaction(u16);

impl Interaction {
    /// The pointer is over the node or over one of its descendants.
    pub(crate) const HOVERED: u16 = 1 << 0;

    /// A primary press started on the node and is still held.
    pub(crate) const PRESSED: u16 = 1 << 1;

    /// The node is left out of hit testing, so it receives no pointer events
    /// of its own.
    pub(crate) const DISABLED: u16 = 1 << 2;

    /// The node holds the keyboard focus.
    pub(crate) const FOCUSED: u16 = 1 << 3;

    /// The node holds the keyboard focus and the focus last moved by keyboard
    /// navigation, so a focus ring can draw for keyboard users alone.
    pub(crate) const KEYBOARD_FOCUS: u16 = 1 << 4;

    /// The node can take the keyboard focus, by a press or by Tab.
    pub(crate) const FOCUSABLE: u16 = 1 << 5;

    /// A press on the node leaves the keyboard focus where it is.
    pub(crate) const KEEPS_FOCUS: u16 = 1 << 6;

    /// The node holds a value the app rejects.
    pub(crate) const INVALID: u16 = 1 << 7;

    /// A drag whose payload the node accepts is held over it.
    pub(crate) const DROP: u16 = 1 << 8;

    /// Returns whether `bit` is set.
    pub(crate) fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    /// Sets or clears `bit`, and returns whether the state changed.
    pub(crate) fn set(&mut self, bit: u16, on: bool) -> bool {
        let changed = self.has(bit) != on;
        if changed {
            if on {
                self.0 |= bit;
            } else {
                self.0 &= !bit;
            }
        }
        changed
    }
}

/// A key handler: it decides whether the event stops bubbling to ancestors.
pub(crate) type KeyHandler = Box<dyn FnMut(&KeyEvent) -> Handled>;

/// A commit handler: it inserts the text an input method committed.
pub(crate) type CommitHandler = Box<dyn FnMut(&str)>;

/// A preedit handler: it shows or clears the text an input method composes.
pub(crate) type PreeditHandler = Box<dyn FnMut(&str, Option<(usize, usize)>)>;

/// A button press handler: it decides whether the node takes the press, and
/// with it the moves and the release of the button.
pub(crate) type ButtonDownHandler = Box<dyn FnMut(ButtonEvent) -> Handled>;

/// A file hover handler: it hears the paths of the files over the node, or
/// `None` when they left it.
pub(crate) type FileHoverHandler = Box<dyn FnMut(Option<&[PathBuf]>)>;

/// A file drop handler: it hears the paths of the files dropped on the node
/// and where they landed.
pub(crate) type FileDropHandler = Box<dyn FnMut(&[PathBuf], Point)>;

/// The input handlers one node owns.
///
/// The builder methods on [`Element`](crate::element::Element) set them, and
/// the tree keeps them for as long as the node lives. Every handler runs
/// inside its node's reactive owner, so it sees the same context the node's
/// bindings see.
#[derive(Default)]
pub(crate) struct Handlers {
    /// Runs when the primary button is released over the node after a press
    /// that started on it.
    pub(crate) on_press: Option<Box<dyn FnMut()>>,
    /// Runs when the node enters or leaves the hovered chain.
    pub(crate) on_hover: Option<Box<dyn FnMut(bool)>>,
    /// Runs when a pointer button is pressed on the node.
    pub(crate) on_pointer_down: Option<Box<dyn FnMut(Point)>>,
    /// Runs when a primary press targets a node that is neither this node nor
    /// one of its descendants.
    pub(crate) on_press_outside: Option<Box<dyn FnMut()>>,
    /// Runs when a key event reaches the node while it or a descendant holds
    /// the focus. Returning [`Handled::Yes`] stops the event from bubbling.
    pub(crate) on_key: Option<KeyHandler>,
    /// Runs when the wheel scrolls while the node is in the hovered chain.
    /// Returning [`Handled::Yes`] stops the event from bubbling.
    pub(crate) on_wheel: Option<Box<dyn FnMut(Point) -> Handled>>,
    /// Runs when a pointer move happens while a press on the node is held,
    /// or, with no press held, when the node is the hover target.
    pub(crate) on_pointer_move: Option<Box<dyn FnMut(Point)>>,
    /// Runs when the primary button is released after a press on the node,
    /// whether or not the release happened over the node, or, with no press
    /// held, when it happens over the node as the hover target.
    pub(crate) on_pointer_up: Option<Box<dyn FnMut(Point)>>,
    /// Runs when the input method commits text while the node or a
    /// descendant holds the focus. Routed like a key event, not a pointer
    /// event, so it takes no part in hit testing.
    pub(crate) on_ime_commit: Option<CommitHandler>,
    /// Runs when the input method changes its composing text while the node
    /// or a descendant holds the focus. An empty text clears the
    /// composition. Routed like a commit.
    pub(crate) on_ime_preedit: Option<PreeditHandler>,
    /// Runs when the node gains or loses the keyboard focus.
    pub(crate) on_focus: Option<Box<dyn FnMut(bool)>>,
    /// Runs when the keyboard focus enters the node or its subtree from
    /// outside, or leaves both.
    pub(crate) on_focus_within: Option<Box<dyn FnMut(bool)>>,
    /// The drag-over handlers a node sets, boxed together since few nodes set
    /// either of them.
    pub(crate) drag_over: Option<Box<DragOverHandlers>>,
    /// The file drop handlers of a node, boxed together since few nodes set
    /// either of them.
    pub(crate) file: Option<Box<FileHandlers>>,
    /// The in-app drag-and-drop handlers of a node, boxed together since
    /// few nodes set any of them.
    pub(crate) dnd: Option<Box<DndHandlers>>,
    /// Runs while a primary press that began inside the node's rectangle is
    /// held, and when it ends. Returning [`Handled::Yes`] for a move keeps it
    /// from the observers around the node. It takes no part in hit testing.
    pub(crate) on_drag: Option<Box<dyn FnMut(Drag) -> Handled>>,
    /// Answers whether the node responds to the pointer at a point relative
    /// to its rectangle. Where it answers `false`, hit testing passes the
    /// node by as if it had no handlers.
    pub(crate) responds_at: Option<Box<dyn Fn(Point) -> bool>>,
    /// Runs with the node's new scroll offset when the tree scrolled the node
    /// to bring the focused node into view. Its presence is what lets the
    /// tree scroll the node for that, and it takes no part in hit testing.
    pub(crate) on_reveal: Option<Box<dyn FnMut(Point)>>,
    /// Runs when any pointer button is pressed while the node is in the
    /// chain of the press's target. Returning [`Handled::Yes`] takes the
    /// press: the node then hears the button's moves and its release, and
    /// the ancestors do not hear the press.
    pub(crate) on_button_down: Option<ButtonDownHandler>,
    /// Runs on every pointer move while a button press the node took is
    /// held.
    pub(crate) on_button_move: Option<Box<dyn FnMut(ButtonEvent)>>,
    /// Runs when the button of a press the node took is released, wherever
    /// the pointer is.
    pub(crate) on_button_up: Option<Box<dyn FnMut(ButtonEvent)>>,
    /// Runs with the pointer's raw motion while the node holds the pointer
    /// lock.
    pub(crate) on_pointer_motion: Option<Box<dyn FnMut(Point)>>,
    /// Runs when the node's context menu is asked for: by a secondary press
    /// in its chain that no button handler took, or by the context menu key
    /// or Shift+F10 while it or a descendant holds the focus. It hears the
    /// point to open the menu at, relative to the node's rectangle.
    pub(crate) on_context_menu: Option<Box<dyn FnMut(Point)>>,
    /// Whether a fresh press of Enter or Space runs the node's
    /// [`on_press`](Self::on_press) handler, which is how a button fires from
    /// the keyboard.
    pub(crate) activate_on_keys: bool,
    /// How the arrow keys move the focus among the focusable nodes inside
    /// this one, when they do.
    pub(crate) arrows: Option<ArrowNav>,
    /// Runs when the arrows of a container around this node land the focus
    /// on this one, set by
    /// [`Element::on_arrow_focus`](crate::element::Element::on_arrow_focus).
    pub(crate) on_arrow_focus: Option<Box<dyn FnMut()>>,
    /// What the library builds on the node watches of its hover and its
    /// presses beside the app's own handlers, boxed since few nodes have
    /// any.
    pub(crate) watchers: Option<Box<Watchers>>,
}

/// The drag-over handlers a node sets, boxed together since few nodes set
/// either of them.
#[derive(Default)]
pub(crate) struct DragOverHandlers {
    /// Runs, while a primary press is held, when the node is the deepest one
    /// under the pointer that sets it. It takes no part in hit testing.
    pub(crate) over: Option<Box<dyn FnMut(Point)>>,
    /// Runs when a held press's drag-over leaves the node it last reached.
    /// It takes no part in hit testing.
    pub(crate) leave: Option<Box<dyn FnMut()>>,
}

/// The file handlers a drop zone sets. Neither takes part in hit testing.
#[derive(Default)]
pub(crate) struct FileHandlers {
    /// Runs when files from outside the window hover over the node: with
    /// their paths when they enter it and on every report while they stay
    /// over it, and with `None` when they leave it or the hover is
    /// cancelled.
    pub(crate) hover: Option<FileHoverHandler>,
    /// Runs when files from outside the window are dropped on the node, with
    /// their paths and the position relative to the node's rectangle.
    pub(crate) drop: Option<FileDropHandler>,
}

/// A payload builder: it builds what a drag of the node carries.
pub(crate) type SourceHandler = Box<dyn Fn() -> Rc<dyn Any>>;

/// A preview factory: it builds the element shown at the pointer while the
/// node's drag is in the air.
pub(crate) type PreviewFactory = Rc<dyn Fn() -> crate::element::AnyElement>;

/// A payload acceptor: it answers whether the node takes a drag that
/// carries this payload.
pub(crate) type AcceptsHandler = Box<dyn Fn(&Rc<dyn Any>) -> bool>;

/// A payload drop handler: it hears the payload dropped on the node and
/// where it landed.
pub(crate) type DropHandler = Box<dyn FnMut(Rc<dyn Any>, Point)>;

/// What the default drag preview draws: the surface the source resolved to
/// when the drag left it.
#[derive(Clone, Copy)]
pub(crate) struct PreviewSurface {
    /// The fill the source paints behind its content.
    pub(crate) background: crate::theme::ColorValue,
    /// The corner radii the source rounds its fill with.
    pub(crate) corners: crate::theme::CornerRadii,
    /// The size the source laid out at.
    pub(crate) size: crate::geometry::Size,
}

/// The in-app drag-and-drop handlers a node sets, boxed together since few
/// nodes set any of them.
#[derive(Default)]
pub(crate) struct DndHandlers {
    /// Builds the payload a drag of this node carries, once the drag leaves
    /// the node.
    pub(crate) source: Option<SourceHandler>,
    /// Answers whether this node takes a drag carrying the payload, while a
    /// drag is in the air.
    pub(crate) accepts: Option<AcceptsHandler>,
    /// Runs when a drag whose payload this node accepted is dropped on it,
    /// with the payload and the position relative to the top left of the
    /// node's content box.
    pub(crate) drop: Option<DropHandler>,
    /// Runs when a drag this node started ends, with whether a target took
    /// the payload.
    pub(crate) drag_end: Option<Box<dyn FnMut(bool)>>,
    /// Runs for a release inside the window with no accepting target.
    pub(crate) drag_miss: Option<Box<dyn FnMut(Point)>>,
    /// Builds the preview shown at the pointer while the drag is in the
    /// air, shared with the overlay the element mounted for the source, so
    /// a [`drag_preview`](crate::element::Element::drag_preview) set after
    /// a [`drag_source`](crate::element::Element::drag_source) reaches it.
    pub(crate) preview: Option<Rc<RefCell<Option<PreviewFactory>>>>,
    /// Whether the preview of this node's drag shows. Set once the drag
    /// leaves the node, read by the overlay the element mounted for it.
    pub(crate) preview_open: Option<RwSignal<bool>>,
    /// Where the preview sits, relative to the node's rectangle.
    pub(crate) preview_at: Option<RwSignal<Point>>,
    /// The surface the preview draws when the node set no preview of its
    /// own, written when the drag leaves the node.
    pub(crate) preview_surface: Option<RwSignal<PreviewSurface>>,
}

/// The keys a node set up with
/// [`navigate_with_arrows`](crate::element::Element::navigate_with_arrows)
/// or [`navigate_with_all_arrows`](crate::element::Element::navigate_with_all_arrows)
/// answers the arrows with.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum ArrowNav {
    /// The arrows along one layout axis move the focus: Down and Up through
    /// a column, Right and Left through a row.
    Axis(crate::style::Direction),
    /// Every arrow moves the focus in one order: Down and Right to the next
    /// node, Up and Left to the one before it, as a radio group does.
    Linear,
}

/// Input observers and fallback handlers that leave ordinary handlers in place.
#[derive(Default)]
pub(crate) struct Watchers {
    /// Runs for keys that no ordinary handler took in this focus chain.
    pub(crate) on_unhandled_key: Option<KeyHandler>,
    /// Run with the node's handler when it enters or leaves the hovered
    /// chain.
    pub(crate) hover: Vec<Box<dyn FnMut(bool)>>,
    /// Run when a press of any button begins on the node or a descendant.
    pub(crate) press: Vec<Box<dyn FnMut()>>,
}

/// The drag a held primary press makes, from the press until its release,
/// even when the node that took the press goes away.
pub(crate) struct DragState {
    /// The deepest node the press began over, whose nearest scrolling
    /// ancestor gives the area a drag-over falls back to.
    origin: Option<WidgetId>,
    /// The deepest surviving pointer ancestor of the original hit, used to
    /// keep drag observers within its ancestry when content moves.
    pub(super) observer_origin: Option<WidgetId>,
    /// The visible area of the original hit when the press began, the
    /// fallback for an origin that went away.
    origin_clip: Rect,
    /// The nodes that hear the drag, deepest first: those that set `on_drag`
    /// and whose rectangle held the point the press began at.
    observers: Vec<WidgetId>,
    /// The node and local position the last drag-over reached, so a delivery
    /// that would repeat it is skipped.
    last_over: Option<(WidgetId, Point)>,
    /// Whether the last drag-over delivery reached its node through the
    /// real pointer, so a fallback delivery that lands on the same node at
    /// the same position still runs the leave the pointer owes it.
    last_real: bool,
    /// Whether the pointer left every node the drag-over reaches, so the
    /// leave of the last one runs once until the pointer finds one again.
    left_listeners: bool,
}

/// How far the pointer must move from its press before the press becomes a
/// drag of the source it began on, in logical pixels.
const DRAG_SLOP: f32 = 4.0;

/// What a drag-over delivery runs.
enum DragDelivery {
    /// The delivery repeats the last one, so it runs nothing.
    Repeated,
    /// The delivery runs the over, and the leave of the node it displaced,
    /// when there is one.
    Fresh(Option<WidgetId>),
}

/// The in-app drag a held press on a drag source makes.
///
/// It is armed when the press captures the source, and left the source once
/// the pointer has moved [`DRAG_SLOP`] past the press, which is when the
/// payload is built and the preview shows. A release before that is a plain
/// press.
pub(crate) struct ActiveDrag {
    /// The node the drag started on, whose handlers built the payload, hear
    /// how the drag ended and own the preview.
    source: WidgetId,
    /// What the source carries, held for the drag's duration. `None` while
    /// the drag has not left the source yet.
    payload: Option<Rc<dyn Any>>,
    /// The point the press began at, in window coordinates, which decides
    /// when the drag leaves the source.
    began_at: Point,
    /// The node the drag hovers that accepts the payload, if one does.
    target: Option<WidgetId>,
}

/// Returns `point` moved into `rect`, a hair inside its far edges so the
/// result still lies in it.
fn clamp_into(point: Point, rect: Rect) -> Point {
    let inside = |value: f32, low: f32, high: f32| value.max(low).min((high - 0.5).max(low));
    Point {
        x: inside(point.x, rect.x0, rect.x1),
        y: inside(point.y, rect.y0, rect.y1),
    }
}

/// Returns whether `position` lies in the region where `node` responds to
/// the pointer, which is everywhere unless it set
/// [`responds_at`](Handlers::responds_at).
fn in_region(node: &super::Node, position: Point) -> bool {
    node.handlers.responds_at.as_ref().is_none_or(|region| {
        let origin = node.rect.origin();
        region(Point {
            x: position.x - origin.x,
            y: position.y - origin.y,
        })
    })
}

impl Handlers {
    /// Returns whether any handler is set that makes the node a pointer
    /// target.
    ///
    /// Hit testing makes the deepest node under the pointer that can respond
    /// the target, so a node without any lets a press pass to its ancestors. A
    /// node can also respond by being focusable, which
    /// [`Interaction::FOCUSABLE`] tracks. A commit handler is left out,
    /// because a commit never comes from the pointer.
    pub(crate) fn has_any(&self) -> bool {
        self.watchers
            .as_ref()
            .is_some_and(|watchers| !watchers.hover.is_empty())
            || self.on_press.is_some()
            || self.on_hover.is_some()
            || self.on_pointer_down.is_some()
            || self.on_press_outside.is_some()
            || self.on_key.is_some()
            || self.on_wheel.is_some()
            || self.on_pointer_move.is_some()
            || self.on_pointer_up.is_some()
            || self
                .dnd
                .as_ref()
                .is_some_and(|dnd| dnd.accepts.is_some() || dnd.source.is_some())
    }

    /// Returns whether any handler is set that makes the node take a press.
    ///
    /// The wheel is left out: it reaches the hovered chain, so a node that
    /// only scrolls, such as a scroll view, lets presses on it pass to the
    /// nearest ancestor that takes them. A drag source takes the press so
    /// the move that starts its drag is the press's own.
    pub(crate) fn takes_press(&self) -> bool {
        self.on_press.is_some()
            || self.on_hover.is_some()
            || self.on_pointer_down.is_some()
            || self.on_press_outside.is_some()
            || self.on_key.is_some()
            || self.on_pointer_move.is_some()
            || self.on_pointer_up.is_some()
            || self.dnd.as_ref().is_some_and(|dnd| dnd.source.is_some())
    }
}

impl WidgetTree {
    /// Returns whether a held press or a drag in the air pins the pointer:
    /// hover stays where it is while either lasts.
    pub(crate) fn holds_pointer(&self) -> bool {
        self.pressed.is_some() || self.dnd.as_ref().is_some_and(|drag| drag.payload.is_some())
    }

    /// Routes a pointer move to the node under `position`.
    ///
    /// While a press is held, the captured node keeps the pointer and hears
    /// its `on_pointer_move`, so a drag stays with the node the press started
    /// on; hover does not change and does not come back while the drag lasts,
    /// even when the pointer returns over the captured node. Without a press,
    /// the hovered chain becomes the new target and its ancestors, and the
    /// node under the pointer hears its `on_pointer_move`.
    pub(crate) fn pointer_move(&mut self, position: Point) {
        self.pointer = position;
        self.pointer_inside = true;
        self.run_button_moves(position);
        if self.drag.is_some() {
            self.deliver_drag_over();
            self.deliver_drag(position);
        }
        // The drag keeps flying even when the press capture is gone, because
        // the node that took the press can be removed while it is in the
        // air.
        if self.holds_pointer() {
            self.update_dnd(position);
        }
        if let Some(captured) = self.pressed {
            self.run_pointer_move(captured, position);
            return;
        }
        // A drag whose capture is gone still keeps hover where it is, as a
        // held press does: what lies under the pointer hears the drop, not
        // the hover.
        if self.dnd.as_ref().is_some_and(|drag| drag.payload.is_some()) {
            return;
        }
        let target = self.hover_target(position);
        self.set_hover_target(target);
        if let Some(target) = target {
            self.run_pointer_move(target, position);
        }
    }

    /// Routes a pointer down at `position`.
    ///
    /// A primary press makes the node under the pointer the target, runs its
    /// `on_pointer_down` with the position, sets its pressed bit and captures
    /// it. Every node that asks for presses outside the target hears one. A
    /// press also updates hover, because a press can arrive without a move
    /// before it, except while a press is already held: a non-primary down
    /// then leaves hover and the capture alone, and a primary press replaces
    /// the capture. The replaced node hears `on_pointer_up` first, because the
    /// release of its press never arrived, so a drag it started still ends.
    /// Any processed press turns the keyboard navigation mode
    /// off, and a primary press focuses the target when it can take the focus
    /// or clears the focus when it cannot. While a focus trap is mounted, a
    /// press outside the trap leaves the focus where it is instead, and so
    /// does a press on a node built with
    /// [`keeps_focus`](crate::element::Element::keeps_focus).
    ///
    /// Before any of that, every button offers the press to the button
    /// handlers from the deepest node under the pointer up, deepest first, and the first that
    /// takes it hears the button's moves and release. A secondary press that
    /// no button handler took asks the chain for its context menu.
    pub(crate) fn pointer_down(&mut self, position: Point, button: PointerButton) {
        self.pointer = position;
        self.pointer_inside = true;
        self.dismiss_outside(position);
        self.raise_floating(position);
        let target = self.target(position);
        self.run_press_watchers(self.hit_any(position, true));
        if self.pressed.is_none() || button == PointerButton::Primary {
            self.set_hover_target(self.hover_target(position));
            self.set_keyboard_nav(false);
        }
        // The button handlers and the context menu hear the press from the
        // deepest node under the pointer, so a node with only those takes
        // no primary press from a pressable node around it.
        let deepest = self.hit_any(position, false);
        let taken = self.run_button_down(deepest, button, position);
        if button == PointerButton::Secondary && !taken {
            self.run_context_menu(deepest, position);
        }
        if button != PointerButton::Primary {
            return;
        }
        self.released_early = false;
        // A press outside the active focus trap is not a focus gesture: the
        // focus stays wherever the trap holds it, on nothing when it holds
        // nothing.
        let outside_trap = self
            .active_focus_trap()
            .is_some_and(|trap| target.is_none_or(|target| !self.is_within(target, trap)));
        // The node that takes the press decides, and a press that lands on a
        // disabled node and goes nowhere is decided by that node.
        let keeps_focus = target.or_else(|| self.landing(position)).is_some_and(|id| {
            self.nodes
                .get(id)
                .is_some_and(|node| node.interaction.has(Interaction::KEEPS_FOCUS))
        });
        if !outside_trap && !keeps_focus {
            if target.is_some_and(|id| self.focusable(id)) {
                self.set_focus(target);
            } else {
                self.set_focus(None);
            }
            // A press is a focus gesture, so it also ends the reach of a
            // focus that a disabled or hidden node lost earlier, and drops a
            // reveal an earlier focus move asked for, even when the press
            // lands on the node that move focused.
            self.stale_focus = None;
            self.reveal_focus = false;
        }
        self.drag_left();
        self.end_drag();
        // A primary press ends a drag that is still in the air: the
        // button it waited for is gone. One that never left its source dies
        // with the press it armed, so the new press cannot re-arm it from a
        // threshold measured at the press that is being replaced.
        if let Some(drag) = self.dnd.take()
            && drag.payload.is_some()
        {
            if let Some(old) = drag.target {
                self.set_bit(old, Interaction::DROP, false);
            }
            self.close_preview(drag.source);
            self.run_drag_end(drag.source, false);
        }
        if let Some(previous) = self.pressed.take() {
            self.set_bit(previous, Interaction::PRESSED, false);
            self.run_pointer_up(previous, position);
        }
        self.run_press_outside(target);
        let Some(target) = target else {
            return;
        };
        self.start_drag(position);
        self.deliver_drag_over();
        self.run_pointer_down(target, position);
        self.set_bit(target, Interaction::PRESSED, true);
        self.pressed = Some(target);
        let holds_source = self.nodes.get(target).is_some_and(|node| {
            node.handlers
                .dnd
                .as_ref()
                .is_some_and(|dnd| dnd.source.is_some())
        });
        if holds_source {
            self.dnd = Some(ActiveDrag {
                source: target,
                payload: None,
                began_at: position,
                target: None,
            });
        }
    }

    /// Routes a pointer up at `position`.
    ///
    /// The node that took the button's press through its button handlers
    /// hears the release first, wherever the pointer is. Releasing the
    /// primary button clears the captured node's pressed bit,
    /// runs its `on_press` when the release happens over the captured node,
    /// which is the case when the node under the pointer is the captured node
    /// or one of its descendants, and runs its `on_pointer_up` whether or not
    /// the release happened over it, so a drag that ends elsewhere still
    /// hears the button come up. A release that ends an in-app drag is
    /// heard as a drop or as a cancellation instead, and runs no press. With
    /// no press held, the hover target hears
    /// `on_pointer_up`, unless the release belongs to a press that
    /// [`WidgetTree::release_press`] already ended: then no handler runs.
    /// Hover follows the pointer again either way.
    pub(crate) fn pointer_up(&mut self, position: Point, button: PointerButton) {
        self.run_button_up(button, position);
        if button != PointerButton::Primary {
            return;
        }
        self.pointer = position;
        self.drag_left();
        // The drag outlives the capture: the node that took the press may be
        // gone, and the observers still hear the release.
        self.end_drag();
        // A drag that left its source ends with the release: it is heard as
        // a drop or as a cancellation, and never as a press.
        if let Some(mut drag) = self.dnd.take()
            && let Some(payload) = drag.payload.take()
        {
            self.finish_dnd(drag, payload, position);
            return;
        }
        self.pointer_inside = true;
        let hovered = self.hover_target(position);
        let Some(captured) = self.pressed.take() else {
            self.set_hover_target(hovered);
            if std::mem::take(&mut self.released_early) {
                return;
            }
            if let Some(hovered) = hovered {
                self.run_pointer_up(hovered, position);
            }
            return;
        };
        self.set_bit(captured, Interaction::PRESSED, false);
        self.set_hover_target(hovered);
        // The release counts over the captured node by the hover target, which
        // passes over disabled content, so a press is not lost to a disabled
        // control inside the node it started on.
        if hovered.is_some_and(|hovered| self.chain(Some(hovered)).contains(&captured)) {
            self.run_press(captured);
        }
        self.run_pointer_up(captured, position);
    }

    /// Records that `id` locks the pointer while `signal` reads `true`.
    ///
    /// Called once, while the node is being mounted.
    pub(crate) fn add_pointer_lock(&mut self, id: WidgetId, signal: RwSignal<bool>) {
        self.pointer_locks.push((id, signal));
    }

    /// Returns the node that holds the pointer lock, with its signal: the
    /// last mounted live node whose signal reads `true` and that is neither
    /// disabled nor hidden.
    pub(crate) fn pointer_lock_holder(&self) -> Option<(WidgetId, RwSignal<bool>)> {
        self.pointer_locks
            .iter()
            .rev()
            .find(|(id, signal)| self.hears(*id) && signal.get())
            .cloned()
    }

    /// Hands `delta`, the pointer's raw motion in the device's units, to the
    /// node that holds the pointer lock. Without one it does nothing.
    pub(crate) fn pointer_motion(&mut self, delta: Point) {
        let Some((id, _)) = self.pointer_lock_holder() else {
            return;
        };
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_pointer_motion) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_motion.as_mut())
            {
                on_pointer_motion(delta);
            }
        });
    }

    /// Ends the pointer lock, as Escape and the window losing the focus do:
    /// the signal of every node that asks for it reads `false` from now on.
    /// Returns whether a node held it.
    pub(crate) fn release_pointer_lock(&mut self) -> bool {
        let Some(_) = self.pointer_lock_holder() else {
            return false;
        };
        // Every node that asks lets go, so the lock does not pass to the
        // next one.
        for (_, signal) in &self.pointer_locks {
            if signal.get() {
                signal.set(false);
            }
        }
        true
    }

    /// Delivers the release of every button held by a node of `removed`,
    /// at the pointer, while the nodes still live, and drops the pointer
    /// locks they asked for.
    pub(crate) fn end_holds_of(&mut self, removed: &[WidgetId]) {
        let held: Vec<PointerButton> = self
            .button_holders
            .iter()
            .filter(|(_, id)| removed.contains(id))
            .map(|(button, _)| *button)
            .collect();
        for button in held {
            self.run_button_up(button, self.pointer);
        }
        self.pointer_locks.retain(|(id, _)| !removed.contains(id));
    }

    /// Asks for the context menu of the focused node, as the context menu
    /// key and Shift+F10 do, when `event` is a fresh press of one of them:
    /// the nearest node in the focused node's chain that has a context menu
    /// hears the focused node's top left. Returns whether a node heard it.
    pub(crate) fn context_menu_key(&mut self, event: &KeyEvent) -> bool {
        let asks = event.pressed
            && !event.repeat
            && match event.key {
                Key::ContextMenu => true,
                Key::Function(10) => event.modifiers.shift,
                _ => false,
            };
        let Some(focused) = self
            .focused
            .filter(|&focused| asks && !self.behind_trap(focused))
        else {
            return false;
        };
        let corner = self
            .nodes
            .get(focused)
            .map(|node| node.rect.origin())
            .unwrap_or_default();
        self.run_context_menu(Some(focused), corner)
    }

    /// Ends the held press without a release: the captured node loses its
    /// pressed bit and hears `on_pointer_up`, so a drag it started ends, but
    /// not `on_press`, and the node that took the press through its button
    /// handlers hears the release too, so a hold it keeps ends, whether or
    /// not a node is captured. Hover then follows the pointer again. The
    /// press's own release, when it still arrives, reaches no handler,
    /// because the window system may refuse the gesture it was released
    /// for.
    pub(crate) fn release_press(&mut self) {
        self.drag_left();
        self.end_drag();
        // A drag in the air ends with the press that held it, and its
        // source hears that nothing took the payload.
        if let Some(drag) = self.dnd.take()
            && drag.payload.is_some()
        {
            if let Some(old) = drag.target {
                self.set_bit(old, Interaction::DROP, false);
            }
            self.close_preview(drag.source);
            self.run_drag_end(drag.source, false);
        }
        let buttons = self
            .button_holders
            .iter()
            .map(|(held, _)| *held)
            .collect::<Vec<_>>();
        for button in buttons {
            self.run_button_up(button, self.pointer);
        }
        let Some(captured) = self.pressed.take() else {
            let hovered = if self.pointer_inside {
                self.hover_target(self.pointer)
            } else {
                None
            };
            self.set_hover_target(hovered);
            return;
        };
        self.released_early = true;
        self.set_bit(captured, Interaction::PRESSED, false);
        self.run_pointer_up(captured, self.pointer);
        let hovered = if self.pointer_inside {
            self.hover_target(self.pointer)
        } else {
            None
        };
        self.set_hover_target(hovered);
    }

    /// Clears the hover of the whole tree, as when the pointer leaves the
    /// window.
    ///
    /// A press that is still held keeps its capture, so a drag that leaves
    /// the window stays with the node it started on, and the drag-over
    /// target hears its leave, so nothing stays lit while the drag waits
    /// outside.
    pub(crate) fn pointer_left(&mut self) {
        self.pointer_inside = false;
        self.set_hover_target(None);
        self.drag_left();
        if let Some(mut drag) = self.dnd.take() {
            if let Some(target) = drag.target.take() {
                self.set_bit(target, Interaction::DROP, false);
            }
            self.dnd = Some(drag);
        }
    }

    /// Routes a wheel scroll to the deepest node in the hovered chain that
    /// takes it.
    ///
    /// The hovered chain is the chain of the node under the pointer that can
    /// respond, deepest first, so content inside a scroll view is walked
    /// before the view itself. The first handler that returns
    /// [`Handled::Yes`] takes the whole event, so of two nested scroll views
    /// the one under the pointer scrolls and the one around it does not, and
    /// a handler that returns [`Handled::No`] lets the event bubble on. A
    /// disabled or hidden node's handler never runs, and the event keeps
    /// bubbling past it, as key routing does.
    ///
    /// While Shift is held, a delta with no horizontal part reaches the
    /// handlers turned sideways, so a plain mouse wheel scrolls a wide view.
    pub(crate) fn wheel(&mut self, delta: Point) {
        let Some(target) = self.hover_target(self.pointer) else {
            return;
        };
        let delta = if self.modifiers.shift && delta.x == 0.0 {
            Point { x: delta.y, y: 0.0 }
        } else {
            delta
        };
        for id in self.chain(Some(target)) {
            if self
                .nodes
                .get(id)
                .is_some_and(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
            {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            let mut handled = Handled::No;
            owner.with(|| {
                if let Some(on_wheel) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_wheel.as_mut())
                {
                    handled = on_wheel(delta);
                }
            });
            if handled == Handled::Yes {
                return;
            }
        }
    }

    /// Returns whether the tree holds work only a frame can finish: a node to
    /// lay out or paint, a focused node to scroll into view, or the pixels of
    /// a removed node to clear.
    pub(crate) fn has_pending_work(&self) -> bool {
        self.layout_dirty
            || self.reveal_focus
            || self.has_reveal_request()
            || !self.paint_queue.is_empty()
            || !self.pending_damage.is_empty()
    }

    /// Installs the handlers a node was mounted with.
    ///
    /// Called once, while the node is being mounted. A stale id does nothing.
    pub(crate) fn set_handlers(&mut self, id: WidgetId, handlers: Handlers) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.handlers = handlers;
        }
    }

    /// Sets the cursor the pointer shows over the node, or `None` to leave
    /// the choice to an ancestor.
    ///
    /// Called when a node mounts and again by the binding of a
    /// [`cursor`](crate::element::Element::cursor) builder value. A stale id
    /// does nothing.
    pub(crate) fn set_cursor(&mut self, id: WidgetId, cursor: Option<crate::window::Cursor>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.cursor = cursor;
        }
    }

    /// Returns the cursor of the deepest enabled node under the pointer that
    /// set one, or the platform default when none did.
    ///
    /// The walk starts at the deepest node the pointer hits, not at the hover
    /// target, so a node that sets only a cursor shows it without becoming a
    /// target. A node whose [`responds_at`](Handlers::responds_at) region
    /// leaves the pointer out shows no cursor of its own there. While a
    /// press is held, the walk starts there only when that node lies inside
    /// the captured node, and at the captured node otherwise, because hover
    /// stays with the capture during a drag. A pointer outside the window
    /// shows the default, unless a press is held.
    pub(crate) fn cursor(&self) -> crate::window::Cursor {
        let hit = if self.pointer_inside {
            self.hit_any(self.pointer, false)
        } else {
            None
        };
        let mut next = match self.pressed {
            Some(pressed) => Some(
                hit.filter(|&hit| self.is_within(hit, pressed))
                    .unwrap_or(pressed),
            ),
            None => hit,
        };
        while let Some(id) = next {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            if !node.interaction.has(Interaction::DISABLED)
                && let Some(cursor) = node.cursor
                && in_region(node, self.pointer)
            {
                return cursor;
            }
            next = self.pointer_parent(id);
        }
        crate::window::Cursor::default()
    }

    /// Sets whether the node is disabled, and marks the node when the state
    /// changed.
    ///
    /// Called when a node mounts and again by the binding of a
    /// [`disabled`](crate::element::Element::disabled) builder value. A node
    /// that is switched off loses the keyboard focus and, while no press is
    /// held, gives up its place in the hovered chain, because hover passes
    /// over a disabled node. A stale id does nothing.
    pub(crate) fn set_disabled(&mut self, id: WidgetId, disabled: bool) {
        self.set_bit(id, Interaction::DISABLED, disabled);
        self.push_a11y(id);
        if !disabled {
            return;
        }
        self.drop_invalid_focus();
        // While a press is held or a drag is in the air the pointer keeps
        // the hover of its capture, so the chain is only rebuilt when the
        // pointer is free.
        if !self.holds_pointer()
            && self
                .hovered
                .is_some_and(|hovered| self.chain(Some(hovered)).contains(&id))
        {
            let target = self.hover_target(self.pointer);
            self.set_hover_target(target);
        }
    }

    /// Sets whether the node holds a value the app rejects, which restyles it
    /// and tells a screen reader. A stale id does nothing.
    pub(crate) fn set_invalid(&mut self, id: WidgetId, invalid: bool) {
        let was = self
            .nodes
            .get(id)
            .is_some_and(|node| node.interaction.has(Interaction::INVALID));
        if was != invalid {
            self.set_bit(id, Interaction::INVALID, invalid);
            self.push_a11y(id);
        }
    }

    /// Sets whether the node can take the keyboard focus, and marks the node
    /// when the state changed.
    ///
    /// Called once, while the node is being mounted. A stale id does nothing.
    pub(crate) fn set_focusable(&mut self, id: WidgetId, focusable: bool) {
        self.set_bit(id, Interaction::FOCUSABLE, focusable);
    }

    /// Sets whether a press on the node leaves the keyboard focus where it
    /// is, as the [`keeps_focus`](crate::element::Element::keeps_focus)
    /// builder asks. A stale id does nothing.
    pub(crate) fn set_keeps_focus(&mut self, id: WidgetId, keeps: bool) {
        self.set_bit(id, Interaction::KEEPS_FOCUS, keeps);
    }

    /// Returns the deepest node under `position` that takes a press, or
    /// `None` when no node under the pointer takes one.
    ///
    /// The deepest hit node starts the search, and the first ancestor that
    /// takes a press becomes the target. A node that only scrolls with the
    /// wheel takes none, so a press on a scroll view reaches what is around
    /// it. When that ancestor is disabled, the press
    /// lands nowhere: a disabled control swallows presses on its content
    /// rather than passing them to the nodes around it.
    fn target(&self, position: Point) -> Option<WidgetId> {
        let mut id = self.hit_any(position, false)?;
        while !self.takes_press_at(id, position) {
            id = self.pointer_parent(id)?;
        }
        let disabled = self.nodes.get(id)?.interaction.has(Interaction::DISABLED);
        (!disabled).then_some(id)
    }

    /// Returns the node a press at `position` lands on, a disabled one
    /// included: the deepest node under it, disabled or not, or the first
    /// ancestor of that node that takes a press.
    fn landing(&self, position: Point) -> Option<WidgetId> {
        let mut id = self.hit_any(position, true)?;
        while !self.takes_press_at(id, position) {
            id = self.pointer_parent(id)?;
        }
        Some(id)
    }

    /// Returns the deepest node under `position` that hover lands on, or
    /// `None` when no enabled node under the pointer can respond.
    ///
    /// Unlike a press, hover passes over a disabled node to the nearest
    /// enabled one that can respond, so a container stays hovered while the
    /// pointer is over a disabled control inside it.
    pub(super) fn hover_target(&self, position: Point) -> Option<WidgetId> {
        let mut id = self.hit_any(position, false)?;
        loop {
            let node = self.nodes.get(id)?;
            if self.can_respond_at(id, position) && !node.interaction.has(Interaction::DISABLED) {
                return Some(id);
            }
            id = self.pointer_parent(id)?;
        }
    }

    /// Returns whether a press or hover can land on the node: it has a
    /// handler, it can take the focus, or it carries a style variant, which
    /// makes a press or a hover visible without any handler.
    fn can_respond(&self, id: WidgetId) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        node.handlers.has_any()
            || node.interaction.has(Interaction::FOCUSABLE)
            || node
                .variants
                .as_ref()
                .is_some_and(|variants| !variants.is_empty())
    }

    /// Returns whether a press or hover at `position` can land on the node:
    /// it can respond, and its [`responds_at`](Handlers::responds_at) region,
    /// when it sets one, holds the point.
    fn can_respond_at(&self, id: WidgetId, position: Point) -> bool {
        if !self.can_respond(id) {
            return false;
        }
        self.nodes
            .get(id)
            .is_some_and(|node| in_region(node, position))
    }

    /// Returns whether a press at `position` can land on the node: its
    /// handlers, its focus or its style variants take one, and its
    /// [`responds_at`](Handlers::responds_at) region, when it sets one, holds
    /// the point.
    fn takes_press_at(&self, id: WidgetId, position: Point) -> bool {
        self.nodes.get(id).is_some_and(|node| {
            (node.handlers.takes_press()
                || node.interaction.has(Interaction::FOCUSABLE)
                || node
                    .variants
                    .as_ref()
                    .is_some_and(|variants| !variants.is_empty()))
                && in_region(node, position)
        })
    }

    /// Returns whether the node can take the keyboard focus.
    pub(super) fn focusable(&self, id: WidgetId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| node.interaction.has(Interaction::FOCUSABLE))
    }

    /// Returns the deepest node that `position` hits, walking children in
    /// reverse paint order so the topmost node wins.
    ///
    /// A node is hit when the point is inside its rectangle and its clip,
    /// and it is neither hidden, fading out, nor disabled, unless `disabled`
    /// lets a disabled node be hit too. The children of a node are visited
    /// even when the point is outside the node's own rectangle, because a
    /// child of a node that does not clip its children reaches past them.
    /// Otherwise a disabled node is not hit, but its children still are.
    pub(super) fn hit(&self, id: WidgetId, position: Point, disabled: bool) -> Option<WidgetId> {
        let node = self.nodes.get(id)?;
        if node.withdrawn() || !node.clip.contains(position) {
            return None;
        }
        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit(*child, position, disabled) {
                return Some(hit);
            }
        }
        if node.rect.contains(position)
            && (disabled || !node.interaction.has(Interaction::DISABLED))
        {
            return Some(id);
        }
        None
    }

    /// Returns `target` and its ancestors as the pointer sees them,
    /// deepest first: the chain ends at the top node of an overlay, whose
    /// host lies elsewhere on the screen.
    ///
    /// A stale id, which a removed node can leave behind, ends the chain
    /// there.
    pub(super) fn chain(&self, target: Option<WidgetId>) -> Vec<WidgetId> {
        let mut chain = Vec::new();
        let mut next = target;
        while let Some(id) = next {
            chain.push(id);
            next = self.pointer_parent(id);
        }
        chain
    }

    /// Returns `target` and every ancestor, deepest first, through the
    /// hosts of overlays, so a node inside an overlay lies inside the node
    /// that opened it.
    pub(super) fn lineage(&self, target: Option<WidgetId>) -> Vec<WidgetId> {
        let mut chain = Vec::new();
        let mut next = target;
        while let Some(id) = next {
            chain.push(id);
            next = self.nodes.get(id).and_then(|node| node.parent);
        }
        chain
    }

    /// Makes `target` and its enabled ancestors the hovered chain.
    ///
    /// A disabled ancestor stays out of the chain, so its `on_hover` never
    /// runs, while the enabled nodes around it still count as hovered.
    ///
    /// The nodes that leave the chain clear their bit and hear `on_hover`
    /// with `false` before the nodes that join it are set and hear `on_hover`
    /// with `true`.
    pub(super) fn set_hover_target(&mut self, target: Option<WidgetId>) {
        if target == self.hovered && self.retained.hover_before.is_none() {
            return;
        }
        let old = self
            .retained
            .hover_before
            .take()
            .unwrap_or_else(|| self.chain(self.hovered));
        let mut new = self.chain(target);
        new.retain(|&id| {
            self.nodes
                .get(id)
                .is_some_and(|node| !node.interaction.has(Interaction::DISABLED))
        });
        for id in old.iter().filter(|id| !new.contains(id)) {
            self.set_bit(*id, Interaction::HOVERED, false);
        }
        for id in new.iter().rev().filter(|id| !old.contains(id)) {
            self.set_bit(*id, Interaction::HOVERED, true);
        }
        self.hovered = target;
    }

    /// Sets or clears one interaction bit of `id`, and reports the change to
    /// the node.
    ///
    /// A stale id does nothing.
    pub(super) fn set_bit(&mut self, id: WidgetId, bit: u16, on: bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if !node.interaction.set(bit, on) {
            return;
        }
        self.interaction_changed(id);
        if bit == Interaction::HOVERED {
            self.run_hover(id, on);
        } else if bit == Interaction::FOCUSED {
            self.run_focus(id, on);
        }
    }

    /// Marks that the interaction state of `id` changed.
    ///
    /// The style variants resolve from the interaction bits, hover and focus
    /// among them, so a change has to reach the style the node paints with.
    /// Resolution does nothing for a node whose resolved style does not
    /// change, which keeps a hovered or focused node without patches from
    /// repainting.
    fn interaction_changed(&mut self, id: WidgetId) {
        self.re_resolve(id);
    }

    /// Runs `on_hover` of `id` inside the node's owner.
    fn run_hover(&mut self, id: WidgetId, hovered: bool) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            let Some(handlers) = self.nodes.get_mut(id).map(|node| &mut node.handlers) else {
                return;
            };
            if let Some(on_hover) = handlers.on_hover.as_mut() {
                on_hover(hovered);
            }
            if let Some(watchers) = handlers.watchers.as_deref_mut() {
                for watcher in &mut watchers.hover {
                    watcher(hovered);
                }
            }
        });
    }

    /// Runs the press watchers of `target` and its ancestors as the pointer
    /// sees them, each inside its node's owner.
    fn run_press_watchers(&mut self, target: Option<WidgetId>) {
        for id in self.chain(target) {
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            owner.with(|| {
                if let Some(watchers) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.watchers.as_deref_mut())
                {
                    for watcher in &mut watchers.press {
                        watcher();
                    }
                }
            });
        }
    }

    /// Runs `on_focus` of `id` inside the node's owner.
    fn run_focus(&mut self, id: WidgetId, focused: bool) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_focus) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_focus.as_mut())
            {
                on_focus(focused);
            }
        });
    }

    /// Runs `on_press` of `id` inside the node's owner, and returns whether
    /// the handler ran.
    ///
    /// A node that became disabled or hidden while its press was held does not
    /// run it, because a disabled node's handlers never run. Key activation
    /// shares this path, so a button fires the same handler from either input.
    pub(crate) fn run_press(&mut self, id: WidgetId) -> bool {
        if self
            .nodes
            .get(id)
            .is_none_or(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
        {
            return false;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return false;
        };
        let mut ran = false;
        owner.with(|| {
            if let Some(on_press) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_press.as_mut())
            {
                on_press();
                ran = true;
            }
        });
        ran
    }

    /// Runs the action of the link at `index` of the node `id`, as a screen
    /// reader's press on it does, and returns whether one ran. A disabled or
    /// withdrawn node runs nothing.
    pub(crate) fn run_link(&mut self, id: WidgetId, index: usize) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if node.interaction.has(Interaction::DISABLED) || node.withdrawn() {
            return false;
        }
        let Some(action) = node.widget.link_action(index) else {
            return false;
        };
        let Some(owner) = self.owner(id).cloned() else {
            return false;
        };
        owner.with(|| action());
        true
    }

    /// Returns `position`, in window coordinates, relative to the top left of
    /// the rectangle of `id`, which is where the node's pointer handlers
    /// measure from. A stale id leaves it as it is.
    fn local_position(&self, id: WidgetId, position: Point) -> Point {
        let origin = self
            .nodes
            .get(id)
            .map(|node| node.rect.origin())
            .unwrap_or_default();
        Point {
            x: position.x - origin.x,
            y: position.y - origin.y,
        }
    }

    /// Returns `position` relative to the top left of the content box of
    /// `id`, inside its border and padding, which is where its drag-over
    /// handler measures from. A stale id leaves it as it is.
    fn content_position(&self, id: WidgetId, position: Point) -> Point {
        let origin = self
            .nodes
            .get(id)
            .map(|node| node.content.origin())
            .unwrap_or_default();
        Point {
            x: position.x - origin.x,
            y: position.y - origin.y,
        }
    }

    /// Starts the drag of a primary press that began at `position`: records
    /// the visible area of the node under it and the nodes that observe a
    /// drag that begins inside them.
    fn start_drag(&mut self, position: Point) {
        let origin = self.hit_any(position, false);
        let origin_clip = origin
            .and_then(|id| self.nodes.get(id))
            .map(|node| node.clip)
            .unwrap_or_default();
        let observers = self
            .chain(origin)
            .into_iter()
            .filter(|&id| {
                self.nodes.get(id).is_some_and(|node| {
                    node.handlers.on_drag.is_some() && node.rect.contains(position)
                })
            })
            .collect();
        self.drag = Some(DragState {
            origin,
            observer_origin: origin,
            origin_clip,
            observers,
            last_over: None,
            last_real: false,
            left_listeners: false,
        });
    }

    /// Tells every observer that still exists that the drag ended, and
    /// forgets the drag.
    fn end_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        for id in drag.observers {
            self.run_drag(id, Drag::Ended);
        }
    }

    pub(super) fn retain_drag_observer_origin(&mut self, removed: &[WidgetId]) {
        let Some(origin) = self.drag.as_ref().and_then(|drag| drag.observer_origin) else {
            return;
        };
        if removed.contains(&origin) {
            let surviving = self
                .chain(Some(origin))
                .into_iter()
                .find(|id| !removed.contains(id));
            if let Some(drag) = &mut self.drag {
                drag.observer_origin = surviving;
            }
        }
    }

    pub(super) fn finish_drag_move(&mut self) {
        let Some(mut drag) = self.drag.take() else {
            return;
        };
        let chain = self.chain(drag.observer_origin);
        drag.observers.retain(|&id| {
            if chain.contains(&id) {
                true
            } else {
                self.run_drag(id, Drag::Ended);
                false
            }
        });
        self.drag = Some(drag);
    }

    /// Runs `on_drag` of the observers of the held drag with the move to
    /// `position`, deepest first. Once one returns [`Handled::Yes`], the ones
    /// around it hear [`Drag::Taken`] instead. An observer that became
    /// disabled or hidden skips the move.
    fn deliver_drag(&mut self, position: Point) {
        let Some(observers) = self.drag.as_ref().map(|drag| drag.observers.clone()) else {
            return;
        };
        let mut taken = false;
        for id in observers {
            if self
                .nodes
                .get(id)
                .is_none_or(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
            {
                continue;
            }
            let step = if taken {
                Drag::Taken
            } else {
                Drag::Moved(self.local_position(id, position))
            };
            if self.run_drag(id, step) == Handled::Yes {
                taken = true;
            }
        }
    }

    /// Returns the area a drag-over falls back to, as it is now, so a drag
    /// past a scroll view's edge reaches the row at that edge while the view
    /// scrolls: the visible area of the nearest scrolling ancestor of the
    /// node the press began over whose content reaches past it on an axis
    /// the pointer left it by. A view that has no room that way, such as a
    /// strip that scrolls only sideways when the pointer left it downward,
    /// gives way to one around it, until one holds the pointer, where the
    /// nearest scrolling ancestor's area is used, as it is when none
    /// qualifies. Without one, it
    /// is the visible area of that node, and once that node is gone, the area
    /// it had when the press began.
    fn drag_fallback_area(&self, drag: &DragState) -> Rect {
        let pointer = self.pointer;
        let mut nearest = None;
        let mut next = drag.origin;
        while let Some(id) = next {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            if node.resolved.layout.overflow == crate::style::Overflow::Scroll {
                let area = self.children_clip(id).unwrap_or(node.clip);
                let max = self.max_scroll_offset(id).unwrap_or_default();
                let beyond_x = pointer.x < area.x0 || pointer.x > area.x1;
                let beyond_y = pointer.y < area.y0 || pointer.y > area.y1;
                // Once a view holds the pointer, the nearest view clamps it,
                // so a drag beside a list inside a page stays with the list.
                if !beyond_x && !beyond_y {
                    return nearest.unwrap_or(area);
                }
                if (beyond_x && max.x > 0.0) || (beyond_y && max.y > 0.0) {
                    return area;
                }
                nearest.get_or_insert(area);
            }
            next = self.pointer_parent(id);
        }
        if let Some(area) = nearest {
            return area;
        }
        drag.origin
            .and_then(|id| self.nodes.get(id))
            .map_or(drag.origin_clip, |node| node.clip)
    }

    /// Runs `on_drag_over` of the deepest node under the pointer that sets it,
    /// while a primary press is held.
    ///
    /// When nothing under the pointer listens, as when the pointer left the
    /// scroll view the drag began in, the pointer is moved into the visible
    /// area the press began over, so the drag reaches the content at that
    /// area's edge. This runs on every move of the press, and again when
    /// content moves under a still pointer inside the window; one that left
    /// the window hears nothing until it returns. The node a delivery moves the
    /// drag off, and one the pointer leaves for none that listens, hears
    /// its `on_drag_leave` first.
    pub(super) fn deliver_drag_over(&mut self) {
        let Some(area) = self.drag.as_ref().map(|drag| self.drag_fallback_area(drag)) else {
            return;
        };
        let pointer = self.pointer;
        let real = self.drag_over_target(pointer);
        let found = real.map(|id| (id, pointer)).or_else(|| {
            let clamped = clamp_into(pointer, area);
            (clamped != pointer)
                .then(|| self.drag_over_target(clamped).map(|id| (id, clamped)))
                .flatten()
        });
        let Some((id, position)) = found else {
            self.drag_left();
            return;
        };
        let local = self.content_position(id, position);
        match self.track_drag_over(id, local, real.is_some()) {
            DragDelivery::Repeated => return,
            DragDelivery::Fresh(leave) => {
                if let Some(prev) = leave {
                    self.notify_drag_leave(prev);
                }
            }
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_drag_over) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.drag_over.as_mut())
                .and_then(|drag_over| drag_over.over.as_mut())
            {
                on_drag_over(local);
            }
        });
    }

    /// Records that a drag-over reached `id` at `local`, and returns what the
    /// delivery runs: nothing when it repeats the last one, or the over with
    /// the node whose `on_drag_leave` it displaces, which is the one the drag
    /// moved off, or the last one when the pointer is over no node that
    /// listens. A fallback delivery that repeats a real one still runs, so
    /// the pointer leaving every listener is heard even when the fallback
    /// lands where the pointer was.
    fn track_drag_over(&mut self, id: WidgetId, local: Point, real: bool) -> DragDelivery {
        let Some(drag) = self.drag.as_mut() else {
            return DragDelivery::Fresh(None);
        };
        if real {
            drag.left_listeners = false;
        }
        if drag.last_over == Some((id, local)) && (real || !drag.last_real) {
            drag.last_real = real;
            return DragDelivery::Repeated;
        }
        let displaced = match drag.last_over {
            Some((prev, _)) if prev != id => Some(prev),
            _ => None,
        };
        let leave = if !real && !drag.left_listeners {
            drag.left_listeners = true;
            displaced.or(drag.last_over.map(|(prev, _)| prev))
        } else {
            displaced
        };
        drag.last_over = Some((id, local));
        drag.last_real = real;
        DragDelivery::Fresh(leave)
    }

    /// Runs the `on_drag_leave` of the node the drag-over last reached,
    /// whether the pointer left every node that listens or the drag ended,
    /// and forgets the delivery so a return to the same place delivers
    /// again.
    fn drag_left(&mut self) {
        let leave = if let Some(drag) = self.drag.as_mut() {
            drag.left_listeners = true;
            drag.last_over.take().map(|(prev, _)| prev)
        } else {
            return;
        };
        if let Some(prev) = leave {
            self.notify_drag_leave(prev);
        }
    }

    /// Runs `on_drag_leave` of the node `id` inside its owner.
    fn notify_drag_leave(&mut self, id: WidgetId) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_drag_leave) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.drag_over.as_mut())
                .and_then(|drag_over| drag_over.leave.as_mut())
            {
                on_drag_leave();
            }
        });
    }

    /// Returns the deepest enabled node under `position` that sets a
    /// drag-over handler.
    fn drag_over_target(&self, position: Point) -> Option<WidgetId> {
        let mut next = self.hit_any(position, false);
        while let Some(id) = next {
            let node = self.nodes.get(id)?;
            let listens = node
                .handlers
                .drag_over
                .as_ref()
                .is_some_and(|handlers| handlers.over.is_some() || handlers.leave.is_some());
            if listens && !node.interaction.has(Interaction::DISABLED) {
                return Some(id);
            }
            next = self.pointer_parent(id);
        }
        None
    }

    /// Moves the in-app drag of a held press to `position`.
    ///
    /// A drag that has not left its source yet leaves it once the pointer
    /// has moved [`DRAG_SLOP`] past the press, which builds the payload and
    /// shows the preview. One that has re-resolves the target under the
    /// pointer and moves the preview with the pointer. This runs on every
    /// move of the press, and, once the drag has left its source, again
    /// when content moves under a still pointer.
    fn update_dnd(&mut self, position: Point) {
        let Some(mut drag) = self.dnd.take() else {
            return;
        };
        let payload = match drag.payload.clone() {
            Some(payload) => payload,
            None => {
                let dx = position.x - drag.began_at.x;
                let dy = position.y - drag.began_at.y;
                if dx * dx + dy * dy < DRAG_SLOP * DRAG_SLOP {
                    self.dnd = Some(drag);
                    return;
                }
                let source = drag.source;
                // A source that went disabled or vanished while the press
                // held it builds no payload, and its drag dies with the
                // threshold it never passed.
                let disabled = self.nodes.get(source).is_none_or(|node| {
                    node.interaction.has(Interaction::DISABLED) || node.withdrawn()
                });
                if disabled {
                    return;
                }
                let Some(payload) = self.build_payload(source) else {
                    return;
                };
                self.open_preview(source);
                drag.payload = Some(payload.clone());
                payload
            }
        };
        let target = self.drop_target_at(position, &payload);
        if target != drag.target {
            if let Some(old) = drag.target {
                self.set_bit(old, Interaction::DROP, false);
            }
            if let Some(new) = target {
                self.set_bit(new, Interaction::DROP, true);
            }
            drag.target = target;
        }
        let (preview_at, open) = match self.preview_signals(drag.source) {
            Some((open, preview_at)) => (preview_at, open),
            None => {
                self.dnd = Some(drag);
                return;
            }
        };
        preview_at.set(self.local_position(drag.source, position));
        if !open.get() {
            open.set(true);
        }
        self.dnd = Some(drag);
    }

    /// Returns the deepest enabled node under `position` whose drop target
    /// accepts `payload`.
    fn drop_target_at(&self, position: Point, payload: &Rc<dyn Any>) -> Option<WidgetId> {
        let mut next = self.hit_any(position, false);
        while let Some(id) = next {
            let node = self.nodes.get(id)?;
            let accepts = node
                .handlers
                .dnd
                .as_ref()
                .and_then(|dnd| dnd.accepts.as_ref());
            if let Some(accepts) = accepts
                && !node.interaction.has(Interaction::DISABLED)
                && !node.withdrawn()
                && accepts(payload)
            {
                return Some(id);
            }
            next = self.pointer_parent(id);
        }
        None
    }

    /// Builds the payload of the drag source `source` inside its owner.
    ///
    /// A source whose node went away carries nothing, and its drag ends
    /// before it began.
    fn build_payload(&mut self, source: WidgetId) -> Option<Rc<dyn Any>> {
        let build = self
            .nodes
            .get(source)
            .and_then(|node| node.handlers.dnd.as_ref())
            .and_then(|dnd| dnd.source.as_ref())?;
        let owner = self.owner(source).cloned()?;
        Some(owner.with(build))
    }

    /// Returns the preview signals of the drag source `source`.
    fn preview_signals(&self, source: WidgetId) -> Option<(RwSignal<bool>, RwSignal<Point>)> {
        let dnd = self
            .nodes
            .get(source)
            .and_then(|node| node.handlers.dnd.as_ref())?;
        Some((dnd.preview_open.clone()?, dnd.preview_at.clone()?))
    }

    /// Samples the surface of the drag of `source`, which the preview the
    /// source's element mounted draws when it has no preview of its own.
    fn open_preview(&mut self, source: WidgetId) {
        let Some(surface) = self
            .nodes
            .get(source)
            .and_then(|node| node.handlers.dnd.as_ref())
            .and_then(|dnd| dnd.preview_surface.clone())
        else {
            return;
        };
        let Some(node) = self.nodes.get(source) else {
            return;
        };
        surface.set(PreviewSurface {
            background: node
                .resolved
                .paint
                .background
                .unwrap_or(crate::theme::ColorValue::Token(
                    crate::theme::ColorToken::SURFACE,
                )),
            corners: node.resolved.paint.corner_radius,
            size: crate::geometry::Size {
                width: node.rect.width(),
                height: node.rect.height(),
            },
        });
    }

    /// Hides the preview of the drag of `source`.
    fn close_preview(&mut self, source: WidgetId) {
        if let Some(dnd) = self
            .nodes
            .get(source)
            .and_then(|node| node.handlers.dnd.as_ref())
            && let Some(open) = dnd.preview_open.clone()
        {
            open.set(false);
        }
    }

    /// Ends an in-app drag that left its source, at the release over
    /// `position`.
    ///
    /// The deepest target under the pointer that accepts the payload hears
    /// the drop, and the source hears whether the payload was taken. The
    /// release ends the capture but runs no press: a drag is not a click.
    fn finish_dnd(&mut self, drag: ActiveDrag, payload: Rc<dyn Any>, position: Point) {
        self.pointer_inside &= self.viewport.is_some_and(|size| {
            position.x >= 0.0
                && position.y >= 0.0
                && position.x < size.width
                && position.y < size.height
        });
        let target = self
            .pointer_inside
            .then(|| self.drop_target_at(position, &payload))
            .flatten();
        if let Some(old) = drag.target {
            self.set_bit(old, Interaction::DROP, false);
        }
        self.close_preview(drag.source);
        let dropped = match target {
            Some(id) => {
                let local = self.content_position(id, position);
                self.run_drop(id, payload, local);
                true
            }
            None if self.pointer_inside => {
                self.run_drag_miss(drag.source, position);
                false
            }
            None => false,
        };
        self.run_drag_end(drag.source, dropped);
        let hovered = self
            .pointer_inside
            .then(|| self.hover_target(position))
            .flatten();
        if let Some(captured) = self.pressed.take() {
            self.set_bit(captured, Interaction::PRESSED, false);
            self.run_pointer_up(captured, position);
        }
        self.set_hover_target(hovered);
    }

    /// Cancels the in-app drag in the air, if it left its source, and
    /// returns whether there was one to spend Escape on.
    ///
    /// A drag that has not left its source yet stays where it is: a press
    /// below the drag threshold is not a drag for Escape to end. One that
    /// left takes the press capture with it, so the release that still
    /// arrives runs no press: a cancelled drag is not a click.
    pub(crate) fn cancel_dnd(&mut self) -> bool {
        let Some(drag) = self.dnd.take() else {
            return false;
        };
        if drag.payload.is_none() {
            self.dnd = Some(drag);
            return false;
        }
        if let Some(old) = drag.target {
            self.set_bit(old, Interaction::DROP, false);
        }
        self.close_preview(drag.source);
        self.drag_left();
        self.run_drag_end(drag.source, false);
        if let Some(captured) = self.pressed.take() {
            self.released_early = true;
            self.set_bit(captured, Interaction::PRESSED, false);
            self.run_pointer_up(captured, self.pointer);
        }
        let hovered = if self.pointer_inside {
            self.hover_target(self.pointer)
        } else {
            None
        };
        self.set_hover_target(hovered);
        true
    }

    /// Runs the drop handler of the target `id` with `payload` inside its
    /// owner.
    fn run_drop(&mut self, id: WidgetId, payload: Rc<dyn Any>, at: Point) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(drop) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.dnd.as_mut())
                .and_then(|dnd| dnd.drop.as_mut())
            {
                drop(payload, at);
            }
        });
    }

    fn run_drag_miss(&mut self, id: WidgetId, position: Point) {
        if !self.hears(id) {
            return;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(miss) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.dnd.as_mut())
                .and_then(|dnd| dnd.drag_miss.as_mut())
            {
                miss(position);
            }
        });
    }

    /// Runs the drag-end handler of the source `id` inside its owner.
    fn run_drag_end(&mut self, id: WidgetId, dropped: bool) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(drag_end) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.dnd.as_mut())
                .and_then(|dnd| dnd.drag_end.as_mut())
            {
                drag_end(dropped);
            }
        });
    }

    /// Re-resolves the target of an in-app drag under a still pointer, as
    /// [`Self::deliver_drag_over`] does for a drag-over.
    pub(super) fn redeliver_dnd(&mut self) {
        if self.dnd.as_ref().is_none_or(|drag| drag.payload.is_none()) {
            return;
        }
        self.update_dnd(self.pointer);
    }

    /// Routes a hover of `paths` over the window at `position` to the drop
    /// zones under it.
    ///
    /// The zone that hears it is the deepest enabled node under `position`
    /// that sets `on_file_hover`, and it hears every report, so a list of
    /// files that grows reaches it. A hover moving from one zone to another
    /// tells the first `None` before it tells the second the paths, and a
    /// hover that leaves every zone tells the zone it last hovered `None`.
    pub(crate) fn file_hover(&mut self, paths: &[PathBuf], position: Point) {
        match self.file_target(position, |h| {
            h.file.as_ref().is_some_and(|f| f.hover.is_some())
        }) {
            Some(id) => {
                if self.file_hover != Some(id) {
                    self.clear_file_hover();
                    self.file_hover = Some(id);
                }
                self.run_file_hover(id, Some(paths));
            }
            None => self.clear_file_hover(),
        }
    }

    /// Routes a drop of `paths` at `position` to the drop zone under it.
    ///
    /// The zone that receives it is the deepest enabled node under
    /// `position` that sets `on_file_drop`, so a zone inside a zone takes
    /// the drop and the one around it hears nothing. The hover of the zone
    /// that heard it ends, and the zone hears `None` before the drop.
    pub(crate) fn file_drop(&mut self, paths: &[PathBuf], position: Point) {
        let Some(id) = self.file_target(position, |h| {
            h.file.as_ref().is_some_and(|f| f.drop.is_some())
        }) else {
            return;
        };
        let local = self.local_position(id, position);
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        self.clear_file_hover();
        owner.with(|| {
            if let Some(on_file_drop) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.file.as_mut())
                .and_then(|file| file.drop.as_mut())
            {
                on_file_drop(paths, local);
            }
        });
    }

    /// Ends a file hover, as when the drag that carries the files left the
    /// window or the source cancelled it. The zone that heard the hover is
    /// told with `None`.
    pub(crate) fn file_hover_cancel(&mut self) {
        self.clear_file_hover();
    }

    /// Tells the zone that hears a file hover with `None` and forgets it.
    fn clear_file_hover(&mut self) {
        let Some(id) = self.file_hover.take() else {
            return;
        };
        self.run_file_hover(id, None);
    }

    /// Runs `on_file_hover` of `id` inside the node's owner with `paths`.
    fn run_file_hover(&mut self, id: WidgetId, paths: Option<&[PathBuf]>) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        owner.with(|| {
            if let Some(on_file_hover) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.file.as_mut())
                .and_then(|file| file.hover.as_mut())
            {
                on_file_hover(paths);
            }
        });
    }

    /// Returns the deepest enabled node under `position` that sets the
    /// handler `sets` names.
    fn file_target(&self, position: Point, sets: fn(&Handlers) -> bool) -> Option<WidgetId> {
        let mut next = self.hit_any(position, false);
        while let Some(id) = next {
            let node = self.nodes.get(id)?;
            if sets(&node.handlers) && !node.interaction.has(Interaction::DISABLED) {
                return Some(id);
            }
            next = self.pointer_parent(id);
        }
        None
    }

    /// Runs `on_drag` of `id` inside the node's owner and returns what it
    /// answered, or [`Handled::No`] when the node is gone.
    fn run_drag(&mut self, id: WidgetId, step: Drag) -> Handled {
        let Some(owner) = self.owner(id).cloned() else {
            return Handled::No;
        };
        let mut handled = Handled::No;
        owner.with(|| {
            if let Some(on_drag) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_drag.as_mut())
            {
                handled = on_drag(step);
            }
        });
        handled
    }

    /// Runs `on_pointer_down` of `id` inside the node's owner.
    /// Returns whether the node's handlers may run: it is live, enabled and
    /// not hidden.
    fn hears(&self, id: WidgetId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| !node.interaction.has(Interaction::DISABLED) && !node.withdrawn())
    }

    /// Builds the button event a node hears for `button` with the pointer
    /// at `position`.
    fn button_event(&self, id: WidgetId, button: PointerButton, position: Point) -> ButtonEvent {
        ButtonEvent {
            button,
            position: self.local_position(id, position),
            modifiers: self.modifiers,
        }
    }

    /// Offers a press of `button` at `position` to the button handlers of
    /// `target`'s chain, deepest first, and returns whether one took it. The
    /// node that took it holds the button until its release, in place of
    /// any node that held it before, and the node it replaced hears the
    /// release at once, because the release of its own press never arrives.
    fn run_button_down(
        &mut self,
        target: Option<WidgetId>,
        button: PointerButton,
        position: Point,
    ) -> bool {
        self.run_button_up(button, position);
        for id in self.chain(target) {
            if !self.hears(id) {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            let event = self.button_event(id, button, position);
            let mut handled = Handled::No;
            owner.with(|| {
                if let Some(on_button_down) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_button_down.as_mut())
                {
                    handled = on_button_down(event);
                }
            });
            if handled == Handled::Yes {
                self.button_holders.push((button, id));
                return true;
            }
        }
        false
    }

    /// Runs `on_button_move` of every node that holds a button, with the
    /// pointer at `position`.
    fn run_button_moves(&mut self, position: Point) {
        for (button, id) in self.button_holders.clone() {
            if !self.hears(id) {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            let event = self.button_event(id, button, position);
            owner.with(|| {
                if let Some(on_button_move) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_button_move.as_mut())
                {
                    on_button_move(event);
                }
            });
        }
    }

    /// Ends the hold on `button`: the node that took its press hears the
    /// release at `position`.
    fn run_button_up(&mut self, button: PointerButton, position: Point) {
        let Some(at) = self
            .button_holders
            .iter()
            .position(|(held, _)| *held == button)
        else {
            return;
        };
        // A holder that became disabled or hidden while the button was held
        // still hears its release, so it lets go of what it holds.
        let (_, id) = self.button_holders.remove(at);
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let event = self.button_event(id, button, position);
        owner.with(|| {
            if let Some(on_button_up) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_button_up.as_mut())
            {
                on_button_up(event);
            }
        });
    }

    /// Runs the context menu handler of the nearest node in `start`'s chain
    /// that has one, with `position` relative to that node, and returns
    /// whether a node heard it.
    fn run_context_menu(&mut self, start: Option<WidgetId>, position: Point) -> bool {
        let Some(id) = self.chain(start).into_iter().find(|&id| {
            self.hears(id)
                && self
                    .nodes
                    .get(id)
                    .is_some_and(|node| node.handlers.on_context_menu.is_some())
        }) else {
            return false;
        };
        let Some(owner) = self.owner(id).cloned() else {
            return false;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_context_menu) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_context_menu.as_mut())
            {
                on_context_menu(local);
            }
        });
        true
    }

    fn run_pointer_down(&mut self, id: WidgetId, position: Point) {
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_pointer_down) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_down.as_mut())
            {
                on_pointer_down(local);
            }
        });
    }

    /// Runs `on_pointer_move` of `id` inside the node's owner.
    ///
    /// A node that became disabled or hidden hears nothing, like every other
    /// pointer handler.
    fn run_pointer_move(&mut self, id: WidgetId, position: Point) {
        if self
            .nodes
            .get(id)
            .is_none_or(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
        {
            return;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_pointer_move) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_move.as_mut())
            {
                on_pointer_move(local);
            }
        });
    }

    /// Runs `on_pointer_up` of `id` inside the node's owner.
    ///
    /// A node that became disabled or hidden hears nothing, like every other
    /// pointer handler.
    fn run_pointer_up(&mut self, id: WidgetId, position: Point) {
        if self
            .nodes
            .get(id)
            .is_none_or(|node| node.interaction.has(Interaction::DISABLED) || node.withdrawn())
        {
            return;
        }
        let Some(owner) = self.owner(id).cloned() else {
            return;
        };
        let local = self.local_position(id, position);
        owner.with(|| {
            if let Some(on_pointer_up) = self
                .nodes
                .get_mut(id)
                .and_then(|node| node.handlers.on_pointer_up.as_mut())
            {
                on_pointer_up(local);
            }
        });
    }

    /// Runs `on_press_outside` of every node that is not in the target's
    /// chain, in paint order.
    ///
    /// A press with no target is outside every node, so a press on the empty
    /// window closes every popup that asks.
    fn run_press_outside(&mut self, target: Option<WidgetId>) {
        let chain = self.lineage(target);
        let mut outside = Vec::new();
        self.collect_press_outside(self.root, &mut outside);
        let overlays: Vec<WidgetId> = self
            .overlays
            .iter()
            .filter(|overlay| self.overlay_shown(overlay))
            .map(|overlay| overlay.root)
            .collect();
        for root in overlays {
            self.collect_press_outside(Some(root), &mut outside);
        }
        for id in outside {
            if chain.contains(&id) {
                continue;
            }
            let Some(owner) = self.owner(id).cloned() else {
                continue;
            };
            owner.with(|| {
                if let Some(on_press_outside) = self
                    .nodes
                    .get_mut(id)
                    .and_then(|node| node.handlers.on_press_outside.as_mut())
                {
                    on_press_outside();
                }
            });
        }
    }

    /// Appends every node of the subtree that asks for presses outside it, in
    /// paint order.
    ///
    /// A hidden node and its subtree are skipped, and a disabled node is
    /// skipped while its children are still visited, because the handlers of
    /// a hidden or disabled node never run.
    fn collect_press_outside(&self, id: Option<WidgetId>, outside: &mut Vec<WidgetId>) {
        let Some(id) = id else {
            return;
        };
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.withdrawn() {
            return;
        }
        if node.handlers.on_press_outside.is_some() && !node.interaction.has(Interaction::DISABLED)
        {
            outside.push(id);
        }
        for &child in &node.children {
            self.collect_press_outside(Some(child), outside);
        }
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
