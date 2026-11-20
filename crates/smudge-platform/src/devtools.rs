//! The devtools overlay a window draws over its app, with the `_devtools`
//! feature, which the `devtools` feature of `smudge` turns on.
//!
//! The overlay is an [`App`] of its own, which builds the panel of
//! `smudge-devtools`. It runs on the window's thread with its own root
//! owner, so a remount of the app never touches it, and the two trees never
//! wake each other. The window sends each input event to one of the two, as
//! [`Devtools::dispatch`] decides, and draws the overlay's frame over the
//! app's without letting it reach the app's own frames or numbers.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::app::{App, AppConfig, FrameOutput, RepaintMode};
use smudge_core::geometry::{Color, PixelRect, Point, Rect, Size};
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::inspect::{A11yWarning, BoxModel, FocusInfo, NodeDetails, PointerInfo, TreeEntry};
use smudge_core::paint::{DisplayItem, DisplayList};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::SemanticsNode;
use smudge_core::theme::Theme;
use smudge_core::window::Cursor;
use smudge_cpu::CpuRenderer;

use std::collections::HashMap;
use std::fmt::Write;
use std::path::PathBuf;

use web_time::Instant;

use crate::cover::Cover;

/// The environment variable that opens the panel when the app starts, when
/// it is `1`.
pub(crate) const OPEN_VARIABLE: &str = "SMUDGE_DEVTOOLS";

/// The environment variable that names the key that shows and hides the
/// panel, such as `ctrl+shift+d`.
pub(crate) const KEY_VARIABLE: &str = "SMUDGE_DEVTOOLS_KEY";

/// How long the reactive tab waits between two samples of the app's
/// effects.
const SAMPLE: Duration = Duration::from_secs(1);

/// How many places the busiest effects list shows.
const BUSIEST: usize = 10;

/// A place in the source.
type Site = &'static std::panic::Location<'static>;

/// How long a flash takes to fade out.
const FADE: Duration = Duration::from_millis(500);

/// The opacity a flash starts at, out of 255.
const FLASH_OPACITY: f32 = 160.0;

/// The most flashes kept at once, so a burst of frames with scattered
/// damage stays cheap to draw. The oldest go first.
const FLASH_CAP: usize = 512;

/// Returns the opacity, out of 255, of a flash `age` old: its starting
/// opacity falling evenly to nothing over [`FADE`].
fn opacity(age: Duration) -> u8 {
    let left = 1.0 - age.as_secs_f32() / FADE.as_secs_f32();
    (FLASH_OPACITY * left).round().clamp(0.0, 255.0) as u8
}

/// The devtools of one window: the overlay's app, whether it shows, and
/// where input goes.
pub(crate) struct Devtools {
    /// The overlay's own app, which builds the devtools panel.
    pub(crate) app: App,
    /// The window's size in logical pixels, which the panel stays inside.
    viewport: RwSignal<Size>,
    /// The window's physical pixels per logical pixel.
    scale: f32,
    /// Whether the app repaints its whole surface on every frame, as the
    /// devtools last set it.
    full_repaint: bool,
    /// The damage the tint covers, each pixel once.
    cover: Cover,
    /// The rectangles the app's recent frames repainted, in logical pixels,
    /// each with when it was repainted, oldest first.
    flashes: Vec<(Rect, Instant)>,
    /// The rectangles of the app's nodes last handed to the panel, each
    /// with its depth.
    bounds: Vec<(usize, Rect)>,
    /// The rectangles of the app's nodes as this redraw finds them.
    found: Vec<(usize, Rect)>,
    /// The app's tree last handed to the inspector.
    entries: Vec<TreeEntry>,
    /// The app's tree as this redraw finds it.
    found_entries: Vec<TreeEntry>,
    /// The details of the selected node last handed to the inspector.
    details: Option<NodeDetails>,
    /// The boxes last handed to the overlay to highlight.
    highlighted: Option<BoxModel>,
    /// What the panel showed at the last refresh: the tab, the selection
    /// and the switches its views follow.
    view: Option<View>,
    /// Whether a primary press that picked a node is held, so its release
    /// reaches neither side.
    pick_held: bool,
    /// The directory "Copy as test" writes into.
    pub(crate) copy_dir: PathBuf,
    /// The focus state last handed to the overlay.
    focus_shown: Option<FocusInfo>,
    /// The pointer state last handed to the overlay.
    pointer_shown: Option<PointerInfo>,
    /// The accessibility tree and warnings last handed to the tab.
    access_shown: Option<(Vec<SemanticsNode>, Vec<A11yWarning>)>,
    /// When the reactive tab last sampled the app's effects, and how many
    /// times the effects created at each place had run by then.
    sampled: Option<(Instant, HashMap<Option<Site>, u64>)>,
    /// What the panel shows, which the window feeds.
    tools: smudge_devtools::Tools,
    /// Whether a wake of the overlay requests no redraw, because the window
    /// draws the next frame on its own.
    quiet: Rc<Cell<bool>>,
    /// Whether the panel shows.
    open: bool,
    /// Whether a primary press began over the panel, so the pointer stays
    /// with the overlay until the press ends.
    captured: bool,
    /// Whether a primary press began on the app, so the pointer stays with
    /// the app until the press ends, over the panel too.
    app_pressed: bool,
    /// Whether the pointer is over the panel, as the last move found.
    over: bool,
    /// Whether keys go to the overlay, after a press on the panel and until
    /// a press on the app.
    keys: bool,
    /// Whether the window holds the input focus of the operating system,
    /// which the side with the keys hears.
    window_focused: bool,
    /// The key that shows and hides the panel.
    toggle: Shortcut,
    /// How long the overlay's last presented frame took.
    pub(crate) timing: OverlayTiming,
}

/// The overlay's own time in one present, which the app's frame statistics
/// and present statistics leave out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OverlayTiming {
    /// How long the overlay's app took to run its frame.
    pub(crate) frame: Duration,
    /// How long drawing the overlay and compositing it over the app took.
    /// The GPU renderer draws both lists in one render, so there it is zero
    /// and the overlay's drawing counts in the frame's raster time.
    pub(crate) draw: Duration,
}

impl Devtools {
    /// Creates the devtools of a window whose app shows `viewport` logical
    /// pixels at `scale`, open when [`OPEN_VARIABLE`] is `1`, with the
    /// toggle key [`KEY_VARIABLE`] names.
    pub(crate) fn new(viewport: Size, scale: f32) -> Devtools {
        let open = std::env::var(OPEN_VARIABLE).is_ok_and(|value| value == "1");
        let toggle = std::env::var(KEY_VARIABLE)
            .ok()
            .and_then(|value| Shortcut::parse(&value))
            .unwrap_or_default();
        Devtools::with(viewport, scale, open, toggle)
    }

    /// Creates the devtools with the panel `open` or not and the `toggle`
    /// key, whatever the environment says.
    pub(crate) fn with(viewport: Size, scale: f32, open: bool, toggle: Shortcut) -> Devtools {
        let size = RwSignal::new(viewport);
        let read = size.clone();
        let tools = smudge_devtools::Tools::new();
        let shown = tools.clone();
        let mut app = App::new(
            AppConfig {
                viewport,
                scale,
                clear_color: Color::TRANSPARENT,
                repaint: RepaintMode::Damage,
            },
            move || smudge_devtools::overlay(&shown, move || read.get()),
        );
        app.set_theme(Theme::dark());
        // Keys start with the app, so the overlay starts unfocused.
        app.handle_event(InputEvent::WindowFocus(false));
        Devtools {
            app,
            viewport: size,
            scale,
            full_repaint: false,
            cover: Cover::default(),
            flashes: Vec::new(),
            bounds: Vec::new(),
            found: Vec::new(),
            entries: Vec::new(),
            found_entries: Vec::new(),
            details: None,
            highlighted: None,
            view: None,
            pick_held: false,
            copy_dir: PathBuf::from("target").join("smudge-devtools"),
            focus_shown: None,
            pointer_shown: None,
            access_shown: None,
            sampled: None,
            tools,
            quiet: Rc::new(Cell::new(false)),
            open,
            captured: false,
            app_pressed: false,
            over: false,
            keys: false,
            window_focused: true,
            toggle,
            timing: OverlayTiming::default(),
        }
    }

    /// Sets `wake` as what a wake of the overlay runs to request a redraw.
    pub(crate) fn set_wake(&self, wake: impl Fn() + 'static) {
        let quiet = Rc::clone(&self.quiet);
        self.app.set_wake_callback(move || {
            if !quiet.get() {
                wake();
            }
        });
    }

    /// Records one frame of the app for the timeline.
    ///
    /// With `paced`, the window draws its next frame on its own, so the
    /// recording requests no redraw and its bar shows with that frame. A
    /// redraw requested here would come before the window's pacing allows
    /// the next frame, and an app that keeps itself awake would then draw
    /// as fast as the window system lets it.
    pub(crate) fn record(&self, record: smudge_devtools::FrameRecord, paced: bool) {
        self.quiet.set(paced);
        self.tools.record(record);
        self.quiet.set(false);
    }

    /// Takes the window's new size, `viewport` logical pixels at `scale`,
    /// which the panel stays inside.
    pub(crate) fn set_viewport(&mut self, viewport: Size, scale: f32) {
        self.app.set_viewport(viewport, scale);
        self.viewport.set(viewport);
        self.scale = scale;
    }

    /// Takes `mode`, how much of its surface the app was created to repaint,
    /// as where the full repaint switch starts.
    pub(crate) fn start_repaint(&mut self, mode: RepaintMode) {
        self.full_repaint = mode == RepaintMode::Full;
        let mut switches = self.tools.switches();
        switches.full_repaint = self.full_repaint;
        self.tools.set_switches(switches);
    }

    /// Makes `app` repaint as the full repaint switch says, once it changed.
    pub(crate) fn apply_switches(&mut self, app: &mut App) {
        let full = self.tools.switches().full_repaint;
        if full != self.full_repaint {
            self.full_repaint = full;
            app.set_repaint_mode(if full {
                RepaintMode::Full
            } else {
                RepaintMode::Damage
            });
        }
    }

    /// Takes `damage`, the rectangles in physical pixels that the app's
    /// frame repainted at `now`, which the damage tint covers while it is
    /// on, and which flash from `now` while paint flashing is on.
    ///
    /// The overlay's frame runs next in the same redraw, so the marks show
    /// in the present of the frame they mark, and this requests no redraw.
    pub(crate) fn show_damage(&mut self, damage: &[PixelRect], now: Instant) {
        let switches = self.tools.switches();
        if !switches.damage_tint && !switches.paint_flashing {
            return;
        }
        let scale = self.scale;
        let logical = |rect: &PixelRect| {
            Rect::new(
                rect.x as f32 / scale,
                rect.y as f32 / scale,
                (rect.x + rect.width) as f32 / scale,
                (rect.y + rect.height) as f32 / scale,
            )
        };
        let covered = self.cover.of(damage);
        if switches.paint_flashing {
            self.flashes
                .extend(covered.iter().map(|rect| (logical(rect), now)));
            let excess = self.flashes.len().saturating_sub(FLASH_CAP);
            self.flashes.drain(..excess);
        }
        if switches.damage_tint {
            self.quiet.set(true);
            self.tools.show_damage(covered.iter().map(logical));
            self.quiet.set(false);
        }
    }

    /// Fades the flashes to `now`, dropping those older than [`FADE`], so
    /// the overlay's next frame shows each at the opacity its age gives.
    ///
    /// The overlay's frame runs next in the same redraw, so this requests
    /// no redraw. While flashes remain, [`Devtools::is_fading`] keeps the
    /// window drawing frames.
    pub(crate) fn fade(&mut self, now: Instant) {
        if !self.tools.switches().paint_flashing {
            self.flashes.clear();
        }
        if self.flashes.is_empty() {
            return;
        }
        self.flashes
            .retain(|&(_, at)| now.saturating_duration_since(at) < FADE);
        self.quiet.set(true);
        self.tools.show_flashes(
            self.flashes
                .iter()
                .map(|&(rect, at)| (rect, opacity(now.saturating_duration_since(at)))),
        );
        self.quiet.set(false);
    }

    /// Hands the panel the rectangles of `app`'s nodes while layout bounds
    /// is on, when they differ from the ones it holds. A rectangle with an
    /// edge that is not finite is left out.
    ///
    /// A node moves with a layout pass and with a scroll, so this walks the
    /// tree after every present and writes only when something moved. The
    /// outlines show in the overlay's next frame: with `paced`, the window
    /// draws it on its own, as [`Devtools::record`] describes, and otherwise
    /// the write requests it. That frame finds nothing moved and writes
    /// nothing, so the window then goes idle.
    pub(crate) fn show_bounds(&mut self, app: &App, paced: bool) {
        if !self.tools.switches().layout_bounds {
            self.bounds.clear();
            return;
        }
        self.found.clear();
        // A rectangle that is not finite cannot be outlined, and one holding
        // a NaN never equals itself, which would rewrite it on every redraw.
        app.visit_nodes(|depth, _, rect| {
            if [rect.x0, rect.y0, rect.x1, rect.y1]
                .iter()
                .all(|edge| edge.is_finite())
            {
                self.found.push((depth, rect));
            }
        });
        if self.found == self.bounds {
            return;
        }
        std::mem::swap(&mut self.found, &mut self.bounds);
        self.quiet.set(paced);
        self.tools.show_bounds(self.bounds.iter().copied());
        self.quiet.set(false);
    }

    /// Handles `event` in pick mode, and returns whether it did.
    ///
    /// While pick mode is on and no press is held, the pointer outside the
    /// panel picks instead of reaching the app: a move highlights the node
    /// under it, and a primary press selects that node and ends pick mode,
    /// and its release reaches neither side.
    /// Over the panel, the pointer goes to the panel as usual.
    fn pick(&mut self, app: &mut App, event: &InputEvent) -> bool {
        if !self.tools.is_picking() || self.captured || self.app_pressed {
            return false;
        }
        let position = match event {
            InputEvent::PointerMoved { position }
            | InputEvent::PointerDown { position, .. }
            | InputEvent::PointerUp { position, .. } => *position,
            _ => return false,
        };
        if self.over_panel(position) {
            return false;
        }
        if self.over {
            self.app.handle_event(InputEvent::PointerLeft);
            self.over = false;
        }
        app.handle_event(InputEvent::PointerLeft);
        let node = app.node_at(position);
        match event {
            InputEvent::PointerMoved { .. } => {
                self.highlighted = node
                    .and_then(|id| app.node_details(id))
                    .map(|details| details.boxes);
                self.tools.show_highlight(self.highlighted);
            }
            InputEvent::PointerDown {
                button: PointerButton::Primary,
                ..
            } => {
                self.tools.select(node);
                self.tools.stop_picking();
                self.pick_held = true;
            }
            _ => {}
        }
        true
    }

    /// Hands the inspector the app's tree and the selected node's details,
    /// and the overlay the selected node's boxes, while the inspector tab
    /// shows, each only when it changed.
    ///
    /// Like [`Devtools::show_bounds`], it runs after every present, and with
    /// `paced` its writes request no redraw. It looks at the app only with
    /// `refresh`, after a frame the app wanted or a change of what the panel
    /// shows: values that hold a NaN never compare equal, so comparing on
    /// every redraw would write, and wake the overlay, on every redraw.
    pub(crate) fn inspect(&mut self, app: &App, paced: bool, refresh: bool) {
        if !self.tools.is_inspecting() {
            self.entries.clear();
            self.quiet.set(paced);
            if self.details.take().is_some() {
                self.tools.show_details(None);
            }
            if self.highlighted.take().is_some() {
                self.tools.show_highlight(None);
            }
            self.quiet.set(false);
            return;
        }
        if !refresh {
            if self.tools.take_copy_request() {
                self.copy(app);
            }
            return;
        }
        self.quiet.set(paced);
        self.found_entries.clear();
        app.visit_tree(|entry| self.found_entries.push(entry));
        if self.found_entries != self.entries {
            std::mem::swap(&mut self.found_entries, &mut self.entries);
            self.tools.show_tree(self.entries.iter().copied());
        }
        let details = self.tools.selected().and_then(|id| app.node_details(id));
        if details != self.details {
            self.details.clone_from(&details);
            self.tools.show_details(details);
        }
        if self.tools.take_copy_request() {
            self.copy(app);
        }
        if !self.tools.is_picking() {
            let boxes = self.details.as_ref().map(|details| details.boxes);
            if boxes != self.highlighted {
                self.highlighted = boxes;
                self.tools.show_highlight(boxes);
            }
        }
        self.quiet.set(false);
    }

    /// Runs "Copy as test" and shows what it wrote or why it failed.
    fn copy(&self, app: &App) {
        let message = match self.copy_test(app) {
            Ok((dump, png)) => format!("Wrote {} and {}", dump.display(), png.display()),
            Err(error) => format!("Could not copy: {error}"),
        };
        self.tools.show_copied(message);
    }

    /// Returns whether the tab, the selection or a switch the panel's views
    /// follow changed since the last call, so they must refresh even when
    /// the app ran no frame.
    pub(crate) fn view_changed(&mut self) -> bool {
        let switches = self.tools.switches();
        let view = View {
            inspecting: self.tools.is_inspecting(),
            auditing: self.tools.is_auditing(),
            picking: self.tools.is_picking(),
            selected: self.tools.selected(),
            focus: switches.focus,
            hit_testing: switches.hit_testing,
        };
        self.view.replace(view) != Some(view)
    }

    /// Writes the selected node's subtree as a test's expectations, into
    /// [`Devtools::copy_dir`], and returns the two files it wrote.
    ///
    /// The text file holds the subtree in the format of `smudge-test`'s
    /// `layout_dump`, with the rectangles measured from the node's top left
    /// corner, and the image is the node's rectangle of the app's last frame
    /// drawn at a scale of 1, so it also shows what ancestors paint behind
    /// the node and what overlaps it. A harness at a scale of 1, as large as
    /// the node and built from the same element, dumps the same text, and
    /// draws the same image when nothing behind or over the node paints.
    ///
    /// # Errors
    ///
    /// Returns an error when no node is selected, and when the directory or
    /// a file cannot be written.
    fn copy_test(&self, app: &App) -> std::io::Result<(PathBuf, PathBuf)> {
        let details = self
            .details
            .as_ref()
            .ok_or_else(|| std::io::Error::other("select a node first"))?;
        let start = self
            .entries
            .iter()
            .position(|entry| entry.id == details.id)
            .ok_or_else(|| std::io::Error::other("the node is gone"))?;
        let origin = details.boxes.border;
        let depth = self.entries[start].depth;
        let mut dump = String::new();
        let subtree = std::iter::once(&self.entries[start]).chain(
            self.entries[start + 1..]
                .iter()
                .take_while(|entry| entry.depth > depth),
        );
        for entry in subtree.filter(|entry| !entry.hidden) {
            let name = short_name(entry.type_name);
            let rect = entry.rect;
            let _ = writeln!(
                dump,
                "{:indent$}{name} [{}, {}, {}, {}]",
                "",
                rect.x0 - origin.x0,
                rect.y0 - origin.y0,
                rect.x1 - origin.x0,
                rect.y1 - origin.y0,
                indent = (entry.depth - depth) * 2
            );
        }

        let mut list = DisplayList::new();
        list.push_offset(Point {
            x: -origin.x0,
            y: -origin.y0,
        });
        for item in app.display_list().items() {
            match item {
                DisplayItem::Op(op) => list.push_op(op.clone()),
                DisplayItem::PushClip(rect) => list.push_clip(*rect),
                DisplayItem::PopClip => list.pop_clip(),
                DisplayItem::PushOffset(offset) => list.push_offset(*offset),
                DisplayItem::PopOffset => list.pop_offset(),
                DisplayItem::PushGroup(opacity) => list.push_group(*opacity),
                DisplayItem::PopGroup => list.pop_group(),
                _ => {}
            }
        }
        list.pop_offset();
        list.finish();
        let (width, height) = (
            (origin.width().ceil() as u32).max(1),
            (origin.height().ceil() as u32).max(1),
        );
        let mut renderer = CpuRenderer::new(width, height);
        renderer.render(&FrameOutput {
            list: &list,
            damage: &[PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            }],
            full: true,
            clear_color: app.clear_color(),
            scale: 1.0,
        });

        std::fs::create_dir_all(&self.copy_dir)?;
        let name = short_name(details.type_name).to_lowercase();
        let (dump_path, png_path) = (1..)
            .map(|index| {
                (
                    self.copy_dir.join(format!("{name}-{index}.txt")),
                    self.copy_dir.join(format!("{name}-{index}.png")),
                )
            })
            .find(|(dump, png)| !dump.exists() && !png.exists())
            .expect("an unused name is always found");
        std::fs::write(&dump_path, dump)?;
        std::fs::write(&png_path, renderer.encode_png())?;
        Ok((dump_path, png_path))
    }

    /// Hands the overlay the app's focus and pointer state while their
    /// switches are on, and the accessibility tab the app's tree and
    /// warnings while it shows, each only when it changed.
    ///
    /// Like [`Devtools::inspect`], it looks at the app only with `refresh`.
    pub(crate) fn observe(&mut self, app: &App, paced: bool, refresh: bool) {
        if !refresh {
            return;
        }
        let switches = self.tools.switches();
        self.quiet.set(paced);
        let focus = switches.focus.then(|| app.focus_info());
        if focus != self.focus_shown {
            self.focus_shown.clone_from(&focus);
            self.tools.show_focus(focus);
        }
        let pointer = switches.hit_testing.then(|| app.pointer_info());
        if pointer != self.pointer_shown {
            self.pointer_shown.clone_from(&pointer);
            self.tools.show_pointer(pointer);
        }
        if self.tools.is_auditing() {
            let access = (app.semantics_nodes(), app.accessibility_warnings());
            if self.access_shown.as_ref() != Some(&access) {
                self.tools.show_access(&access.0, &access.1);
                self.access_shown = Some(access);
            }
        } else {
            self.access_shown = None;
        }
        self.quiet.set(false);
    }

    /// Hands the reactive tab the app's busiest effects, its cycles and its
    /// owner tree at `now`, while the tab shows, at most once a second.
    ///
    /// The busiest effects are the places effects were created, with how
    /// many times a second the effects created there ran since the last
    /// sample, busiest first. The first sample has no rate yet. Like
    /// [`Devtools::show_bounds`], it runs after every present, and with
    /// `paced` its write requests no redraw.
    pub(crate) fn trace(&mut self, app: &App, now: Instant, paced: bool) {
        if !self.tools.is_tracing() {
            self.sampled = None;
            return;
        }
        if let Some((at, _)) = &self.sampled
            && now.saturating_duration_since(*at) < SAMPLE
        {
            return;
        }
        let mut totals: HashMap<Option<Site>, u64> = HashMap::new();
        for effect in app.busiest_effects(usize::MAX) {
            *totals.entry(effect.defined_at).or_default() += effect.runs;
        }
        let mut busiest = Vec::new();
        if let Some((then, before)) = &self.sampled {
            let seconds = now.saturating_duration_since(*then).as_secs_f64();
            busiest = totals
                .iter()
                .filter_map(|(place, runs)| {
                    let ran = runs.saturating_sub(before.get(place).copied().unwrap_or(0));
                    (ran > 0).then(|| (*place, ran as f64 / seconds))
                })
                .collect();
            busiest.sort_by(|a, b| b.1.total_cmp(&a.1));
            busiest.truncate(BUSIEST);
        }
        let mut owners = Vec::new();
        app.visit_owners(|owner| owners.push(owner));
        self.quiet.set(paced);
        self.tools.show_reactive(&busiest, &app.cycles(), &owners);
        self.quiet.set(false);
        self.sampled = Some((now, totals));
    }

    /// Returns whether hot patches apply without remounting the root.
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    pub(crate) fn keeps_tree(&self) -> bool {
        self.tools.switches().keep_tree
    }

    /// Records one step of hot reload for the live tab.
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    pub(crate) fn record_hot(&self, event: smudge_devtools::HotEvent) {
        self.tools.record_hot(event);
    }

    /// Returns whether flashes are still fading, so the window keeps
    /// drawing frames.
    pub(crate) fn is_fading(&self) -> bool {
        self.open && !self.flashes.is_empty()
    }

    /// Returns whether the panel shows.
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Hands `event` to the overlay or to `app`, and returns whether it
    /// showed or hid the panel.
    ///
    /// A press of the toggle key, not a repeat, shows and hides the panel,
    /// and neither app hears the key. While the panel shows:
    /// - the pointer goes to the overlay while it is over the panel, and
    ///   stays with the side a primary press began on until the press ends;
    ///   the side the pointer leaves hears it leave;
    /// - the wheel goes where the pointer is;
    /// - keys and the input method go to the side of the last press, and
    ///   that side alone counts the window as focused, so the other shows no
    ///   caret or focus ring;
    /// - the modifiers reach both.
    ///
    /// While it is hidden, everything goes to the app.
    pub(crate) fn dispatch(&mut self, app: &mut App, event: InputEvent) -> bool {
        if let InputEvent::Key(key) = &event
            && self.toggle.matches(key)
        {
            if key.pressed && !key.repeat {
                self.show(app, !self.open);
                return true;
            }
            return false;
        }
        if !self.open {
            match &event {
                InputEvent::PointerDown {
                    button: PointerButton::Primary,
                    ..
                } => self.app_pressed = true,
                InputEvent::PointerUp {
                    button: PointerButton::Primary,
                    ..
                } => self.app_pressed = false,
                InputEvent::WindowFocus(focused) => self.window_focused = *focused,
                _ => {}
            }
            app.handle_event(event);
            return false;
        }
        if self.pick_held
            && let InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } = event
        {
            self.pick_held = false;
            return false;
        }
        if self.pick(app, &event) {
            return false;
        }
        match event {
            InputEvent::PointerMoved { position } => {
                let over = self.takes_pointer(position);
                if over != self.over {
                    if over {
                        app.handle_event(InputEvent::PointerLeft);
                    } else {
                        self.app.handle_event(InputEvent::PointerLeft);
                    }
                    self.over = over;
                }
                self.target(app, over).handle_event(event);
            }
            InputEvent::PointerLeft => {
                let over = self.over;
                self.target(app, over).handle_event(event);
                self.over = false;
            }
            InputEvent::PointerDown { position, button } => {
                let over = self.takes_pointer(position);
                if button == PointerButton::Primary {
                    self.captured |= over;
                    self.app_pressed |= !over;
                }
                self.focus_keys(app, over);
                self.target(app, over).handle_event(event);
            }
            InputEvent::PointerUp { button, .. } => {
                let over = self.captured || (!self.app_pressed && self.over);
                if button == PointerButton::Primary {
                    self.captured = false;
                    self.app_pressed = false;
                }
                self.target(app, over).handle_event(event);
            }
            InputEvent::Wheel { .. } => {
                let over = self.over;
                self.target(app, over).handle_event(event);
            }
            InputEvent::Key(_) | InputEvent::ImePreedit { .. } | InputEvent::ImeCommit { .. } => {
                let keys = self.keys;
                self.target(app, keys).handle_event(event);
            }
            InputEvent::WindowFocus(focused) => {
                self.window_focused = focused;
                let keys = self.keys;
                self.target(app, keys).handle_event(event);
                self.target(app, !keys)
                    .handle_event(InputEvent::WindowFocus(false));
            }
            InputEvent::ModifiersChanged(_) => {
                self.app.handle_event(event.clone());
                app.handle_event(event);
            }
        }
        false
    }

    /// Shows or hides the panel. A press held on the panel ends there, the
    /// pointer leaves it, the keys go back to `app`, and the damage tint
    /// forgets the frames the hidden panel no longer hears.
    fn show(&mut self, app: &mut App, open: bool) {
        if !open {
            if self.captured {
                let position = self.app.pointer_position();
                self.app.handle_event(InputEvent::PointerUp {
                    position,
                    button: PointerButton::Primary,
                });
            }
            if self.over {
                self.app.handle_event(InputEvent::PointerLeft);
            }
            self.focus_keys(app, false);
            self.tools.forget_marks();
            self.flashes.clear();
            self.bounds.clear();
            self.entries.clear();
            self.details = None;
            self.highlighted = None;
            self.focus_shown = None;
            self.pointer_shown = None;
            self.access_shown = None;
        }
        self.open = open;
        self.captured = false;
        self.over = false;
    }

    /// Returns whether the pointer at `position` goes to the overlay: a press
    /// that began on it holds it, one that began on the app keeps it, and
    /// otherwise it goes where the panel is.
    fn takes_pointer(&self, position: Point) -> bool {
        !self.app_pressed && (self.captured || self.over_panel(position))
    }

    /// Sends the keys to the overlay when `overlay` holds, and to `app`
    /// otherwise, and tells the side that loses them that the window lost
    /// its focus, and the side that gains them whether the window has it.
    fn focus_keys(&mut self, app: &mut App, overlay: bool) {
        if overlay == self.keys {
            return;
        }
        self.keys = overlay;
        let focused = self.window_focused;
        self.target(app, !overlay)
            .handle_event(InputEvent::WindowFocus(false));
        self.target(app, overlay)
            .handle_event(InputEvent::WindowFocus(focused));
    }

    /// Returns the cursor the window shows: the overlay's while the pointer
    /// is over the panel, and `app`'s otherwise.
    pub(crate) fn cursor(&self, app: &App) -> Cursor {
        if self.open && self.over {
            self.app.cursor()
        } else {
            app.cursor()
        }
    }

    /// Returns the area the input method anchors to: the overlay's while
    /// the keys go to it, and `app`'s otherwise, so composed text reaches
    /// the side that hears the keys.
    pub(crate) fn ime_area(&self, app: &App) -> Option<Rect> {
        if self.open && self.keys {
            self.app.ime_area()
        } else {
            app.ime_area()
        }
    }

    /// Returns the overlay when `overlay` holds, and `app` otherwise.
    fn target<'a>(&'a mut self, app: &'a mut App, overlay: bool) -> &'a mut App {
        if overlay { &mut self.app } else { app }
    }

    /// Returns whether `position` lies over the panel: on a node of the
    /// overlay below its root, which covers the window and takes nothing.
    fn over_panel(&self, position: Point) -> bool {
        // The panel is the root's last child, and it clips what it holds, so
        // its own rectangle is all it covers. The children before it, such
        // as the tab order numbers, take no pointer input.
        let mut panel = None;
        self.app.visit_nodes(|depth, _, rect| {
            if depth == 1 {
                panel = Some(rect);
            }
        });
        panel.is_some_and(|panel| panel.contains(position))
    }
}

/// A key with the modifiers that must be held with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shortcut {
    /// The character the key types, in lower case.
    character: String,
    /// The modifiers held with it, exactly.
    modifiers: Modifiers,
}

impl Default for Shortcut {
    /// Control, Shift and D, or Command, Shift and D on macOS, since a
    /// browser takes F12 for its own devtools.
    fn default() -> Self {
        Shortcut {
            character: String::from("d"),
            modifiers: Modifiers {
                shift: true,
                control: !cfg!(target_os = "macos"),
                meta: cfg!(target_os = "macos"),
                alt: false,
            },
        }
    }
}

impl Shortcut {
    /// Reads a key such as `ctrl+shift+d`: modifiers named `ctrl` or
    /// `control`, `shift`, `alt` or `option`, and `cmd`, `meta` or `super`,
    /// then one character, joined by `+` in any case. Anything else names no
    /// key, and `None` is returned.
    pub(crate) fn parse(text: &str) -> Option<Shortcut> {
        let mut modifiers = Modifiers::default();
        let mut character = None;
        for part in text.split('+') {
            let part = part.trim().to_lowercase();
            match part.as_str() {
                "ctrl" | "control" => modifiers.control = true,
                "shift" => modifiers.shift = true,
                "alt" | "option" => modifiers.alt = true,
                "cmd" | "meta" | "super" => modifiers.meta = true,
                _ if character.is_none() && part.chars().count() == 1 => character = Some(part),
                _ => return None,
            }
        }
        Some(Shortcut {
            character: character?,
            modifiers,
        })
    }

    /// Returns whether `key` is this shortcut, pressed or released.
    fn matches(&self, key: &KeyEvent) -> bool {
        let Key::Character(character) = &key.key else {
            return false;
        };
        character.to_lowercase() == self.character && key.modifiers == self.modifiers
    }
}

#[cfg(test)]
#[path = "devtools_tests.rs"]
mod tests;

/// Returns the name of a widget type without its path or generic arguments.
fn short_name(type_name: &str) -> &str {
    let path = type_name.split('<').next().unwrap_or(type_name);
    path.rsplit("::").next().unwrap_or(path)
}

/// What the panel shows that its views follow, compared between refreshes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct View {
    /// Whether the inspector tab shows.
    inspecting: bool,
    /// Whether the accessibility tab shows.
    auditing: bool,
    /// Whether pick mode is on.
    picking: bool,
    /// The node the inspector shows.
    selected: Option<smudge_core::tree::WidgetId>,
    /// Whether the focus overlay is on.
    focus: bool,
    /// Whether the hit testing overlay is on.
    hit_testing: bool,
}
