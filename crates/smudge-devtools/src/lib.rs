//! Devtools that Smudge draws over a running app's window.
//!
//! The crate builds the devtools overlay: a panel on a root as large as the
//! window, meant to run as an app of its own drawn over the app a window
//! shows, so the app keeps working underneath and its own frames, numbers
//! and pixels stay its own.

mod access;
mod inspector;
mod marks;
mod reactive;
mod timeline;

use std::cell::Cell;
use std::rc::Rc;

use marks::Marks;
pub use timeline::FrameRecord;
use timeline::{HEIGHT, Timeline, TimelineView};

use smudge_core::element::{AnyElement, Element, IntoElement, Slot, for_each, show};
use smudge_core::geometry::{Point, Rect, Size};
use smudge_core::input::{Drag, Handled};
use smudge_core::inspect::{
    A11yWarning, BoxModel, CycleInfo, FocusInfo, NodeDetails, OwnerEntry, PointerInfo, TreeEntry,
};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::SemanticsNode;
use smudge_core::style::{Edges, Length, Overflow, pct, px};
use smudge_core::theme::ColorToken;
use smudge_core::tree::{Dirty, WidgetId};
use smudge_core::widget::{Panel, panel};
use smudge_text::label;
use smudge_widgets::{button, column, row};

/// Where the panel opens, from the window's top left, in logical pixels.
const START: Point = Point { x: 16.0, y: 16.0 };

/// The width of the panel, in logical pixels.
const WIDTH: f32 = 320.0;

/// The width of the panel's border, in logical pixels.
const BORDER: f32 = 1.0;

/// The padding around the panel's body, in logical pixels.
const PAD: f32 = 8.0;

/// The width of the timeline, in logical pixels: the panel's inside.
const TIMELINE_WIDTH: f32 = WIDTH - 2.0 * (BORDER + PAD);

/// How much of the panel's top stays in the window when a drag or a resize
/// takes it toward the bottom edge, in logical pixels, so its title bar can
/// be grabbed again.
const KEEP: f32 = 32.0;

/// What the devtools show, which the window that runs them feeds and the
/// overlay reads. Cloning the handle is cheap, and every clone shares the
/// same state.
#[derive(Clone)]
pub struct Tools {
    /// The app's last frames.
    timeline: RwSignal<Rc<Timeline>>,
    /// Which switches are on.
    switches: RwSignal<Switches>,
    /// The rectangles the app's last frame repainted, in logical pixels.
    damage: RwSignal<Rc<Vec<Rect>>>,
    /// The rectangles the app's recent frames repainted, in logical pixels,
    /// each with the opacity it flashes at.
    flashes: RwSignal<Rc<Vec<(Rect, u8)>>>,
    /// The rectangles of the app's nodes, in logical pixels, each with its
    /// depth in the tree.
    bounds: RwSignal<Rc<Vec<(usize, Rect)>>>,
    /// The tab the panel shows.
    tab: RwSignal<Tab>,
    /// What the reactive tab shows, as text.
    reactive: RwSignal<String>,
    /// What the accessibility tab shows, as text.
    access: RwSignal<String>,
    /// Where the app's focus is and where Tab takes it, while the focus
    /// overlay is on.
    focus: RwSignal<Option<Rc<FocusInfo>>>,
    /// What the app's pointer reaches, while the hit testing overlay is on.
    pointer: RwSignal<Option<Rc<PointerInfo>>>,
    /// Whether pick mode is on, so the pointer picks a node of the app.
    picking: RwSignal<bool>,
    /// Every node of the app's tree, hidden ones included.
    tree: RwSignal<Rc<Vec<TreeEntry>>>,
    /// The node the inspector shows.
    selected: RwSignal<Option<WidgetId>>,
    /// The details of the selected node.
    details: RwSignal<Option<Rc<NodeDetails>>>,
    /// The boxes the overlay highlights: the node under the pointer while
    /// picking, and the selected node otherwise.
    highlight: RwSignal<Option<BoxModel>>,
    /// Whether "Copy as test" was pressed and the window has not copied yet.
    copy: RwSignal<bool>,
    /// What the last copy wrote, or why it failed.
    copied: RwSignal<Option<String>>,
}

/// The devtools' switches, which the panel's buttons turn on and off and the
/// window that runs the devtools follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Switches {
    /// Whether the overlay tints the pixels the app's last frame repainted,
    /// with the color and opacity of the window's fixed damage overlay.
    pub damage_tint: bool,
    /// Whether the overlay flashes the pixels the app's recent frames
    /// repainted, each flash fading out after its frame.
    pub paint_flashing: bool,
    /// Whether the overlay outlines the rectangle of every node of the app,
    /// in a color that follows its depth in the tree. A node that a scroll
    /// view or a clip hides is outlined too, where layout put it.
    pub layout_bounds: bool,
    /// Whether the app repaints its whole surface on every frame, instead of
    /// only what changed.
    pub full_repaint: bool,
    /// Whether the overlay shows where the keyboard focus is: the focused
    /// node, the trap that holds the keyboard, and every node Tab visits
    /// with its number in the tab order.
    pub focus: bool,
    /// Whether the overlay shows what the pointer reaches: the hovered node
    /// and its ancestors, the node a held press began on, and every node
    /// that takes a press.
    pub hit_testing: bool,
}

impl Default for Tools {
    fn default() -> Self {
        Tools::new()
    }
}

impl Tools {
    /// Creates the devtools state with no frames recorded.
    pub fn new() -> Tools {
        Tools {
            timeline: RwSignal::new(Rc::new(Timeline::default())),
            switches: RwSignal::new(Switches::default()),
            damage: RwSignal::new(Rc::new(Vec::new())),
            flashes: RwSignal::new(Rc::new(Vec::new())),
            bounds: RwSignal::new(Rc::new(Vec::new())),
            tab: RwSignal::new(Tab::Frames),
            reactive: RwSignal::new(String::new()),
            access: RwSignal::new(String::new()),
            focus: RwSignal::new(None),
            pointer: RwSignal::new(None),
            picking: RwSignal::new(false),
            tree: RwSignal::new(Rc::new(Vec::new())),
            selected: RwSignal::new(None),
            details: RwSignal::new(None),
            highlight: RwSignal::new(None),
            copy: RwSignal::new(false),
            copied: RwSignal::new(None),
        }
    }

    /// Returns which switches are on.
    pub fn switches(&self) -> Switches {
        self.switches.get()
    }

    /// Turns the switches on and off as `switches` says, as the panel's
    /// buttons do.
    ///
    /// With the damage tint, paint flashing or layout bounds off, the
    /// rectangles it would mark are dropped, so turning it on again marks
    /// nothing until the window hands it new ones.
    pub fn set_switches(&self, switches: Switches) {
        self.switches.set(switches);
        if !switches.damage_tint && self.damage.with(|damage| !damage.is_empty()) {
            self.show_damage([]);
        }
        if !switches.paint_flashing && self.flashes.with(|flashes| !flashes.is_empty()) {
            self.show_flashes([]);
        }
        if !switches.layout_bounds && self.bounds.with(|bounds| !bounds.is_empty()) {
            self.show_bounds([]);
        }
        if !switches.focus && self.focus.with(Option::is_some) {
            self.show_focus(None);
        }
        if !switches.hit_testing && self.pointer.with(Option::is_some) {
            self.show_pointer(None);
        }
    }

    /// Drops the rectangles the damage tint, paint flashing and layout
    /// bounds mark, as when the panel hides and no longer hears the app's
    /// frames.
    pub fn forget_marks(&self) {
        if self.damage.with(|damage| !damage.is_empty()) {
            self.show_damage([]);
        }
        if self.flashes.with(|flashes| !flashes.is_empty()) {
            self.show_flashes([]);
        }
        if self.bounds.with(|bounds| !bounds.is_empty()) {
            self.show_bounds([]);
        }
        if self.highlight.with(Option::is_some) {
            self.show_highlight(None);
        }
        if self.focus.with(Option::is_some) {
            self.show_focus(None);
        }
        if self.pointer.with(Option::is_some) {
            self.show_pointer(None);
        }
        if self.picking.get() {
            self.picking.set(false);
        }
    }

    /// Returns whether the inspector tab shows, so the window hands it the
    /// app's tree and the selected node's details.
    pub fn is_inspecting(&self) -> bool {
        self.tab.get() == Tab::Inspector
    }

    /// Returns whether the accessibility tab shows, so the window hands it
    /// the app's accessibility tree and warnings.
    pub fn is_auditing(&self) -> bool {
        self.tab.get() == Tab::Access
    }

    /// Takes `nodes`, the app's accessibility tree in paint order, and
    /// `warnings`, the nodes assistive technology cannot describe well, for
    /// the accessibility tab.
    pub fn show_access(&self, nodes: &[SemanticsNode], warnings: &[A11yWarning]) {
        self.access.set(access::describe(nodes, warnings));
    }

    /// Takes where the app's focus is, or `None` when the focus overlay is
    /// off.
    pub fn show_focus(&self, focus: Option<FocusInfo>) {
        self.focus.set(focus.map(Rc::new));
    }

    /// Takes what the app's pointer reaches, or `None` when the hit testing
    /// overlay is off.
    pub fn show_pointer(&self, pointer: Option<PointerInfo>) {
        self.pointer.set(pointer.map(Rc::new));
    }

    /// Returns whether the reactive tab shows, so the window hands it the
    /// app's busiest effects, cycles and owners.
    pub fn is_tracing(&self) -> bool {
        self.tab.get() == Tab::Reactive
    }

    /// Takes what the reactive tab shows: `busiest`, the places effects were
    /// created with how many times a second they ran there, busiest first,
    /// `None` standing for the element bindings the library creates;
    /// `cycles`, the cycles a frame cut short; and `owners`, the app's owner
    /// tree, parents first.
    pub fn show_reactive(
        &self,
        busiest: &[(Option<&'static std::panic::Location<'static>>, f64)],
        cycles: &[CycleInfo],
        owners: &[OwnerEntry],
    ) {
        self.reactive
            .set(reactive::describe(busiest, cycles, owners));
    }

    /// Returns whether pick mode is on, so the window sends the pointer to
    /// [`Tools::show_highlight`] and [`Tools::select`] instead of the app.
    pub fn is_picking(&self) -> bool {
        self.picking.get()
    }

    /// Turns pick mode off, as a pick does.
    pub fn stop_picking(&self) {
        self.picking.set(false);
    }

    /// Returns the node the inspector shows.
    pub fn selected(&self) -> Option<WidgetId> {
        self.selected.get()
    }

    /// Makes `id` the node the inspector shows.
    pub fn select(&self, id: Option<WidgetId>) {
        self.selected.set(id);
    }

    /// Takes `tree`, every node of the app's tree, which the inspector lists.
    pub fn show_tree(&self, tree: impl IntoIterator<Item = TreeEntry>) {
        self.tree.update(|shown| {
            let shown = Rc::make_mut(shown);
            shown.clear();
            shown.extend(tree);
        });
    }

    /// Takes the details of the selected node, or `None` when no node is
    /// selected or it is gone.
    pub fn show_details(&self, details: Option<NodeDetails>) {
        self.details.set(details.map(Rc::new));
    }

    /// Returns whether "Copy as test" was pressed since the last call, and
    /// clears the request, so the window copies the selected node's subtree
    /// once.
    pub fn take_copy_request(&self) -> bool {
        let requested = self.copy.get();
        if requested {
            self.copy.set(false);
        }
        requested
    }

    /// Takes `message`, which says what the last copy wrote or why it
    /// failed, for the inspector to show.
    pub fn show_copied(&self, message: String) {
        self.copied.set(Some(message));
    }

    /// Takes the boxes the overlay highlights, or `None` for none.
    pub fn show_highlight(&self, boxes: Option<BoxModel>) {
        self.highlight.set(boxes);
    }

    /// Takes `bounds`, the rectangles of the app's nodes in logical pixels,
    /// each with its depth in the tree, which layout bounds outlines while
    /// it is on.
    pub fn show_bounds(&self, bounds: impl IntoIterator<Item = (usize, Rect)>) {
        self.bounds.update(|shown| {
            let shown = Rc::make_mut(shown);
            shown.clear();
            shown.extend(bounds);
        });
    }

    /// Takes `rects`, the rectangles the app's last frame repainted in
    /// logical pixels, which the damage tint covers while it is on.
    pub fn show_damage(&self, rects: impl IntoIterator<Item = Rect>) {
        self.damage.update(|damage| {
            let damage = Rc::make_mut(damage);
            damage.clear();
            damage.extend(rects);
        });
    }

    /// Takes `flashes`, the rectangles the app's recent frames repainted in
    /// logical pixels, each with the opacity it flashes at out of 255, which
    /// paint flashing shows while it is on. The window fades them as their
    /// frames grow older.
    pub fn show_flashes(&self, flashes: impl IntoIterator<Item = (Rect, u8)>) {
        self.flashes.update(|shown| {
            let shown = Rc::make_mut(shown);
            shown.clear();
            shown.extend(flashes);
        });
    }

    /// Records one frame of the app, which the timeline shows as its newest
    /// bar.
    pub fn record(&self, record: FrameRecord) {
        self.timeline
            .update(|timeline| Rc::make_mut(timeline).push(record));
    }
}

/// Builds the devtools overlay: one panel over a root as large as the
/// window, whose size in logical pixels `viewport` reads, showing what
/// `tools` holds.
///
/// The root takes no pointer input of its own, so wherever no panel is, the
/// pointer reaches the app underneath. The panel moves with a drag on its
/// title bar, and the button on the bar collapses it to the bar and opens it
/// again. It stays inside the window, across a drag and a resize, as far as
/// the window is wide and tall enough to hold its title bar.
///
/// The panel shows the app's last frames as a timeline of stacked bars, one
/// per frame and one color per stage, with a line at the budget of a frame
/// at 60 frames a second. Hovering a bar describes its frame below. Under
/// the timeline, buttons turn the [`Switches`] on and off, and the root
/// draws what they show over the app, under the panel.
///
/// ```
/// use smudge_core::geometry::Size;
/// use smudge_devtools::Tools;
///
/// let tools = Tools::new();
/// let overlay = smudge_devtools::overlay(&tools, || Size {
///     width: 800.0,
///     height: 600.0,
/// });
/// ```
pub fn overlay(tools: &Tools, viewport: impl Fn() -> Size + 'static) -> AnyElement {
    let viewport: Rc<dyn Fn() -> Size> = Rc::new(viewport);
    let at = RwSignal::new(START);
    let open = RwSignal::new(true);
    // Where the press that grabbed the title bar landed on it, which stays
    // under the pointer while the drag moves the panel.
    let grab: Rc<Cell<Option<Point>>> = Rc::new(Cell::new(None));

    let place = at.clone();
    let shown = open.clone();
    let caption = open.clone();
    let toggle = open.clone();
    let grabbed = Rc::clone(&grab);
    let title = row()
        .padding(px(6.0))
        .background(ColorToken::SURFACE_HOVER)
        .on_pointer_down(move |point| grabbed.set(Some(point)))
        .child(label("Devtools").grow(1.0))
        .child(
            button(move || String::from(if caption.get() { "Collapse" } else { "Open" }))
                .on_press(move || toggle.update(|open| *open = !*open)),
        );
    let window = panel()
        .absolute()
        .inset({
            let viewport = Rc::clone(&viewport);
            move || {
                let at = inside(place.get(), viewport());
                Edges {
                    left: px(at.x),
                    top: px(at.y),
                    right: Length::Auto,
                    bottom: Length::Auto,
                }
            }
        })
        .width(px(WIDTH))
        .column()
        .background(ColorToken::SURFACE)
        .border(BORDER)
        .border_color(ColorToken::BORDER)
        .corner_radius(6.0)
        .overflow(Overflow::Clip)
        .child(title)
        .child(show(move || shown.get(), {
            let tools = tools.clone();
            move || body(&tools)
        }));

    // The root hears the drags that begin on the title bar, in its own
    // coordinates, which stay put while the panel moves. The title bar sits
    // inside the border.
    let (switches, damage) = (tools.switches.clone(), tools.damage.clone());
    let (flashing, flashes) = (tools.switches.clone(), tools.flashes.clone());
    let (outlining, bounds) = (tools.switches.clone(), tools.bounds.clone());
    let highlight = tools.highlight.clone();
    let (focus, pointer) = (tools.focus.clone(), tools.pointer.clone());
    let badges = tools.focus.clone();
    Element::new(Marks::default())
        .bind(
            Focused,
            move || focus.get(),
            |marks, focus| {
                marks.focus = focus;
                Dirty::Paint
            },
        )
        .bind(
            Pointed,
            move || pointer.get(),
            |marks, pointer| {
                marks.pointer = pointer;
                Dirty::Paint
            },
        )
        .bind(
            Highlighted,
            move || highlight.get(),
            |marks, boxes| {
                marks.highlight = boxes;
                Dirty::Paint
            },
        )
        .bind(
            Outlined,
            move || outlining.get().layout_bounds.then(|| bounds.get()),
            |marks, bounds| {
                marks.bounds.clear();
                if let Some(bounds) = bounds {
                    marks.bounds.extend_from_slice(&bounds);
                }
                Dirty::Paint
            },
        )
        .bind(
            Flashed,
            move || flashing.get().paint_flashing.then(|| flashes.get()),
            |marks, flashes| {
                marks.flashes.clear();
                if let Some(flashes) = flashes {
                    marks.flashes.extend_from_slice(&flashes);
                }
                Dirty::Paint
            },
        )
        .bind(
            Tinted,
            move || switches.get().damage_tint.then(|| damage.get()),
            |marks, damage| {
                marks.damage.clear();
                if let Some(damage) = damage {
                    marks.damage.extend_from_slice(&damage);
                }
                Dirty::Paint
            },
        )
        .width(pct(100.0))
        .height(pct(100.0))
        .on_drag(move |step| match (step, grab.get()) {
            (Drag::Moved(point), Some(grab)) => {
                let wanted = Point {
                    x: point.x - grab.x - BORDER,
                    y: point.y - grab.y - BORDER,
                };
                at.set(inside(wanted, viewport()));
                Handled::Yes
            }
            (Drag::Moved(_), None) => Handled::No,
            _ => {
                grab.set(None);
                Handled::No
            }
        })
        .child(tab_order(badges))
        .child(window)
        .into_any()
}

/// The marker of the damage the root tints, which [`Element::bind`] keys on.
struct Tinted;

/// The marker of the damage the root flashes, which [`Element::bind`] keys
/// on.
struct Flashed;

/// The marker of the node rectangles the root outlines, which
/// [`Element::bind`] keys on.
struct Outlined;

/// The marker of the boxes the root highlights, which [`Element::bind`]
/// keys on.
struct Highlighted;

/// The marker of the focus the root marks, which [`Element::bind`] keys on.
struct Focused;

/// The marker of the pointer state the root marks, which [`Element::bind`]
/// keys on.
struct Pointed;

/// Builds the tab order numbers: a small badge at the top left corner of
/// every node Tab visits, while the focus overlay is on. They sit under the
/// panel and take no pointer input.
fn tab_order(focus: RwSignal<Option<Rc<FocusInfo>>>) -> Element<Slot> {
    let listed = focus.clone();
    for_each(
        move || {
            listed.with(|focus| (0..focus.as_ref().map_or(0, |focus| focus.order.len())).collect())
        },
        |index: &usize| *index,
        move |index| {
            let place = focus.clone();
            label(format!("{}", index + 1))
                .absolute()
                .inset(move || {
                    let at = place.with(|focus| {
                        focus
                            .as_ref()
                            .and_then(|focus| focus.order.get(index))
                            .map(|(_, rect)| (rect.x0, rect.y0))
                    });
                    let (x, y) = at.unwrap_or_default();
                    Edges {
                        left: px(x),
                        top: px(y),
                        right: Length::Auto,
                        bottom: Length::Auto,
                    }
                })
                .padding(px(1.0))
                .background(ColorToken::ACCENT)
                .text_color(ColorToken::ACCENT_TEXT)
        },
    )
}

/// Returns `at` moved as little as it takes for a panel there to stay inside
/// a window of `viewport`: its left and top edges in the window, its right
/// edge too while the window is wide enough, and at least [`KEEP`] of it
/// above the bottom edge.
fn inside(at: Point, viewport: Size) -> Point {
    Point {
        x: at.x.min(viewport.width - WIDTH).max(0.0),
        y: at.y.min(viewport.height - KEEP).max(0.0),
    }
}

/// The marker of the timeline's frames, which [`Element::bind`] keys on.
struct Frames;

/// Builds the panel's body: the frame timeline, and a line that describes
/// the frame under the pointer. The line follows the bars as new frames
/// move them, so it always describes the bar the pointer is over.
fn body(tools: &Tools) -> Element<Panel> {
    let tab = |name: &'static str, which: Tab| {
        let (shown, choose) = (tools.tab.clone(), tools.tab.clone());
        button(move || {
            let mark = if shown.get() == which { "> " } else { "" };
            format!("{mark}{name}")
        })
        .on_press(move || choose.set(which))
    };
    let shown = |which: Tab, build: fn(&Tools) -> Element<Panel>| {
        let (tab, tools) = (tools.tab.clone(), tools.clone());
        show(move || tab.get() == which, move || build(&tools))
    };
    column()
        .width(pct(100.0))
        .padding(px(PAD))
        .gap(Size {
            width: PAD,
            height: PAD,
        })
        .child(
            row()
                .gap(Size {
                    width: PAD,
                    height: PAD,
                })
                .child(tab("Frames", Tab::Frames))
                .child(tab("Inspector", Tab::Inspector))
                .child(tab("Reactive", Tab::Reactive))
                .child(tab("Access", Tab::Access)),
        )
        .child(shown(Tab::Frames, frames_tab))
        .child(shown(Tab::Inspector, inspector::inspector))
        .child(shown(Tab::Reactive, reactive::reactive_tab))
        .child(shown(Tab::Access, access::access_tab))
}

/// The tabs of the panel.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tab {
    /// The frame timeline and the switches.
    Frames,
    /// The node picker, the tree and a node's details.
    Inspector,
    /// The busiest effects, the cycles and the owner tree.
    Reactive,
    /// The accessibility tree and its warnings.
    Access,
}

/// Builds the frames tab: the timeline, the switches, and a line that
/// describes the frame under the pointer.
fn frames_tab(tools: &Tools) -> Element<Panel> {
    let hovered: RwSignal<Option<f32>> = RwSignal::new(None);
    let frames = tools.timeline.clone();
    let described = tools.timeline.clone();
    let (over, left, shown) = (hovered.clone(), hovered.clone(), hovered);
    let timeline = Element::new(TimelineView::default())
        .width(px(TIMELINE_WIDTH))
        .height(px(HEIGHT))
        .bind(
            Frames,
            move || frames.get(),
            |view, timeline| {
                view.timeline.copy_from(&timeline);
                Dirty::Paint
            },
        )
        .on_pointer_move(move |point| {
            over.set(Some(point.x));
        })
        .on_hover(move |inside| {
            if !inside {
                left.set(None);
            }
        });
    column()
        .width(pct(100.0))
        .gap(Size {
            width: PAD,
            height: PAD,
        })
        .child(timeline)
        .child(
            row()
                .wrap(true)
                .gap(Size {
                    width: PAD,
                    height: PAD,
                })
                .child(switch(tools, "Damage tint", |switches| {
                    &mut switches.damage_tint
                }))
                .child(switch(tools, "Paint flashing", |switches| {
                    &mut switches.paint_flashing
                }))
                .child(switch(tools, "Layout bounds", |switches| {
                    &mut switches.layout_bounds
                }))
                .child(switch(tools, "Focus", |switches| &mut switches.focus))
                .child(switch(tools, "Hit testing", |switches| {
                    &mut switches.hit_testing
                }))
                .child(switch(tools, "Full repaint", |switches| {
                    &mut switches.full_repaint
                })),
        )
        .child(label(move || {
            let x = shown.get();
            described.with(|timeline| {
                x.and_then(|x| timeline.at(x, TIMELINE_WIDTH)).map_or_else(
                    || String::from("Hover a bar to see its frame."),
                    FrameRecord::describe,
                )
            })
        }))
}

/// Builds the button that turns the switch `field` picks on and off, named
/// `name` and showing its state.
fn switch(
    tools: &Tools,
    name: &'static str,
    field: fn(&mut Switches) -> &mut bool,
) -> Element<Panel> {
    let read = tools.switches.clone();
    let write = tools.clone();
    button(move || {
        let mut switches = read.get();
        let state = if *field(&mut switches) { "on" } else { "off" };
        format!("{name}: {state}")
    })
    .on_press(move || {
        let mut switches = write.switches();
        let on = field(&mut switches);
        *on = !*on;
        write.set_switches(switches);
    })
}
