//! Elements: the builder that describes one node, and the bindings that keep
//! its properties current.
//!
//! An element holds a widget, a style, the bindings that update parts of that
//! style, and its children. Building one touches no tree: mounting creates the
//! nodes and installs one render effect per binding inside the node's owner.
//!
//! A binding never writes to the tree itself. Its effect reads the value and
//! pushes an update onto the shared update queue, and the frame applies the
//! queued updates before it lays anything out. That is what keeps every write
//! to the tree in one place and in one order.
//!
//! Every builder takes either a plain value or a closure. A plain value is
//! written when the element is built and costs nothing afterwards. A closure
//! becomes a binding: it is read when the node mounts and again whenever a
//! signal it reads changes, and it updates that one property without rebuilding
//! the element or its children.
//!
//! ```
//! use smudge_core::geometry::Color;
//! use smudge_core::reactive::{self, prelude::*};
//! use smudge_core::style::px;
//! use smudge_core::widget::panel;
//!
//! let (on, _set_on) = reactive::signal_local(false);
//! let card = panel()
//!     .width(px(120.0))
//!     .height(px(40.0))
//!     .corner_radius(6.0)
//!     .background(move || {
//!         if on.get() {
//!             Color::rgb(220, 60, 60)
//!         } else {
//!             Color::rgb(60, 60, 220)
//!         }
//!     })
//!     .child(panel().width(px(20.0)).height(px(20.0)).background(Color::WHITE));
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use crate::geometry::{Color, Corners, Size};
use crate::paint::PaintOp;
use crate::reactive::{Owner, RenderEffect};
use crate::style::{
    Align, Direction, Edges, Justify, LayoutProps, Length, Overflow, PaintProps, Position, Style,
};
use crate::tree::{Dirty, WidgetId, WidgetTree};
use crate::widget::{PaintCx, Widget};

/// One change to the widget tree, applied by the frame's apply step.
pub(crate) type Update = Box<dyn FnOnce(&mut WidgetTree)>;

/// The updates that are waiting to be applied.
///
/// Every binding shares one queue, so a frame applies the effects of all of
/// them in the order the effects were queued.
#[derive(Clone, Default)]
pub(crate) struct UpdateQueue(Rc<RefCell<Vec<Update>>>);

impl UpdateQueue {
    /// Queues `update` to run in the next apply step.
    pub(crate) fn push(&self, update: Update) {
        self.0.borrow_mut().push(update);
    }
}

/// A property that is either fixed or computed from signals.
///
/// [`Value::Static`] is written once, when the element is built. [`Value::Dynamic`]
/// becomes a binding: the closure is read when the node mounts and again
/// whenever a signal it reads changes.
///
/// A value converts into this type, so a builder method can take either one.
pub enum Value<T> {
    /// A value that never changes.
    Static(T),
    /// A closure that reads the value from signals.
    Dynamic(Rc<dyn Fn() -> T>),
}

/// Implements the conversion from a plain value for every property type.
macro_rules! static_values {
    ($($property:ty),* $(,)?) => {
        $(
            impl From<$property> for Value<$property> {
                fn from(value: $property) -> Value<$property> {
                    Value::Static(value)
                }
            }
        )*
    };
}

static_values!(
    Color,
    Length,
    Size,
    Edges<Length>,
    f32,
    bool,
    Direction,
    Align,
    Justify,
);

impl<F: Fn() -> T + 'static, T: 'static> From<F> for Value<T> {
    fn from(get: F) -> Value<T> {
        Value::Dynamic(Rc::new(get))
    }
}

/// Installs a binding's effect inside a node's owner and returns its handle.
type Install = Box<dyn FnOnce(WidgetId, &UpdateQueue) -> RenderEffect<()>>;

/// One property of one node that a render effect keeps up to date.
struct Binding {
    /// Creates the effect inside the node's owner and returns its handle.
    ///
    /// The effect runs once immediately and queues an update every time it
    /// runs.
    install: Install,
}

impl Binding {
    /// A binding that writes one layout property.
    fn layout<T: 'static>(get: Rc<dyn Fn() -> T>, write: fn(&mut LayoutProps, T)) -> Binding {
        Binding {
            install: Box::new(move |id, queue| {
                let queue = queue.clone();
                RenderEffect::new(move |_| {
                    let value = get();
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| {
                        tree.apply_layout(id, move |layout| write(layout, value));
                    }));
                })
            }),
        }
    }

    /// A binding that writes one paint property.
    fn paint<T: 'static>(get: Rc<dyn Fn() -> T>, write: fn(&mut PaintProps, T)) -> Binding {
        Binding {
            install: Box::new(move |id, queue| {
                let queue = queue.clone();
                RenderEffect::new(move |_| {
                    let value = get();
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| {
                        tree.apply_paint(id, move |paint| write(paint, value));
                    }));
                })
            }),
        }
    }

    /// A binding that writes one property of the node's widget.
    ///
    /// The apply function reports how much of the node its write invalidated,
    /// and the node is marked with that. A write that changes what the widget
    /// measures returns [`Dirty::Layout`], which also invalidates the size the
    /// layout engine cached for the node.
    fn widget<W: Widget, T: 'static>(
        get: Rc<dyn Fn() -> T>,
        apply: fn(&mut W, T) -> Dirty,
    ) -> Binding {
        Binding {
            install: Box::new(move |id, queue| {
                let queue = queue.clone();
                RenderEffect::new(move |_| {
                    let value = get();
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| {
                        let dirty = match tree.widget_mut::<W>(id) {
                            Some(widget) => apply(widget, value),
                            None => return,
                        };
                        tree.mark(id, dirty);
                    }));
                })
            }),
        }
    }
}

/// A node under construction: its widget, its style, its pending bindings and
/// its children.
///
/// Build one with a widget constructor such as [`panel`](crate::widget::panel),
/// chain the builder methods, and hand it to a parent with [`Element::child`]
/// or to [`show`].
///
/// A builder takes either a plain value, which is written into the style
/// immediately, or a closure, which becomes a binding that is read at mount and
/// again whenever a signal it reads changes.
pub struct Element<W: Widget> {
    /// The widget the node will own.
    widget: W,
    /// The style the node starts with.
    style: Style,
    /// Bindings installed when the node mounts.
    bindings: Vec<Binding>,
    /// The children, in paint order.
    children: Vec<AnyElement>,
}

impl<W: Widget> Element<W> {
    /// Creates an element for `widget` with the default style.
    ///
    /// ```
    /// use smudge_core::element::Element;
    /// use smudge_core::widget::Panel;
    ///
    /// let element = Element::new(Panel);
    /// ```
    pub fn new(widget: W) -> Element<W> {
        Element {
            widget,
            style: Style::default(),
            bindings: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Appends `child` after the children the element already has.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any());
        self
    }

    /// Appends every child of `children`, in order.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children
            .extend(children.into_iter().map(IntoElement::into_any));
        self
    }

    /// Writes one layout property, or records a binding for it.
    fn layout_value<T: 'static>(
        mut self,
        value: impl Into<Value<T>>,
        write: fn(&mut LayoutProps, T),
    ) -> Self {
        match value.into() {
            Value::Static(value) => write(&mut self.style.layout, value),
            Value::Dynamic(get) => self.bindings.push(Binding::layout(get, write)),
        }
        self
    }

    /// Writes one paint property, or records a binding for it.
    fn paint_value<T: 'static>(
        mut self,
        value: impl Into<Value<T>>,
        write: fn(&mut PaintProps, T),
    ) -> Self {
        match value.into() {
            Value::Static(value) => write(&mut self.style.paint, value),
            Value::Dynamic(get) => self.bindings.push(Binding::paint(get, write)),
        }
        self
    }

    /// Sets the width. This takes part in layout.
    pub fn width(self, width: impl Into<Value<Length>>) -> Self {
        self.layout_value(width, |layout, width| layout.width = width)
    }

    /// Sets the height. This takes part in layout.
    pub fn height(self, height: impl Into<Value<Length>>) -> Self {
        self.layout_value(height, |layout, height| layout.height = height)
    }

    /// Sets the width and the height, in logical pixels. This takes part in
    /// layout.
    pub fn size(self, size: impl Into<Value<Size>>) -> Self {
        self.layout_value(size, |layout, size| {
            layout.width = Length::px(size.width);
            layout.height = Length::px(size.height);
        })
    }

    /// Sets the lower bound of the width. This takes part in layout.
    pub fn min_width(self, width: impl Into<Value<Length>>) -> Self {
        self.layout_value(width, |layout, width| layout.min_width = width)
    }

    /// Sets the lower bound of the height. This takes part in layout.
    pub fn min_height(self, height: impl Into<Value<Length>>) -> Self {
        self.layout_value(height, |layout, height| layout.min_height = height)
    }

    /// Sets the upper bound of the width. This takes part in layout.
    pub fn max_width(self, width: impl Into<Value<Length>>) -> Self {
        self.layout_value(width, |layout, width| layout.max_width = width)
    }

    /// Sets the upper bound of the height. This takes part in layout.
    pub fn max_height(self, height: impl Into<Value<Length>>) -> Self {
        self.layout_value(height, |layout, height| layout.max_height = height)
    }

    /// Sets the same padding on all four sides. This takes part in layout.
    pub fn padding(self, padding: impl Into<Value<Length>>) -> Self {
        self.layout_value(padding, |layout, padding| {
            layout.padding = Edges::all(padding);
        })
    }

    /// Sets the horizontal padding from `width` and the vertical padding from
    /// `height`. This takes part in layout.
    pub fn padding_xy(self, padding: impl Into<Value<Size>>) -> Self {
        self.layout_value(padding, |layout, padding| {
            layout.padding = Edges::xy(Length::px(padding.width), Length::px(padding.height));
        })
    }

    /// Sets the same margin on all four sides. This takes part in layout.
    pub fn margin(self, margin: impl Into<Value<Length>>) -> Self {
        self.layout_value(margin, |layout, margin| {
            layout.margin = Edges::all(margin);
        })
    }

    /// Sets the space between adjacent children. This takes part in layout.
    pub fn gap(self, gap: impl Into<Value<Size>>) -> Self {
        self.layout_value(gap, |layout, gap| layout.gap = gap)
    }

    /// Sets the main axis of the children's flex layout. This takes part in
    /// layout.
    pub fn direction(self, direction: impl Into<Value<Direction>>) -> Self {
        self.layout_value(direction, |layout, direction| {
            layout.direction = direction;
        })
    }

    /// Lays the children out left to right. This takes part in layout.
    pub fn row(mut self) -> Self {
        self.style.layout.direction = Direction::Row;
        self
    }

    /// Lays the children out top to bottom. This takes part in layout.
    pub fn column(mut self) -> Self {
        self.style.layout.direction = Direction::Column;
        self
    }

    /// Lets children that do not fit move to a new line instead of shrinking.
    /// This takes part in layout.
    pub fn wrap(self, wrap: impl Into<Value<bool>>) -> Self {
        self.layout_value(wrap, |layout, wrap| layout.wrap = wrap)
    }

    /// Sets the share of the leftover space on the main axis this node takes.
    /// This takes part in layout.
    pub fn grow(self, grow: impl Into<Value<f32>>) -> Self {
        self.layout_value(grow, |layout, grow| layout.grow = grow)
    }

    /// Sets how readily this node gives up space when its siblings overflow.
    /// This takes part in layout.
    pub fn shrink(self, shrink: impl Into<Value<f32>>) -> Self {
        self.layout_value(shrink, |layout, shrink| layout.shrink = shrink)
    }

    /// Sets the main axis size the node starts from before growing or
    /// shrinking. This takes part in layout.
    pub fn basis(self, basis: impl Into<Value<Length>>) -> Self {
        self.layout_value(basis, |layout, basis| layout.basis = basis)
    }

    /// Sets how the children line up across the main axis. This takes part in
    /// layout.
    pub fn align_items(self, align: impl Into<Value<Align>>) -> Self {
        self.layout_value(align, |layout, align| layout.align_items = Some(align))
    }

    /// Sets how the children are spread along the main axis. This takes part in
    /// layout.
    pub fn justify_content(self, justify: impl Into<Value<Justify>>) -> Self {
        self.layout_value(justify, |layout, justify| {
            layout.justify_content = Some(justify);
        })
    }

    /// Takes the node out of normal flow, so its insets place it against its
    /// containing block and its siblings are laid out as if it were not there.
    /// This takes part in layout.
    pub fn absolute(mut self) -> Self {
        self.style.layout.position = Position::Absolute;
        self
    }

    /// Sets the offsets from the containing block's edges. This takes part in
    /// layout.
    pub fn inset(self, inset: impl Into<Value<Edges<Length>>>) -> Self {
        self.layout_value(inset, |layout, inset| layout.inset = inset)
    }

    /// Sets the border width, the same on all four sides. This takes part in
    /// layout, because the border takes space away from the content. The border
    /// is painted in the color set by [`Element::border_color`].
    pub fn border(self, width: impl Into<Value<f32>>) -> Self {
        self.layout_value(width, |layout, width| {
            layout.border_width = Edges::all(width);
        })
    }

    /// Clips the children to the node's bounds. This takes part in layout,
    /// because clipping also changes the automatic minimum size of a flex item.
    pub fn clip(self, clip: impl Into<Value<bool>>) -> Self {
        self.layout_value(clip, |layout, clip| {
            layout.overflow = if clip {
                Overflow::Clip
            } else {
                Overflow::Visible
            };
        })
    }

    /// Hides the node and its subtree, or shows them again. This takes part in
    /// layout, because a hidden node is not laid out and paints nothing.
    pub fn hidden(self, hidden: impl Into<Value<bool>>) -> Self {
        self.layout_value(hidden, |layout, hidden| layout.hidden = hidden)
    }

    /// Sets the fill painted behind the node. This repaints the node without
    /// laying it out again.
    pub fn background(self, color: impl Into<Value<Color>>) -> Self {
        self.paint_value(color, |paint, color| paint.background = Some(color))
    }

    /// Sets the color of the border whose width [`Element::border`] sets. This
    /// repaints the node without laying it out again.
    pub fn border_color(self, color: impl Into<Value<Color>>) -> Self {
        self.paint_value(color, |paint, color| paint.border_color = Some(color))
    }

    /// Sets the corner radius of the background and the border, the same on all
    /// four corners. This repaints the node without laying it out again.
    pub fn corner_radius(self, radius: impl Into<Value<f32>>) -> Self {
        self.paint_value(radius, |paint, radius| {
            paint.corner_radius = Corners::all(radius);
        })
    }

    /// Binds one property of the element's own widget.
    ///
    /// `apply` writes the value into the widget and returns how much of the
    /// node the write invalidated. A write that changes what the widget
    /// measures returns [`Dirty::Layout`], and the node is laid out and painted
    /// again. Any other write returns [`Dirty::Paint`] and only repaints the
    /// node.
    ///
    /// ```
    /// use smudge_core::element::Element;
    /// use smudge_core::paint::PaintOp;
    /// use smudge_core::tree::Dirty;
    /// use smudge_core::widget::{PaintCx, Widget};
    ///
    /// struct Dot(f32);
    ///
    /// impl Widget for Dot {
    ///     fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
    /// }
    ///
    /// let element = Element::new(Dot(0.0)).bind(3.0, |dot, value| {
    ///     dot.0 = value;
    ///     Dirty::Paint
    /// });
    /// ```
    pub fn bind<T: 'static>(
        mut self,
        value: impl Into<Value<T>>,
        apply: fn(&mut W, T) -> Dirty,
    ) -> Self {
        match value.into() {
            Value::Static(value) => {
                apply(&mut self.widget, value);
            }
            Value::Dynamic(get) => self.bindings.push(Binding::widget(get, apply)),
        }
        self
    }
}

/// An element with its widget type erased, so that a parent can hold children
/// of different widget types.
///
/// Build one with [`IntoElement::into_any`]. Its contents are private: only
/// mounting reads them.
pub struct AnyElement {
    /// The node's widget.
    widget: Box<dyn Widget>,
    /// The style the node starts with.
    style: Style,
    /// Bindings installed when the node mounts.
    bindings: Vec<Binding>,
    /// The children, in paint order.
    children: Vec<AnyElement>,
}

impl AnyElement {
    /// Erases the widget type of `element`.
    fn new<W: Widget>(element: Element<W>) -> AnyElement {
        AnyElement {
            widget: Box::new(element.widget),
            style: element.style,
            bindings: element.bindings,
            children: element.children,
        }
    }
}

/// Converts an element into the erased form a parent stores.
pub trait IntoElement {
    /// Erases the widget type of this element.
    fn into_any(self) -> AnyElement;
}

impl<W: Widget> IntoElement for Element<W> {
    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}

/// Mounts `element` as a child of `parent`, or as the root when `parent` is
/// `None`, and returns the id of the node it created.
///
/// The node starts at [`Dirty::Layout`]. Bindings are installed inside the
/// node's owner and run once immediately, so the updates they queue are applied
/// before the first layout.
pub(crate) fn mount(
    element: AnyElement,
    tree: &mut WidgetTree,
    queue: &UpdateQueue,
    parent: Option<WidgetId>,
) -> WidgetId {
    let AnyElement {
        widget,
        style,
        bindings,
        children,
    } = element;
    let id = tree.insert(parent, widget, style);
    let owner = tree.owner(id).expect("the node was just inserted").clone();
    for binding in bindings {
        let effect = owner.with(|| (binding.install)(id, queue));
        tree.add_effect(id, effect);
    }
    for child in children {
        mount(child, tree, queue, Some(id));
    }
    id
}

/// A node that paints nothing and holds at most one child.
///
/// [`show`] returns an element for a slot and mounts its child under it.
pub struct Slot;

impl Widget for Slot {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// Mounts the element `build` returns while `when` is true, and removes it
/// while `when` is false.
///
/// Only `when` runs inside a binding, so only the signals it reads make the
/// slot check again. `build` runs untracked, once each time the child mounts,
/// under an owner of its own. Signals that `build` creates therefore live
/// exactly as long as the child: they survive every check that leaves the
/// child mounted, and they are disposed when the child is removed, after the
/// child's bindings have stopped.
///
/// ```
/// use smudge_core::element::show;
/// use smudge_core::reactive::{self, prelude::*};
/// use smudge_core::widget::panel;
///
/// let (visible, _set_visible) = reactive::signal_local(true);
/// let root = show(move || visible.get(), panel);
/// ```
pub fn show<E: IntoElement>(
    when: impl Fn() -> bool + 'static,
    build: impl Fn() -> E + 'static,
) -> Element<Slot> {
    let build: Rc<dyn Fn() -> AnyElement> = Rc::new(move || build().into_any());
    let mut element = Element::new(Slot);
    element.bindings.push(Binding {
        install: Box::new(move |id, queue| {
            let queue = queue.clone();
            let mounted: Rc<RefCell<Option<(WidgetId, Owner)>>> = Rc::new(RefCell::new(None));
            RenderEffect::new(move |_| {
                let visible = when();
                let child_queue = queue.clone();
                let mounted = Rc::clone(&mounted);
                let build = Rc::clone(&build);
                queue.push(Box::new(move |tree| {
                    let mut mounted = mounted.borrow_mut();
                    match (visible, mounted.take()) {
                        (true, None) => {
                            let Some(owner) = tree.owner(id).map(Owner::child) else {
                                return;
                            };
                            let element = owner.with(|| build());
                            let child = mount(element, tree, &child_queue, Some(id));
                            *mounted = Some((child, owner));
                        }
                        (false, Some((child, owner))) => {
                            tree.remove(child);
                            owner.cleanup();
                        }
                        (_, unchanged) => *mounted = unchanged,
                    }
                }));
            })
        }),
    });
    element
}

#[cfg(test)]
#[path = "element_tests.rs"]
mod tests;
