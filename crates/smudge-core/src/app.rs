//! The application: the single entry point that runs a frame from reactive
//! effects to damage.
//!
//! An [`App`] owns the reactive runtime, the widget tree, the update queue and
//! the damage for one window. [`App::frame`] runs the whole pipeline in order,
//! so a window, a headless test and a benchmark all drive the same code.

use std::time::Duration;

use web_time::Instant;

use crate::damage::DamageAccumulator;
use crate::element::{IntoElement, UpdateQueue, mount};
use crate::geometry::{Color, PixelRect, Point, Rect, Size};
use crate::input::{InputEvent, Modifiers};
use crate::paint::DisplayList;
use crate::reactive::{self, Owner};
use crate::tree::WidgetTree;

/// How many flush and apply rounds a frame runs before it reports a cycle.
const MAX_FLUSH_ROUNDS: u32 = 16;

/// How much of the surface a frame repaints.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum RepaintMode {
    /// Repaint only the pixels that changed since the previous frame.
    #[default]
    Damage,
    /// Repaint the whole surface on every frame.
    Full,
}

/// The settings an [`App`] starts with.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// The size of the window in logical pixels.
    pub viewport: Size,
    /// The number of physical pixels per logical pixel.
    pub scale: f32,
    /// The color a renderer fills the surface with before it replays the
    /// display list.
    pub clear_color: Color,
    /// How much of the surface each frame repaints.
    pub repaint: RepaintMode,
}

/// What a renderer draws for one frame.
#[derive(Debug)]
pub struct FrameOutput<'a> {
    /// The operations to replay, in paint order.
    pub list: &'a DisplayList,
    /// The physical rectangles that changed since the previous frame.
    pub damage: &'a [PixelRect],
    /// Whether the frame repaints the whole surface.
    pub full: bool,
    /// The color the surface starts from, before `list` is replayed.
    pub clear_color: Color,
    /// The number of physical pixels per logical pixel.
    pub scale: f32,
}

/// How long each step of the last frame took and how much work it did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Time spent running reactive effects and applying their updates.
    pub flush: Duration,
    /// Time spent in the layout pass.
    pub layout: Duration,
    /// Time spent in the paint pass.
    pub paint: Duration,
    /// Time spent rebuilding the display list.
    pub list: Duration,
    /// Time spent converting and merging the damage.
    pub damage: Duration,
    /// Whether the layout pass ran the layout engine.
    pub layout_ran: bool,
    /// How many nodes the paint pass regenerated.
    pub nodes_painted: u32,
    /// How many physical rectangles the damage holds.
    pub damage_rects: u32,
    /// How many physical pixels the damage covers, counting an overlap once per
    /// rectangle.
    pub damage_pixels: u64,
    /// How many physical pixels the damage covered before the merge step
    /// collapsed the rectangles, counting an overlap once per rectangle.
    ///
    /// Together with [`FrameStats::damage_pixels`] this shows what the merge
    /// step traded: rectangles for area.
    pub damage_pixels_before_merge: u64,
    /// How many flush and apply rounds the frame ran.
    pub flush_rounds: u32,
}

/// One window's widget tree, reactive runtime and frame pipeline.
///
/// The app owns everything the frame touches. [`App::frame`] is the only method
/// that mutates the tree, and it does so in one fixed order, so an effect never
/// observes a half-applied frame.
///
/// # Examples
///
/// ```
/// use smudge_core::app::{App, AppConfig, RepaintMode};
/// use smudge_core::geometry::{Color, Size};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
///
/// let mut app = App::new(
///     AppConfig {
///         viewport: Size {
///             width: 320.0,
///             height: 200.0,
///         },
///         scale: 1.0,
///         clear_color: Color::WHITE,
///         repaint: RepaintMode::Damage,
///     },
///     || panel().width(px(120.0)).height(px(40.0)).background(Color::BLACK),
/// );
///
/// let full = app.frame().full;
/// assert!(full, "the first frame repaints the whole surface");
/// assert_eq!(app.stats().damage_rects, 1);
///
/// let empty = app.frame().damage.is_empty();
/// assert!(empty, "a frame with no changes damages nothing");
/// ```
pub struct App {
    /// The widget tree the frame lays out and paints.
    tree: WidgetTree,
    /// The updates that reactive effects have queued.
    queue: UpdateQueue,
    /// The damage collected for the frame that is running.
    damage: DamageAccumulator,
    /// The display list of the last frame that built one.
    list: DisplayList,
    /// The settings the frame uses.
    config: AppConfig,
    /// The pointer position the last pointer event reported, in logical
    /// pixels.
    pointer: Point,
    /// The keyboard modifiers the last key event reported.
    modifiers: Modifiers,
    /// The timings and counts of the last frame.
    stats: FrameStats,
    /// The callback that runs at the start of every frame.
    tick: Option<Box<dyn FnMut(Instant)>>,
    /// Whether the next frame repaints the whole surface.
    force_full: bool,
    /// The owner that the build closure, the signals it created and every
    /// node's owner live under.
    ///
    /// Held so those signals stay alive for as long as the app does.
    _root: Owner,
}

impl App {
    /// Creates an app and mounts the element that `build` returns.
    ///
    /// Installs the reactive runtime on the calling thread, then runs `build`
    /// and mounts its element inside an owner that lives as long as the app, so
    /// every binding sees the context that `build` provides. The mounted tree
    /// is laid out and painted by the first call to [`App::frame`], which
    /// repaints the whole surface.
    ///
    /// # Panics
    ///
    /// Panics when another async executor was installed in this process
    /// before, because the reactive runtime installs a process-wide one.
    pub fn new<E: IntoElement>(config: AppConfig, build: impl FnOnce() -> E) -> App {
        reactive::init();
        let root = Owner::new();
        let mut tree = WidgetTree::new();
        let queue = UpdateQueue::default();
        root.with(|| mount(build().into_any(), &mut tree, &queue, None));

        App {
            tree,
            queue,
            damage: DamageAccumulator::new(),
            list: DisplayList::new(),
            config,
            pointer: Point::default(),
            modifiers: Modifiers::default(),
            stats: FrameStats::default(),
            tick: None,
            force_full: true,
            _root: root,
        }
    }

    /// Changes the viewport and the scale, and makes the next frame repaint the
    /// whole surface.
    ///
    /// The viewport is in logical pixels and the scale is the number of
    /// physical pixels per logical pixel. A viewport that differs from the last
    /// one also runs the layout pass again.
    pub fn set_viewport(&mut self, viewport: Size, scale: f32) {
        self.config.viewport = viewport;
        self.config.scale = scale;
        self.force_full = true;
    }

    /// Changes how much of the surface each frame repaints, and makes the next
    /// frame repaint the whole surface.
    pub fn set_repaint_mode(&mut self, mode: RepaintMode) {
        self.config.repaint = mode;
        self.force_full = true;
    }

    /// Sets a callback that runs once at the start of every frame with the
    /// current time.
    ///
    /// The clock is [`web_time::Instant`], which is [`std::time::Instant`] on
    /// every target that has one and reads the browser's clock on the web,
    /// where `std::time::Instant::now` panics.
    pub fn set_tick(&mut self, tick: impl FnMut(Instant) + 'static) {
        self.tick = Some(Box::new(tick));
    }

    /// Returns whether a frame tick callback is set.
    pub fn has_tick(&self) -> bool {
        self.tick.is_some()
    }

    /// Handles one input event between frames.
    ///
    /// The pointer events are routed to the tree: a move moves the hovered
    /// chain, a primary press captures the node under the pointer, and a
    /// release runs the captured node's `on_press` when it happens over it.
    /// Handlers run at once, inside their node's owner, and a handler that
    /// writes a signal makes the change visible with the next call to
    /// [`App::frame`]. The app also records the pointer position from pointer
    /// events and the keyboard modifiers from key and modifiers events.
    pub fn handle_event(&mut self, event: InputEvent) {
        match event {
            InputEvent::PointerMoved { position } => {
                self.pointer = position;
                self.tree.pointer_move(position);
            }
            InputEvent::PointerDown { position, button } => {
                self.pointer = position;
                self.tree.pointer_down(position, button);
            }
            InputEvent::PointerUp { position, button } => {
                self.pointer = position;
                self.tree.pointer_up(position, button);
            }
            InputEvent::PointerLeft => self.tree.pointer_left(),
            InputEvent::Key(key) => self.modifiers = key.modifiers,
            InputEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            InputEvent::Wheel { .. }
            | InputEvent::ImePreedit { .. }
            | InputEvent::ImeCommit { .. }
            | InputEvent::WindowFocus(_) => {}
        }
    }

    /// Returns the pointer position the last pointer event reported, in
    /// logical pixels.
    ///
    /// The position starts at the origin and moves only when a pointer event
    /// carries a position, so it lags the real pointer until the first event
    /// arrives.
    pub fn pointer_position(&self) -> Point {
        self.pointer
    }

    /// Returns the keyboard modifiers the last key or modifiers event
    /// reported.
    ///
    /// They start at none and change only when one of those events arrives.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Returns the surface size in physical pixels.
    ///
    /// This is the viewport multiplied by the scale and rounded to the nearest
    /// pixel. A viewport or scale that gives a negative or NaN size gives zero
    /// on that axis.
    pub fn surface_size(&self) -> (u32, u32) {
        let width = (self.config.viewport.width * self.config.scale).round();
        let height = (self.config.viewport.height * self.config.scale).round();
        (width.max(0.0) as u32, height.max(0.0) as u32)
    }

    /// Runs one frame and returns what the renderer has to draw.
    ///
    /// The frame runs these steps in order:
    ///
    /// 1. The tick callback, when one is set.
    /// 2. Reactive effects and the updates they queue, until neither has work
    ///    left. A signal write therefore lands before layout.
    /// 3. The layout pass, which runs the layout engine only when a node needs
    ///    it.
    /// 4. The paint pass, which regenerates the operations of the nodes that
    ///    changed and reports their old and new bounds as damage.
    /// 5. The display list, rebuilt only when something may have changed.
    /// 6. Damage conversion and merging, then the output.
    ///
    /// The output borrows the app until it is dropped, so read [`App::stats`]
    /// either before the output is kept or after it is let go.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the update queue does not settle within a
    /// fixed number of rounds, which means applying updates keeps waking a
    /// binding: a mounted child or a widget's bind function writes a signal
    /// that leads back to the same update. Release builds carry on with the
    /// state they reached.
    ///
    /// Also panics in debug builds when one binding runs too many times in one
    /// flush, which means it writes a signal that wakes it again, directly or
    /// through another binding. A release build never returns from that frame.
    pub fn frame(&mut self) -> FrameOutput<'_> {
        if let Some(tick) = &mut self.tick {
            tick(Instant::now());
        }

        let flush_start = Instant::now();
        let mut rounds = 0u32;
        loop {
            rounds += 1;
            let flushed = reactive::flush();
            let applied = self.queue.apply(&mut self.tree);
            if !flushed && !applied {
                break;
            }
            debug_assert!(
                rounds < MAX_FLUSH_ROUNDS,
                "the update queue did not settle in {MAX_FLUSH_ROUNDS} rounds, \
                 so applying updates keeps waking a binding in a cycle, for \
                 example through a mounted child or a widget's bind function"
            );
            if rounds >= MAX_FLUSH_ROUNDS {
                break;
            }
        }
        self.stats.flush = flush_start.elapsed();
        self.stats.flush_rounds = rounds;

        let layout_start = Instant::now();
        self.stats.layout_ran = self.tree.layout(self.config.viewport);
        self.stats.layout = layout_start.elapsed();

        self.damage.clear();
        let paint_start = Instant::now();
        self.stats.nodes_painted = self.tree.paint(&mut self.damage);
        self.stats.paint = paint_start.elapsed();

        let full = self.force_full || self.config.repaint == RepaintMode::Full;
        self.force_full = false;

        let list_start = Instant::now();
        if full || self.stats.nodes_painted > 0 || !self.damage.is_empty() {
            self.tree.build_display_list(&mut self.list);
        }
        self.stats.list = list_start.elapsed();

        if full {
            self.damage.add_full();
        }

        let (width, height) = self.surface_size();
        let damage_start = Instant::now();
        let (damage, damage_pixels_before_merge) =
            self.damage
                .finish_measured(self.config.scale, width, height);
        self.stats.damage = damage_start.elapsed();
        self.stats.damage_rects = damage.len() as u32;
        self.stats.damage_pixels = damage.iter().map(|rect| rect.area()).sum();
        self.stats.damage_pixels_before_merge = damage_pixels_before_merge;

        FrameOutput {
            list: &self.list,
            damage,
            full,
            clear_color: self.config.clear_color,
            scale: self.config.scale,
        }
    }

    /// Returns the timings and counts of the last frame.
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    /// Visits every visible node of the tree in paint order, for diagnostics.
    ///
    /// `visit` receives the node's depth, with the root at zero, the full name
    /// of the widget's concrete type, and the node's absolute rectangle in
    /// logical pixels as the last layout pass left it. A hidden node and its
    /// subtree are skipped, because they are laid out as empty and paint
    /// nothing. The tree itself stays private, so this is the only way code
    /// outside this crate can describe the structure of a frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::style::px;
    /// use smudge_core::widget::panel;
    ///
    /// let mut app = App::new(
    ///     AppConfig {
    ///         viewport: Size {
    ///             width: 100.0,
    ///             height: 50.0,
    ///         },
    ///         scale: 1.0,
    ///         clear_color: Color::WHITE,
    ///         repaint: RepaintMode::Damage,
    ///     },
    ///     || panel().child(panel().width(px(10.0)).height(px(10.0))),
    /// );
    /// app.frame();
    ///
    /// let mut depths = Vec::new();
    /// app.visit_nodes(|depth, _type_name, _rect| depths.push(depth));
    /// assert_eq!(depths, [0, 1]);
    /// ```
    pub fn visit_nodes(&self, mut visit: impl FnMut(usize, &'static str, Rect)) {
        self.tree.visit_nodes(&mut visit);
    }
}
