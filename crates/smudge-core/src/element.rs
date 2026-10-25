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
//!             Color::rgb(220, 60, 60).into()
//!         } else {
//!             Color::rgb(60, 60, 220).into()
//!         }
//!     })
//!     .child(panel().width(px(20.0)).height(px(20.0)).background(Color::WHITE));
//! ```

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::rc::Rc;

use crate::geometry::{Color, Point, Size};
use crate::input::{Handled, KeyEvent};
use crate::paint::PaintOp;
use crate::reactive::{Owner, RenderEffect};
use crate::style::{
    Align, Direction, Edges, Justify, LayoutProps, Length, Overflow, PaintProps, Position, Style,
};
use crate::theme::{ColorToken, ColorValue, RadiusToken, RadiusValue};
use crate::tree::interaction::Handlers;
use crate::tree::{Dirty, WidgetId, WidgetTree};
use crate::variants::{PatchBuilder, StylePatch, Variants};
use crate::widget::{PaintCx, Widget};

/// One change to the widget tree, applied by the frame's apply step.
pub(crate) type Update = Box<dyn FnOnce(&mut WidgetTree)>;

/// The style that an element applies to every child as it is added.
type ChildStyling = Rc<dyn Fn(&mut Style)>;

/// The updates that are waiting to be applied.
///
/// Every binding shares one queue, so a frame applies the effects of all of
/// them in the order the effects were queued.
#[derive(Clone, Default)]
pub(crate) struct UpdateQueue(Rc<RefCell<VecDeque<Update>>>);

impl UpdateQueue {
    /// Queues `update` to run in the next apply step.
    pub(crate) fn push(&self, update: Update) {
        self.0.borrow_mut().push_back(update);
    }

    /// Applies every queued update to `tree`, in the order they were queued,
    /// and returns whether any ran.
    ///
    /// An update may queue another update, as mounting a child does when its
    /// bindings run. Those run after the updates that were already waiting, in
    /// the same call, so one call settles everything the queue holds.
    pub(crate) fn apply(&self, tree: &mut WidgetTree) -> bool {
        let mut applied = false;
        loop {
            let next = self.0.borrow_mut().pop_front();
            let Some(update) = next else {
                return applied;
            };
            applied = true;
            update(tree);
        }
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
    ColorValue,
    RadiusValue,
    Length,
    Size,
    Edges<Length>,
    f32,
    bool,
    Direction,
    Align,
    Justify,
    Point,
);

impl From<String> for Value<String> {
    fn from(text: String) -> Value<String> {
        Value::Static(text)
    }
}

impl From<&str> for Value<String> {
    fn from(text: &str) -> Value<String> {
        Value::Static(text.to_owned())
    }
}

impl From<Color> for Value<ColorValue> {
    fn from(color: Color) -> Value<ColorValue> {
        Value::Static(color.into())
    }
}

impl From<ColorToken> for Value<ColorValue> {
    fn from(token: ColorToken) -> Value<ColorValue> {
        Value::Static(token.into())
    }
}

impl From<f32> for Value<RadiusValue> {
    fn from(radius: f32) -> Value<RadiusValue> {
        Value::Static(radius.into())
    }
}

impl From<RadiusToken> for Value<RadiusValue> {
    fn from(token: RadiusToken) -> Value<RadiusValue> {
        Value::Static(token.into())
    }
}

impl<F: Fn() -> T + 'static, T: 'static> From<F> for Value<T> {
    fn from(get: F) -> Value<T> {
        Value::Dynamic(Rc::new(get))
    }
}

/// Installs a binding's effect inside a node's owner and returns its handle.
type Install = Box<dyn FnOnce(WidgetId, &UpdateQueue) -> RenderEffect>;

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
                RenderEffect::new(move || {
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
                RenderEffect::new(move || {
                    let value = get();
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| {
                        tree.apply_paint(id, move |paint| write(paint, value));
                    }));
                })
            }),
        }
    }

    /// A binding that writes the node's disabled bit.
    fn disabled(get: Rc<dyn Fn() -> bool>) -> Binding {
        Binding {
            install: Box::new(move |id, queue| {
                let queue = queue.clone();
                RenderEffect::new(move || {
                    let value = get();
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| {
                        tree.set_disabled(id, value);
                    }));
                })
            }),
        }
    }

    /// A binding that writes the node's scroll offset.
    ///
    /// The write moves the node's flowing descendants without running the
    /// layout engine, which is what a scroll view does on every wheel tick.
    fn scroll_offset(get: Rc<dyn Fn() -> Point>) -> Binding {
        Binding {
            install: Box::new(move |id, queue| {
                let queue = queue.clone();
                RenderEffect::new(move || {
                    let value = get();
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| {
                        tree.set_scroll_offset(id, value);
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
                RenderEffect::new(move || {
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
    /// The style patches per interaction state, when a variant builder ran.
    variants: Option<Variants>,
    /// The input handlers the node will own.
    handlers: Handlers,
    /// Whether the node starts disabled.
    disabled: bool,
    /// Whether the node can take the keyboard focus.
    focusable: bool,
    /// The scroll offset written to the node when it mounts.
    scroll_offset: Point,
    /// Applied to the style of every child as it is added.
    child_styling: Option<ChildStyling>,
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
            variants: None,
            handlers: Handlers::default(),
            disabled: false,
            focusable: false,
            scroll_offset: Point::default(),
            child_styling: None,
            children: Vec::new(),
        }
    }

    /// Appends `child` after the children the element already has.
    ///
    /// When [`Element::style_children`] set a style for the children, it is
    /// applied to this one.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        let mut child = child.into_any();
        if let Some(style) = &self.child_styling {
            style(&mut child.style);
        }
        self.children.push(child);
        self
    }

    /// Appends every child of `children`, in order.
    ///
    /// When [`Element::style_children`] set a style for the children, it is
    /// applied to each one.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children.extend(children.into_iter().map(|child| {
            let mut child = child.into_any();
            if let Some(style) = &self.child_styling {
                style(&mut child.style);
            }
            child
        }));
        self
    }

    /// Applies `style` to every direct child of this element: to the children
    /// added so far, at once, and to every child added afterwards as it is
    /// added.
    ///
    /// The children of a child keep their own style, and a child that mounts
    /// later through [`show`] mounts under a slot, so it is styled only
    /// through that slot. A binding on the same field of a child applies when
    /// the child mounts, after this style, so the binding wins.
    ///
    /// This is how a container decides how its children sit: a stack applies
    /// the absolute position that makes children overlap, for example.
    ///
    /// ```
    /// use smudge_core::element::Element;
    /// use smudge_core::widget::Panel;
    ///
    /// let layers = Element::new(Panel).style_children(|style| {
    ///     style.paint.corner_radius = 4.0.into();
    /// });
    /// ```
    pub fn style_children(mut self, style: impl Fn(&mut Style) + 'static) -> Self {
        let style = Rc::new(style);
        for child in &mut self.children {
            style(&mut child.style);
        }
        self.child_styling = Some(style);
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

    /// Translates the node's flowing children by this offset after layout,
    /// outside the layout engine.
    ///
    /// This is what a scroll view changes when it scrolls: the walk that
    /// computes absolute rectangles subtracts the offset from the origin of
    /// every child that stays in the flow, without running the layout engine
    /// again, and the moved nodes repaint. An absolutely positioned child is
    /// left where its insets place it, so chrome drawn over the content does
    /// not scroll away with it.
    ///
    /// The offset can be bound to a signal, like the style builders. It is a
    /// position, not a style: it takes part in no layout comparison, and an
    /// equal offset changes nothing.
    pub fn scroll_offset(mut self, offset: impl Into<Value<Point>>) -> Self {
        match offset.into() {
            Value::Static(offset) => self.scroll_offset = offset,
            Value::Dynamic(get) => self.bindings.push(Binding::scroll_offset(get)),
        }
        self
    }

    /// Sets the fill painted behind the node. This repaints the node without
    /// laying it out again.
    ///
    /// The color can be a [`Color`] or a
    /// [`ColorToken`]. A token resolves against the
    /// theme that is current when the node paints.
    pub fn background(self, color: impl Into<Value<ColorValue>>) -> Self {
        self.paint_value(color, |paint, color| paint.background = Some(color))
    }

    /// Sets the color of the border whose width [`Element::border`] sets. This
    /// repaints the node without laying it out again.
    ///
    /// The color can be a [`Color`] or a
    /// [`ColorToken`]. A token resolves against the
    /// theme that is current when the node paints.
    pub fn border_color(self, color: impl Into<Value<ColorValue>>) -> Self {
        self.paint_value(color, |paint, color| paint.border_color = Some(color))
    }

    /// Sets the color a widget's text paints with. This repaints the node
    /// without laying it out again.
    ///
    /// Widgets that show text read it; widgets without text ignore it. The
    /// color can be a [`Color`] or a
    /// [`ColorToken`]. A token resolves against the
    /// theme that is current when the node paints.
    pub fn text_color(self, color: impl Into<Value<ColorValue>>) -> Self {
        self.paint_value(color, |paint, color| paint.text_color = Some(color))
    }

    /// Sets the corner radius of the background and the border, the same on all
    /// four corners. This repaints the node without laying it out again.
    ///
    /// The radius can be an [`f32`] or a
    /// [`RadiusToken`]. A token resolves against
    /// the theme that is current when the node paints.
    pub fn corner_radius(self, radius: impl Into<Value<RadiusValue>>) -> Self {
        self.paint_value(radius, |paint, radius| {
            paint.corner_radius = radius;
        })
    }

    /// Records a patch built by `build` into one slot of the element's
    /// variants.
    fn variant(
        mut self,
        select: impl FnOnce(&mut Variants) -> &mut Option<StylePatch>,
        build: impl FnOnce(PatchBuilder) -> PatchBuilder,
    ) -> Self {
        let builder = build(PatchBuilder::default());
        let variants = self.variants.get_or_insert_with(Variants::default);
        *select(variants) = Some(builder.patch());
        self
    }

    /// Applies a patch while the node is in the hovered chain, which is the
    /// node under the pointer that can respond and its ancestors.
    ///
    /// The closure records static values into a [`PatchBuilder`]; a reactive
    /// value has no place in a patch, so the patch methods take plain values.
    /// While the pointer is over the node the patch overrides the fields it
    /// sets, and when the pointer leaves, the base style stands again. A node
    /// with a variant is a target for hover and presses even without
    /// handlers, so a styled card reacts to the pointer on its own.
    ///
    /// A change of hover re-resolves the style: a patch that touches only
    /// paint properties repaints the node, one that touches layout
    /// properties lays it out again.
    ///
    /// ```
    /// use smudge_core::geometry::Color;
    /// use smudge_core::widget::panel;
    ///
    /// let card = panel()
    ///     .background(Color::rgb(50, 50, 50))
    ///     .hover(|p| p.background(Color::rgb(70, 70, 70)));
    /// ```
    pub fn hover(self, patch: impl FnOnce(PatchBuilder) -> PatchBuilder) -> Self {
        self.variant(|variants| &mut variants.hovered, patch)
    }

    /// Applies a patch while the node holds the keyboard focus and the focus
    /// last moved by keyboard navigation.
    ///
    /// The focus a press moved draws no focus style, so a ring follows Tab and
    /// not a mouse click. Everything else works as
    /// [`hover`](Element::hover) does.
    ///
    /// ```
    /// use smudge_core::geometry::Color;
    /// use smudge_core::style::px;
    /// use smudge_core::widget::panel;
    ///
    /// let field = panel()
    ///     .width(px(120.0))
    ///     .height(px(24.0))
    ///     .focusable()
    ///     .focused(|p| p.border(1.0).border_color(Color::rgb(0, 120, 215)));
    /// ```
    pub fn focused(self, patch: impl FnOnce(PatchBuilder) -> PatchBuilder) -> Self {
        self.variant(|variants| &mut variants.focused, patch)
    }

    /// Applies a patch while a primary press is held on the node.
    ///
    /// Everything works as [`hover`](Element::hover) does; the pressed state
    /// starts when the button goes down on the node and ends when it is
    /// released.
    ///
    /// ```
    /// use smudge_core::geometry::Color;
    /// use smudge_core::widget::panel;
    ///
    /// let button = panel()
    ///     .background(Color::rgb(50, 50, 50))
    ///     .pressed(|p| p.background(Color::rgb(30, 30, 30)));
    /// ```
    pub fn pressed(self, patch: impl FnOnce(PatchBuilder) -> PatchBuilder) -> Self {
        self.variant(|variants| &mut variants.pressed, patch)
    }

    /// Applies a patch while the node is disabled.
    ///
    /// The patch wins over every other state, so a disabled control keeps its
    /// switched-off look while it is hovered or pressed. Everything else
    /// works as [`hover`](Element::hover) does.
    ///
    /// ```
    /// use smudge_core::geometry::Color;
    /// use smudge_core::widget::panel;
    ///
    /// let button = panel()
    ///     .background(Color::rgb(50, 50, 50))
    ///     .disabled_style(|p| p.background(Color::rgb(35, 35, 35)));
    /// ```
    pub fn disabled_style(self, patch: impl FnOnce(PatchBuilder) -> PatchBuilder) -> Self {
        self.variant(|variants| &mut variants.disabled, patch)
    }

    /// Runs `handler` when the primary button is released over the node after
    /// a press that started on it.
    ///
    /// The press is captured: between the press and the release the node keeps
    /// the pointer even when it moves away, and the handler runs only when the
    /// release happens over the node again. A press released elsewhere runs
    /// nothing.
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Point, Size};
    /// use smudge_core::input::{InputEvent, PointerButton};
    /// use smudge_core::style::px;
    /// use smudge_core::widget::panel;
    ///
    /// let presses = Rc::new(Cell::new(0u32));
    /// let counted = Rc::clone(&presses);
    /// let mut app = App::new(
    ///     AppConfig {
    ///         viewport: Size {
    ///             width: 100.0,
    ///             height: 60.0,
    ///         },
    ///         scale: 1.0,
    ///         clear_color: Color::WHITE,
    ///         repaint: RepaintMode::Damage,
    ///     },
    ///     move || {
    ///         panel().width(px(40.0)).height(px(20.0)).on_press(move || {
    ///             counted.set(counted.get() + 1);
    ///         })
    ///     },
    /// );
    /// app.frame();
    ///
    /// let corner = Point { x: 10.0, y: 5.0 };
    /// app.handle_event(InputEvent::PointerMoved { position: corner });
    /// app.handle_event(InputEvent::PointerDown {
    ///     position: corner,
    ///     button: PointerButton::Primary,
    /// });
    /// app.handle_event(InputEvent::PointerUp {
    ///     position: corner,
    ///     button: PointerButton::Primary,
    /// });
    /// assert_eq!(presses.get(), 1);
    /// ```
    pub fn on_press(mut self, handler: impl FnMut() + 'static) -> Self {
        self.handlers.on_press = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when the node enters (`true`) or leaves (`false`) the
    /// hovered chain.
    ///
    /// The hovered chain is the nearest enabled node under the pointer that can
    /// respond and its enabled ancestors, so a container stays hovered while
    /// the pointer is over one of its children, and a disabled node never
    /// hears it. While a press is held, hover does not change until the
    /// button is released. The handler runs inside the node's owner, so it can
    /// write the signals the element's bindings read.
    pub fn on_hover(mut self, handler: impl FnMut(bool) + 'static) -> Self {
        self.handlers.on_hover = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when a pointer button is pressed on the node.
    ///
    /// The position is in logical pixels, relative to the top left of the
    /// window. Every button runs the handler; only the primary button also
    /// starts a press that [`Element::on_press`] hears.
    pub fn on_pointer_down(mut self, handler: impl FnMut(Point) + 'static) -> Self {
        self.handlers.on_pointer_down = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when a primary press targets a node that is neither this
    /// node nor one of its descendants.
    ///
    /// A press on the node itself, on a child, or on a deeper descendant does
    /// not run it, so a popup asks for presses outside itself and closes on
    /// the ones it gets. A press that reaches no node at all is outside every
    /// node.
    pub fn on_press_outside(mut self, handler: impl FnMut() + 'static) -> Self {
        self.handlers.on_press_outside = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when the wheel scrolls while the node is in the hovered
    /// chain, which is the case when the pointer is over the node or over one
    /// of its descendants.
    ///
    /// The delta is in logical pixels, positive for scrolling right and down.
    /// The deepest node in the hovered chain that has a wheel handler takes
    /// the whole event, so of two nested scroll views the one under the
    /// pointer scrolls and the one around it does not. The handler runs inside
    /// the node's owner, so it can write the signals the element's bindings
    /// read.
    pub fn on_wheel(mut self, handler: impl FnMut(Point) + 'static) -> Self {
        self.handlers.on_wheel = Some(Box::new(handler));
        self
    }

    /// Disables the node, which takes it out of hit testing, or enables it
    /// again.
    ///
    /// A disabled node receives no pointer events of its own: its handlers
    /// never run, and a press over it passes to whatever enabled node is hit
    /// instead. Its children are still hit. This can be bound to a signal,
    /// like the style builders.
    pub fn disabled(mut self, disabled: impl Into<Value<bool>>) -> Self {
        match disabled.into() {
            Value::Static(disabled) => self.disabled = disabled,
            Value::Dynamic(get) => self.bindings.push(Binding::disabled(get)),
        }
        self
    }

    /// Makes the node able to take the keyboard focus, by a press on it or by
    /// Tab navigation.
    ///
    /// While a node holds the focus, key events reach it before they bubble to
    /// its ancestors, which is what [`Element::on_key`] hears.
    /// [`Element::on_key`] implies this, so a key handler never needs a
    /// separate `focusable` call.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    /// Runs `handler` when a key event reaches the node while the node or one
    /// of its descendants holds the keyboard focus.
    ///
    /// The event walks up from the focused node, and the first handler that
    /// returns [`Handled::Yes`] stops it, so a handler that takes a key hides
    /// it from its ancestors. Tab moves the focus away only when no handler
    /// took the Tab key. This makes the node focusable, so a press on it or a
    /// Tab puts the focus on it.
    ///
    /// The handler runs inside the node's owner, so it can write the signals
    /// the element's bindings read.
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::input::{Handled, InputEvent, Key, KeyEvent};
    /// use smudge_core::style::px;
    /// use smudge_core::widget::panel;
    ///
    /// let enters = Rc::new(Cell::new(0u32));
    /// let counted = Rc::clone(&enters);
    /// let mut app = App::new(
    ///     AppConfig {
    ///         viewport: Size {
    ///             width: 100.0,
    ///             height: 60.0,
    ///         },
    ///         scale: 1.0,
    ///         clear_color: Color::WHITE,
    ///         repaint: RepaintMode::Damage,
    ///     },
    ///     move || {
    ///         panel().child(
    ///             panel()
    ///                 .width(px(40.0))
    ///                 .height(px(20.0))
    ///                 .on_key(move |event: &KeyEvent| {
    ///                     if event.pressed && event.key == Key::Enter {
    ///                         counted.set(counted.get() + 1);
    ///                         Handled::Yes
    ///                     } else {
    ///                         Handled::No
    ///                     }
    ///                 }),
    ///         )
    ///     },
    /// );
    /// app.frame();
    ///
    /// // Tab moves the focus to the panel, which then hears the Enter key.
    /// app.handle_event(InputEvent::Key(KeyEvent {
    ///     key: Key::Tab,
    ///     pressed: true,
    ///     repeat: false,
    ///     modifiers: Default::default(),
    ///     text: None,
    /// }));
    /// app.handle_event(InputEvent::Key(KeyEvent {
    ///     key: Key::Enter,
    ///     pressed: true,
    ///     repeat: false,
    ///     modifiers: Default::default(),
    ///     text: None,
    /// }));
    /// assert_eq!(enters.get(), 1);
    /// ```
    pub fn on_key(mut self, handler: impl FnMut(&KeyEvent) -> Handled + 'static) -> Self {
        self.handlers.on_key = Some(Box::new(handler));
        self.focusable = true;
        self
    }

    /// Runs the handler set with [`Element::on_press`] when the node holds
    /// the keyboard focus and a fresh press of Enter or Space arrives.
    ///
    /// This is how a button fires from the keyboard with the same handler its
    /// pointer press uses. The activation runs before any
    /// [`Element::on_key`] handler on the same node, and a key repeat fires
    /// nothing. A press handler that ran takes the key, so it does not reach
    /// an ancestor; without a press handler the key bubbles as usual.
    ///
    /// This makes the node focusable, like [`Element::on_key`] does.
    pub fn activate_on_keys(mut self) -> Self {
        self.handlers.activate_on_keys = true;
        self.focusable = true;
        self
    }

    /// Runs `handler` when the node gains (`true`) or loses (`false`) the
    /// keyboard focus.
    ///
    /// The handler does not make the node focusable; pair it with
    /// [`Element::focusable`] for that. It runs inside the node's owner, so it
    /// can write the signals the element's bindings read. A node that is
    /// removed while focused clears the focus without hearing `on_focus` again,
    /// because its handlers are being torn down with it.
    pub fn on_focus(mut self, handler: impl FnMut(bool) + 'static) -> Self {
        self.handlers.on_focus = Some(Box::new(handler));
        self
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
    /// The style patches per interaction state, when a variant builder ran.
    variants: Option<Variants>,
    /// The input handlers the node will own.
    handlers: Handlers,
    /// Whether the node starts disabled.
    disabled: bool,
    /// Whether the node can take the keyboard focus.
    focusable: bool,
    /// The scroll offset written to the node when it mounts.
    scroll_offset: Point,
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
            variants: element.variants,
            handlers: element.handlers,
            disabled: element.disabled,
            focusable: element.focusable,
            scroll_offset: element.scroll_offset,
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

impl IntoElement for AnyElement {
    fn into_any(self) -> AnyElement {
        self
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
        variants,
        handlers,
        disabled,
        focusable,
        scroll_offset,
        children,
    } = element;
    let id = tree.insert(parent, widget, style);
    tree.set_variants(id, variants.map(Rc::new));
    tree.set_disabled(id, disabled);
    tree.set_focusable(id, focusable);
    tree.set_handlers(id, handlers);
    if scroll_offset != Point::default() {
        tree.set_scroll_offset(id, scroll_offset);
    }
    let owner = tree.owner(id).expect("the node was just inserted").clone();
    for binding in bindings {
        owner.with(|| (binding.install)(id, queue));
    }
    for child in children {
        mount(child, tree, queue, Some(id));
    }
    id
}

/// A node that paints nothing and leaves its children to a binding.
///
/// [`show`] mounts its child under a slot, and [`for_each`] keeps one child
/// per key of a list under one.
pub struct Slot;

impl Widget for Slot {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// Mounts the element that `build` returns as a child of `slot`, inside an
/// owner of its own.
///
/// The owner keeps the values the build creates alive for as long as the
/// child stays mounted, and [`unmount_child`] disposes them. Returns `None`
/// when `slot` is a stale id, which happens when the slot was removed while
/// an update that mounts into it was still queued.
fn mount_under_slot(
    slot: WidgetId,
    tree: &mut WidgetTree,
    queue: &UpdateQueue,
    build: impl FnOnce() -> AnyElement,
) -> Option<(WidgetId, Owner)> {
    let owner = tree.owner(slot)?.child();
    let element = owner.with(build);
    let child = mount(element, tree, queue, Some(slot));
    Some((child, owner))
}

/// Removes a child that [`mount_under_slot`] mounted and disposes the owner
/// its build ran in.
///
/// The tree removal runs first, so the child's bindings stop before the
/// values the build created are disposed.
fn unmount_child(child: WidgetId, owner: &Owner, tree: &mut WidgetTree) {
    tree.remove(child);
    owner.cleanup();
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
            RenderEffect::new(move || {
                let visible = when();
                let child_queue = queue.clone();
                let mounted = Rc::clone(&mounted);
                let build = Rc::clone(&build);
                queue.push(Box::new(move |tree| {
                    let mut mounted = mounted.borrow_mut();
                    match (visible, mounted.take()) {
                        (true, None) => {
                            if let Some(child) =
                                mount_under_slot(id, tree, &child_queue, || build())
                            {
                                *mounted = Some(child);
                            }
                        }
                        (false, Some((child, owner))) => unmount_child(child, &owner, tree),
                        (_, unchanged) => *mounted = unchanged,
                    }
                }));
            })
        }),
    });
    element
}

/// Mounts one child per key of the list that `items` returns, keeping the
/// children that survive a change mounted and in place.
///
/// The returned element is a slot whose children follow the list. A key the
/// list gains mounts a new child, a key it loses removes that child's whole
/// tree, and a key that moves keeps its child, which is reordered rather than
/// rebuilt. A child keeps its reactive state across a reorder, and the frame
/// repaints only what changed position or stacking.
///
/// A row is built once per key, by `row`, the first time the key appears in
/// the list. Content that changes while a row stays mounted must come from
/// the signals that row reads: a new value for a key the list already had
/// reaches no mounted child.
///
/// The keys of a list must be unique. In a debug build a duplicate key trips
/// a debug assertion. In a release build the last duplicate wins: the row
/// mounted for that key takes the position of its last occurrence, and the
/// earlier occurrences contribute nothing.
///
/// `items` runs inside a binding, so only the signals it reads make the slot
/// diff the list again. `key` should depend on its item alone, because a key
/// that changes makes the slot remove one child and mount another. `row` runs
/// once per new key inside an owner of its own, so the signals a row creates
/// live exactly as long as the row.
///
/// The slot manages its own children; do not add children to the returned
/// element beside it.
///
/// ```
/// use smudge_core::element::for_each;
/// use smudge_core::reactive::{self, prelude::*};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
///
/// let (items, _set_items) = reactive::signal_local(vec![1u32, 2, 3]);
/// let list = for_each(
///     move || items.get(),
///     |item| *item,
///     |item| panel().width(px(40.0)).height(px(10.0)),
/// );
/// ```
pub fn for_each<T, K, E>(
    items: impl Fn() -> Vec<T> + 'static,
    key: impl Fn(&T) -> K + 'static,
    row: impl Fn(T) -> E + 'static,
) -> Element<Slot>
where
    T: 'static,
    K: Eq + Hash + Clone + 'static,
    E: IntoElement,
{
    let items = Rc::new(items);
    let key = Rc::new(key);
    let row: Rc<dyn Fn(T) -> AnyElement> = Rc::new(move |item| row(item).into_any());
    let mut element = Element::new(Slot);
    element.bindings.push(Binding {
        install: Box::new(move |slot, queue| {
            let queue = queue.clone();
            let mounted: Rc<RefCell<HashMap<K, (WidgetId, Owner)>>> =
                Rc::new(RefCell::new(HashMap::new()));
            let order: Rc<RefCell<Vec<K>>> = Rc::new(RefCell::new(Vec::new()));
            RenderEffect::new(move || {
                let items = items();
                let keys: Vec<K> = items.iter().map(|item| key(item)).collect();
                // The last occurrence of a key decides where its row goes, so
                // a duplicate keeps one row and the earlier ones are skipped.
                let mut last: HashMap<K, usize> = HashMap::with_capacity(keys.len());
                for (at, item_key) in keys.iter().enumerate() {
                    last.insert(item_key.clone(), at);
                }
                debug_assert!(
                    last.len() == keys.len(),
                    "for_each found a duplicate key, but the keys of a list must be unique"
                );
                let child_queue = queue.clone();
                let mounted = Rc::clone(&mounted);
                let order = Rc::clone(&order);
                let row = Rc::clone(&row);
                queue.push(Box::new(move |tree| {
                    let mut mounted = mounted.borrow_mut();
                    let mut order = order.borrow_mut();

                    mounted.retain(|key, (child, owner)| {
                        let kept = last.contains_key(key);
                        if !kept {
                            unmount_child(*child, owner, tree);
                        }
                        kept
                    });

                    let mut new_order = Vec::with_capacity(last.len());
                    let mut children = Vec::with_capacity(last.len());
                    for (at, (item, item_key)) in items.into_iter().zip(keys).enumerate() {
                        if last[&item_key] != at {
                            continue;
                        }
                        let child = match mounted.get(&item_key) {
                            Some((child, _)) => *child,
                            None => {
                                let row = Rc::clone(&row);
                                let Some((child, owner)) =
                                    mount_under_slot(slot, tree, &child_queue, move || row(item))
                                else {
                                    continue;
                                };
                                mounted.insert(item_key.clone(), (child, owner));
                                child
                            }
                        };
                        children.push(child);
                        new_order.push(item_key);
                    }

                    if *order != new_order {
                        tree.set_children(slot, children);
                        *order = new_order;
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
