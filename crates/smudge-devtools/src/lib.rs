//! Devtools that Smudge draws over a running app's window.
//!
//! The crate builds the devtools overlay: a panel on a root as large as the
//! window, meant to run as an app of its own drawn over the app a window
//! shows, so the app keeps working underneath and its own frames, numbers
//! and pixels stay its own.

mod access;
mod frames;
mod inspector;
mod live;
mod look;
mod marks;
mod reactive;
mod styles;
mod theme_file;
mod timeline;

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use marks::Marks;
pub use theme_file::{apply_theme_file, theme_file, watch_theme};
pub use timeline::FrameRecord;
use timeline::Timeline;

use smudge_core::app::Remounted;
use smudge_core::element::{AnyElement, Element, IntoElement, Slot, for_each, show};
use smudge_core::geometry::{Color, Point, Rect, Size};
use smudge_core::input::{Drag, Handled};
use smudge_core::inspect::{
    A11yWarning, BoxModel, CycleInfo, FocusInfo, NodeDetails, OwnerEntry, PointerInfo, TreeEntry,
};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, Edges, Justify, Length, Overflow, pct, px};
use smudge_core::theme::ColorValue;
use smudge_core::tree::{Dirty, WidgetId};
use smudge_core::widget::{Panel, panel};
use smudge_text::{LabelExt, label, provide_text_style};
use smudge_widgets::{column, row, spacer};

/// Where the panel opens, from the window's top left, in logical pixels.
const START: Point = Point { x: 16.0, y: 16.0 };

/// How many hot reload events the live tab shows.
const HOT_LINES: usize = 6;

/// One step of hot reload, as the window reports it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HotEvent {
    /// The patch builder started building a patch.
    Building,
    /// The patch could not be built, for this reason, such as the
    /// compiler's message.
    BuildFailed(String),
    /// A patch was applied in `took`, with `remounted` saying what the
    /// remount kept, or `None` when the tree was kept as it was.
    Applied {
        /// How long applying the patch and remounting took.
        took: Duration,
        /// What the remount kept, or `None` when it did not remount.
        remounted: Option<Remounted>,
    },
    /// The app refused the patch, for this reason.
    Refused(String),
}

impl HotEvent {
    /// Returns the color of the event's dot, a line that names it, and a
    /// line or a few of detail, which may be empty.
    fn parts(&self) -> (Color, String, String) {
        match self {
            HotEvent::Building => (look::WARN, String::from("Building a patch"), String::new()),
            HotEvent::BuildFailed(reason) => (
                look::BAD,
                String::from("Build failed"),
                String::from(reason.trim()),
            ),
            HotEvent::Refused(reason) => (look::BAD, String::from("Patch refused"), reason.clone()),
            HotEvent::Applied { took, remounted } => {
                let title = format!("Patch applied in {:.1} ms", took.as_secs_f64() * 1000.0);
                let detail = match remounted {
                    None => String::from("The tree was kept as it was."),
                    Some(remounted) => format!(
                        "Remounted, focus {}, {} {} kept their values, {} unclaimed",
                        if remounted.focus_restored {
                            "restored"
                        } else {
                            "not restored"
                        },
                        remounted.signals_kept,
                        if remounted.signals_kept == 1 {
                            "signal"
                        } else {
                            "signals"
                        },
                        remounted.signals_left,
                    ),
                };
                (look::GOOD, title, detail)
            }
        }
    }
}

/// The width of the panel, in logical pixels.
const WIDTH: f32 = 400.0;

/// The width of the panel's border, in logical pixels.
const BORDER: f32 = 1.0;

/// The padding at the sides of a tab, in logical pixels.
const PAD: f32 = 12.0;

/// The tallest the panel's body grows, in logical pixels.
const BODY_MAX: f32 = 540.0;

/// The shortest the panel's body shrinks in a small window, in logical
/// pixels.
const BODY_MIN: f32 = 240.0;

/// The height of the title bar, in logical pixels, which the body's height
/// leaves room for.
const TITLE: f32 = 42.0;

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
    /// What the reactive tab shows, once the window has traced the app.
    reactive: RwSignal<Option<Rc<reactive::Trace>>>,
    /// What the accessibility tab shows, once the window has audited the
    /// app.
    access: RwSignal<Option<Rc<access::Audit>>>,
    /// The last few hot reload events, newest first.
    hot: RwSignal<Rc<VecDeque<HotEvent>>>,
    /// The lines the last read of the watched theme file skipped.
    theme_skipped: RwSignal<Vec<String>>,
    /// What the last "Write theme" press wrote, or why it could not.
    theme_written: RwSignal<Option<String>>,
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
    /// Whether a hot patch is applied without remounting the root, so the
    /// tree and its state stay as they are. An edit to what the build
    /// creates does not show until a patch remounts, and a closure created
    /// after the first patch keeps that patch's code on the next one.
    pub keep_tree: bool,
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
    ///
    /// The lines that a read of the file [`watch_theme`] watches skips go to
    /// the panel of the newest `Tools` of the thread.
    pub fn new() -> Tools {
        let theme_skipped = RwSignal::new(Vec::new());
        theme_file::report_to(theme_skipped.clone());
        Tools {
            theme_skipped,
            theme_written: RwSignal::new(None),
            timeline: RwSignal::new(Rc::new(Timeline::default())),
            switches: RwSignal::new(Switches::default()),
            damage: RwSignal::new(Rc::new(Vec::new())),
            flashes: RwSignal::new(Rc::new(Vec::new())),
            bounds: RwSignal::new(Rc::new(Vec::new())),
            tab: RwSignal::new(Tab::Frames),
            reactive: RwSignal::new(None),
            access: RwSignal::new(None),
            hot: RwSignal::new(Rc::new(VecDeque::new())),
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
    /// bounds mark, the inspector's highlight and details, and the focus and
    /// pointer marks, and ends pick mode, as when the panel hides and no
    /// longer hears the app's frames.
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
        if self.details.with(Option::is_some) {
            self.show_details(None);
        }
    }

    /// Returns whether the inspector tab shows, so the window hands it the
    /// app's tree and the selected node's details.
    pub fn is_inspecting(&self) -> bool {
        self.tab.get() == Tab::Inspector
    }

    /// Records `event`, one step of hot reload, which the live tab shows
    /// among the last few. An event replaces the build it ends, so a
    /// finished build shows only how it ended.
    pub fn record_hot(&self, event: HotEvent) {
        self.hot.update(|events| {
            let events = Rc::make_mut(events);
            if events.front() == Some(&HotEvent::Building) {
                events.pop_front();
            }
            events.truncate(HOT_LINES - 1);
            events.push_front(event);
        });
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
        self.access
            .set(Some(Rc::new(access::Audit::new(nodes, warnings))));
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
            .set(Some(Rc::new(reactive::Trace::new(busiest, cycles, owners))));
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
/// The panel paints in colors of its own, the same over every app. Its tabs
/// each start with a line that says what the tab is for:
///
/// - Frames sums up what the app's frames cost, shows the last ones as a
///   timeline of stacked bars, one color per stage, with a dashed line at
///   the budget of a frame at 60 frames a second, and breaks the hovered
///   frame down by stage.
/// - Overlays turns the [`Switches`] on and off, each with a line on what
///   it draws and in which color. The root draws what they show over the
///   app, under the panel.
/// - Inspector picks a node, lists the tree, and shows the selected node's
///   source, box model, state, bindings and style.
/// - Reactive lists the busiest effects, the cycles and the owner tree.
/// - Access lists the accessibility warnings and tree, and a press on
///   either shows its node in the inspector.
/// - Live shows hot reload's last steps and the theme file.
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

    provide_text_style(look::text_style());
    let place = at.clone();
    let shown = open.clone();
    let caption = open.clone();
    let toggle = open.clone();
    let grabbed = Rc::clone(&grab);
    let title = row()
        .padding_xy(Size {
            width: PAD,
            height: 8.0,
        })
        .gap(look::gap(8.0))
        .align_items(Align::Center)
        .background(look::BAR)
        .on_pointer_down(move |point| grabbed.set(Some(point)))
        .child(look::dot(look::ACCENT))
        .child(look::heading("Devtools"))
        .child(frame_cost(tools))
        .child(spacer())
        .child(
            look::action(move || String::from(if caption.get() { "Hide" } else { "Show" }))
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
        .background(look::BACKGROUND)
        .border(BORDER)
        .border_color(look::LINE)
        .corner_radius(12.0)
        .overflow(Overflow::Clip)
        .child(title)
        .child(show(move || shown.get(), {
            let tools = tools.clone();
            let viewport = Rc::clone(&viewport);
            move || body(&tools, Rc::clone(&viewport))
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

/// Writes the theme [`watch_theme`] watches, in the theme file format, to
/// `target/smudge-devtools/theme.txt`, and says where, or why it could not.
fn write_theme() -> String {
    let Some((_, theme)) = theme_file::watched() else {
        return String::from("Call watch_theme to tune the theme from a file.");
    };
    let dir = std::path::Path::new("target").join("smudge-devtools");
    let path = dir.join("theme.txt");
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, theme.with(theme_file::theme_file)));
    match written {
        Ok(()) => format!("Wrote the theme to {}", path.display()),
        Err(error) => format!("Could not write the theme: {error}"),
    }
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
                .text_size(look::SMALL)
                .weight(smudge_text::FontWeight::BOLD)
                .padding_xy(Size {
                    width: 4.0,
                    height: 0.0,
                })
                .corner_radius(4.0)
                .background(marks::FOCUS)
                .text_color(look::ON_ACCENT)
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

/// Builds the chip in the title bar that shows what the app's frames cost
/// on average, in red once that is over the budget of a frame at 60 frames a
/// second.
fn frame_cost(tools: &Tools) -> Element<Panel> {
    let (text, color) = (tools.timeline.clone(), tools.timeline.clone());
    row()
        .padding_xy(Size {
            width: 6.0,
            height: 0.0,
        })
        .corner_radius(4.0)
        .background(look::SUNKEN)
        .child(
            look::mono(move || {
                let summary = text.with(|timeline| timeline.summary());
                if summary.frames == 0 {
                    String::from("app idle")
                } else {
                    format!("{:.1} ms a frame", summary.average.as_secs_f64() * 1000.0)
                }
            })
            .text_color(move || {
                let average = color.with(|timeline| timeline.summary().average);
                ColorValue::from(if average > timeline::BUDGET {
                    look::BAD
                } else {
                    look::MUTED
                })
            }),
        )
}

/// The tabs of the panel, in the order the tab bar shows them, with their
/// names and the line under the bar that says what each is for.
const TABS: [(Tab, &str, &str); 6] = [
    (
        Tab::Frames,
        "Frames",
        "What each frame of the app cost, stage by stage.",
    ),
    (
        Tab::Overlays,
        "Overlays",
        "Draw over the app to see what it repaints, lays out and focuses.",
    ),
    (
        Tab::Inspector,
        "Inspector",
        "Pick a node in the app, or in the tree, to see where it was built and \
         how it was laid out.",
    ),
    (
        Tab::Reactive,
        "Reactive",
        "Which effects run most, which updates loop, and what each owner holds.",
    ),
    (
        Tab::Access,
        "Access",
        "What assistive technology is told about the app, and what it cannot \
         describe.",
    ),
    (
        Tab::Live,
        "Live",
        "Patch the code and tune the theme while the app runs.",
    ),
];

/// Builds the panel's body: the tab bar, the line that says what the tab
/// is for, and the tab, as tall as the window leaves room for.
fn body(tools: &Tools, viewport: Rc<dyn Fn() -> Size>) -> Element<Panel> {
    let shown = |which: Tab, build: fn(&Tools) -> Element<Panel>| {
        let (tab, grown, tools) = (tools.tab.clone(), tools.tab.clone(), tools.clone());
        show(
            move || tab.get() == which,
            move || build(&tools).grow(1.0).min_height(px(0.0)),
        )
        .column()
        .min_height(px(0.0))
        .grow(move || if grown.get() == which { 1.0 } else { 0.0 })
    };
    let about = tools.tab.clone();
    column()
        .height(move || px((viewport().height - 2.0 * START.y - TITLE).clamp(BODY_MIN, BODY_MAX)))
        .border_edges(Edges {
            top: BORDER,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        })
        .border_color(look::LINE)
        .child(
            column()
                .padding_xy(Size {
                    width: PAD,
                    height: 10.0,
                })
                .gap(look::gap(8.0))
                .child(tab_bar(tools))
                .child(look::note(move || {
                    let tab = about.get();
                    TABS.iter()
                        .find(|(which, _, _)| *which == tab)
                        .map_or_else(String::new, |(_, _, about)| String::from(*about))
                })),
        )
        .child(shown(Tab::Frames, frames::frames_tab))
        .child(shown(Tab::Overlays, overlays_tab))
        .child(shown(Tab::Inspector, inspector::inspector))
        .child(shown(Tab::Reactive, reactive::reactive_tab))
        .child(shown(Tab::Access, access::access_tab))
        .child(shown(Tab::Live, live::live_tab))
}

/// Builds the tab bar: a segment per tab, the chosen one raised. Choosing
/// a tab other than the inspector ends pick mode.
fn tab_bar(tools: &Tools) -> Element<Panel> {
    let segment = |which: Tab, name: &'static str| {
        let (back, text, choose, picking) = (
            tools.tab.clone(),
            tools.tab.clone(),
            tools.tab.clone(),
            tools.picking.clone(),
        );
        row()
            .grow(1.0)
            .justify_content(Justify::Center)
            .padding_xy(Size {
                width: 6.0,
                height: 3.0,
            })
            .corner_radius(6.0)
            .border(1.0)
            .border_color(Color::TRANSPARENT)
            .background(move || {
                ColorValue::from(if back.get() == which {
                    look::HOVER
                } else {
                    Color::TRANSPARENT
                })
            })
            .hover(|p| p.background(look::RAISED))
            .focused(|p| p.border_color(look::ACCENT))
            .role(Role::Button)
            .accessible_label(name)
            .focusable()
            .activate_on_keys()
            .on_press(move || {
                choose.set(which);
                if which != Tab::Inspector {
                    picking.set(false);
                }
            })
            .child(label(name).text_color(move || {
                ColorValue::from(if text.get() == which {
                    look::TEXT
                } else {
                    look::MUTED
                })
            }))
    };
    TABS.iter().fold(
        row()
            .padding(px(3.0))
            .gap(look::gap(2.0))
            .background(look::SUNKEN)
            .corner_radius(8.0),
        |bar, &(which, name, _)| bar.child(segment(which, name)),
    )
}

/// The tabs of the panel.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tab {
    /// The frame timeline and what each stage cost.
    Frames,
    /// The switches of what the overlay draws over the app.
    Overlays,
    /// The node picker, the tree and a node's details.
    Inspector,
    /// The busiest effects, the cycles and the owner tree.
    Reactive,
    /// The accessibility tree and its warnings.
    Access,
    /// Hot reload and the theme file.
    Live,
}

/// Builds the overlays tab: a switch for each mark the overlay draws over
/// the app, in the color it draws in.
fn overlays_tab(tools: &Tools) -> Element<Panel> {
    look::scrolling(
        look::tab_content()
            .gap(look::gap(2.0))
            .child(switch(
                tools,
                "Damage tint",
                "Tints what the last frame repainted. A still app shows none.",
                Some(marks::TINT),
                |switches| &mut switches.damage_tint,
            ))
            .child(switch(
                tools,
                "Paint flashing",
                "Flashes each repaint and fades it out, so a burst stays visible.",
                Some(marks::FLASH),
                |switches| &mut switches.paint_flashing,
            ))
            .child(switch(
                tools,
                "Layout bounds",
                "Outlines every node, colored by its depth in the tree.",
                Some(marks::DEPTHS[0]),
                |switches| &mut switches.layout_bounds,
            ))
            .child(switch(
                tools,
                "Focus",
                "Numbers the Tab order and outlines the focused node and its trap.",
                Some(marks::FOCUS),
                |switches| &mut switches.focus,
            ))
            .child(switch(
                tools,
                "Hit testing",
                "Outlines what takes a press, and what the pointer is over.",
                Some(marks::PRESSABLE),
                |switches| &mut switches.hit_testing,
            ))
            .child(
                look::note("The overlay draws under this panel and never takes the app's input.")
                    .text_color(look::FAINT)
                    .margin(px(8.0)),
            )
            .child(
                look::section(
                    "Rendering",
                    "Change how the app repaints, and watch the frames tab.",
                )
                .child(switch(
                    tools,
                    "Full repaint",
                    "Repaint the whole window every frame instead of only what changed, \
                         to compare what the two cost.",
                    None,
                    |switches| &mut switches.full_repaint,
                )),
            ),
    )
}

/// Builds the row that turns the switch `field` picks on and off, named
/// `name`, with `about` saying what it does and `swatch` the color it draws
/// in.
fn switch(
    tools: &Tools,
    name: &'static str,
    about: &'static str,
    swatch: Option<Color>,
    field: fn(&mut Switches) -> &mut bool,
) -> Element<Panel> {
    let read = tools.switches.clone();
    let write = tools.clone();
    look::switch_row(
        name,
        about,
        swatch,
        move || {
            let mut switches = read.get();
            *field(&mut switches)
        },
        move || {
            let mut switches = write.switches();
            let on = field(&mut switches);
            *on = !*on;
            write.set_switches(switches);
        },
    )
}
