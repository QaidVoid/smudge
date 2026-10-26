//! The application: the single entry point that runs a frame from reactive
//! effects to damage.
//!
//! An [`App`] owns the reactive runtime, the widget tree, the update queue and
//! the damage for one window. [`App::frame`] runs the whole pipeline in order,
//! so a window, a headless test and a benchmark all drive the same code.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use web_time::Instant;

use crate::damage::DamageAccumulator;
use crate::element::{IntoElement, UpdateQueue, mount};
use crate::geometry::{Color, PixelRect, Point, Rect, Size};
use crate::input::{InputEvent, Modifiers};
use crate::paint::DisplayList;
use crate::reactive::runtime::Scope;
use crate::reactive::{self, Owner, RenderEffect};
use crate::theme::Theme;
use crate::timer::{self, Clock, TimerBook};
use crate::tree::{WidgetId, WidgetTree};

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
    /// The timers of the app and the clock they read.
    timers: Rc<RefCell<TimerBook>>,
    /// Whether the next frame repaints the whole surface.
    force_full: bool,
    /// The owner that the build closure, the signals it created, every
    /// node's owner and the theme bindings live under.
    ///
    /// Held so those stay alive for as long as the app does, and so the
    /// cleanup at the end stops the theme bindings.
    _root: Owner,
    /// The clipboard the app's handlers copy and paste through, when one is
    /// installed.
    clipboard: Option<crate::clipboard::Slot>,
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
    /// Timers read the operating system's clock. Use [`App::with_clock`] to
    /// run them on another one.
    pub fn new<E: IntoElement>(config: AppConfig, build: impl FnOnce() -> E) -> App {
        App::with_clock(config, Clock::system(), build)
    }

    /// Creates an app like [`App::new`] whose timers and tick callback read
    /// `clock`.
    ///
    /// The clock is in place before `build` runs, so a timer that `build`
    /// creates is scheduled on it too. A test passes a
    /// [`VirtualClock`](crate::timer::VirtualClock) and advances it by hand.
    pub fn with_clock<E: IntoElement>(
        config: AppConfig,
        clock: Clock,
        build: impl FnOnce() -> E,
    ) -> App {
        reactive::init();
        let root = Owner::new();
        let timers = Rc::new(RefCell::new(TimerBook::new(clock)));
        root.data().timer_book.replace(Some(Rc::downgrade(&timers)));
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
            timers,
            force_full: true,
            _root: root,
            clipboard: None,
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

    /// Replaces the theme that style values which name a token resolve
    /// against, and marks for repainting every node whose last paint looked
    /// up a token, whether through its style or inside its widget's own
    /// `paint`.
    ///
    /// The tree starts on [`Theme::light`]. Nodes whose paint resolves no
    /// value from the theme paint the same pixels under any theme and are not
    /// repainted, so a switch damages only what the switch can change. An
    /// equal theme marks nothing.
    ///
    /// ```
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::style::px;
    /// use smudge_core::theme::{ColorToken, Theme};
    /// use smudge_core::widget::panel;
    ///
    /// let mut app = App::new(
    ///     AppConfig {
    ///         viewport: Size {
    ///             width: 120.0,
    ///             height: 80.0,
    ///         },
    ///         scale: 1.0,
    ///         clear_color: Color::WHITE,
    ///         repaint: RepaintMode::Damage,
    ///     },
    ///     || panel().width(px(40.0)).height(px(20.0)).background(ColorToken::SURFACE),
    /// );
    /// app.frame();
    ///
    /// app.set_theme(Theme::dark());
    /// app.frame();
    /// assert_eq!(app.stats().nodes_painted, 1);
    /// ```
    pub fn set_theme(&mut self, theme: Theme) {
        self.tree.set_theme(theme);
    }

    /// Installs one effect that keeps the theme current.
    ///
    /// The closure is read when it is installed and again whenever a signal it
    /// reads changes; each run hands the theme it returns to
    /// [`App::set_theme`] through the update queue, so the change lands inside
    /// a frame like any other reactive update. Pass a closure that builds the
    /// theme from a signal to switch the theme reactively:
    ///
    /// ```
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::reactive::{prelude::*, signal_local};
    /// use smudge_core::style::px;
    /// use smudge_core::theme::{ColorToken, Theme};
    /// use smudge_core::widget::panel;
    ///
    /// let (dark, set_dark) = signal_local(false);
    /// let mut app = App::new(
    ///     AppConfig {
    ///         viewport: Size {
    ///             width: 120.0,
    ///             height: 80.0,
    ///         },
    ///         scale: 1.0,
    ///         clear_color: Color::WHITE,
    ///         repaint: RepaintMode::Damage,
    ///     },
    ///     || panel().width(px(40.0)).height(px(20.0)).background(ColorToken::SURFACE),
    /// );
    /// app.bind_theme(move || if dark.get() { Theme::dark() } else { Theme::light() });
    /// app.frame();
    ///
    /// set_dark.set(true);
    /// app.frame();
    /// assert_eq!(app.stats().nodes_painted, 1);
    /// ```
    ///
    /// The binding runs under the app's root owner, so it reports its wakes
    /// to the callback set with [`App::set_wake_callback`] and stops when the
    /// app is dropped.
    pub fn bind_theme(&mut self, get: impl Fn() -> Theme + 'static) {
        let queue = self.queue.clone();
        self._root.with(|| {
            RenderEffect::new(move || {
                let theme = get();
                let queue = queue.clone();
                queue.push(Box::new(move |tree| tree.set_theme(theme)));
            });
        });
    }

    /// Installs the clipboard the app's widgets copy and paste through.
    ///
    /// Until one is installed, a clipboard read or write finds nothing, so
    /// cut, copy and paste keys do nothing. The windowing layer installs one
    /// that reaches the system clipboard; a test installs a
    /// [`MemoryClipboard`](crate::clipboard::MemoryClipboard) and keeps a
    /// clone to seed or read the text. Installing again replaces the
    /// previous clipboard.
    pub fn set_clipboard(&mut self, clipboard: Box<dyn crate::clipboard::Clipboard>) {
        self.clipboard = Some(crate::clipboard::install(self._root.data(), clipboard));
    }

    /// Sets the callback that a wake in this app's tree runs to request a
    /// redraw.
    ///
    /// A write that queues this app's first effect since the last flush runs
    /// the callback once, so the platform learns that the next frame has work
    /// to do. Every binding of this app reports its wake here, and an app on
    /// another part of the thread neither sees this app's wakes nor hides
    /// them. A write made while [`App::frame`] runs its timers, its tick and
    /// its updates runs no callback, because the frame runs the effects it
    /// wakes before it lays out; a write made later in the frame, while a
    /// widget paints, does. See
    /// [`set_wake_callback`](crate::reactive::set_wake_callback) for the
    /// exact rules.
    pub fn set_wake_callback(&self, callback: impl Fn() + 'static) {
        self._root.set_wake_callback(callback);
    }

    /// Sets a callback that runs once at the start of every frame with the
    /// current time.
    ///
    /// The time comes from the app's [`Clock`], the same one its timers
    /// read, so a test that runs the app on a virtual clock drives both.
    pub fn set_tick(&mut self, tick: impl FnMut(Instant) + 'static) {
        self.tick = Some(Box::new(tick));
    }

    /// Returns the current time of the app's clock.
    pub fn now(&self) -> Instant {
        self.timers.borrow().clock.now()
    }

    /// Returns the earliest time a timer fires at, or `None` when none is
    /// scheduled.
    ///
    /// A platform that has no tick callback waits until this deadline, so a
    /// pending timer wakes the window when it comes due instead of the
    /// window polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers.borrow().next_deadline()
    }

    /// Runs the timers that are due on the app's clock, outside a frame.
    ///
    /// A timer that writes a signal wakes the app through the callback set
    /// with [`App::set_wake_callback`], like any write between frames, and a
    /// timer that changes nothing asks for nothing. A platform calls this
    /// when [`App::next_deadline`] passes, so a due timer costs a frame only
    /// when it has something to show. [`App::frame`] runs due timers itself
    /// as well, so a caller that draws every frame needs no call.
    pub fn run_due_timers(&mut self) {
        timer::fire_due(&self.timers);
    }

    /// Returns whether a frame tick callback is set.
    pub fn has_tick(&self) -> bool {
        self.tick.is_some()
    }

    /// Returns whether an event left work that only a frame can finish.
    ///
    /// An input event can change how the tree looks without writing a signal,
    /// for example by hovering a node with a hover style, so the reactive wake
    /// callback never hears of it. A platform calls this after handing events
    /// over and asks for a frame when it is `true`. Most pointer moves change
    /// nothing, and asking for a frame every time would present the window
    /// for nothing.
    pub fn needs_frame(&self) -> bool {
        self.tree.has_pending_work()
    }

    /// Handles one input event between frames.
    ///
    /// The pointer events are routed to the tree: a move moves the hovered
    /// chain, a primary press captures the node under the pointer and focuses
    /// it when it can take the focus, and a release runs the captured node's
    /// `on_press` when it happens over it. A press elsewhere clears the focus,
    /// and any press turns the keyboard navigation mode off. A wheel scroll
    /// goes to the deepest node in the hovered chain that has a wheel handler.
    /// Key events are routed to the focused node and bubble to its ancestors
    /// until a handler takes them, and an unhandled Tab or Shift+Tab moves the
    /// focus. Handlers run at once, inside their node's owner, and a handler
    /// that writes a signal makes the change visible with the next call to
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
            InputEvent::Wheel { delta } => self.tree.wheel(delta),
            InputEvent::Key(key) => {
                self.tree.key(&key);
                self.modifiers = key.modifiers;
            }
            InputEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            InputEvent::ImeCommit { text } => self.tree.ime_commit(&text),
            InputEvent::ImePreedit { text, cursor } => self.tree.ime_preedit(&text, cursor),
            InputEvent::WindowFocus(focused) => {
                // A window that loses the input focus drops its input method
                // context, so a composition must not outlive the focus. The
                // empty preedit is the clear.
                if !focused {
                    self.tree.ime_preedit("", None);
                }
            }
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

    /// Returns the root owner, for the crate's own tests.
    #[cfg(test)]
    pub(crate) fn root(&self) -> &Owner {
        &self._root
    }

    /// Returns the node that holds the keyboard focus, when one does.
    ///
    /// A press on a focusable node focuses it, a press elsewhere clears the
    /// focus, and Tab moves it. Removing the focused node clears the focus.
    pub fn focused(&self) -> Option<WidgetId> {
        self.tree.focused()
    }

    /// Returns whether the keyboard navigation mode is on.
    ///
    /// A Tab or an arrow key turns the mode on and a pointer press turns it
    /// off, so a focus ring can draw for keyboard users alone. A node carries
    /// the keyboard focus bit only while it holds the focus and this mode is
    /// on.
    pub fn keyboard_navigation(&self) -> bool {
        self.tree.keyboard_nav()
    }

    /// Returns the area the focused widget reports to the input method, in
    /// window coordinates, when it reports one.
    ///
    /// This is [`None`] while no widget reports an area, which is the default,
    /// and while nothing is focused. A window presents the area to the
    /// operating system so the input method can anchor its candidate window to
    /// it.
    pub fn ime_area(&self) -> Option<Rect> {
        self.tree.ime_area()
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
    /// 1. The timers that are due on the app's clock, then the tick callback,
    ///    when one is set.
    /// 2. Reactive effects and the updates they queue, until neither has work
    ///    left. A signal write therefore lands before layout, including one a
    ///    timer made at the start of the frame.
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
        // The frame runs every effect its timers, its tick and its updates
        // wake before it lays out, so reporting those wakes would only ask
        // the platform for a second frame with nothing to do.
        let quiet = QuietWakes::enter(&self._root);
        timer::fire_due(&self.timers);

        let now = self.now();
        if let Some(tick) = &mut self.tick {
            tick(now);
        }

        let flush_start = Instant::now();
        let mut rounds = 0u32;
        let mut settled = false;
        loop {
            rounds += 1;
            let flushed = reactive::flush();
            let applied = self.queue.apply(&mut self.tree);
            if !flushed && !applied {
                settled = true;
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
        drop(quiet);
        if !settled {
            // The capped loop left work queued that no frame has run yet.
            self._root.wake();
        }

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

/// Keeps the wakes of one app's owner tree unreported while its frame runs
/// them itself, and reports them again when dropped, even when the frame
/// unwinds.
struct QuietWakes(Rc<Scope>);

impl QuietWakes {
    /// Silences the wakes of the tree `root` belongs to.
    fn enter(root: &Owner) -> QuietWakes {
        let scope = Rc::clone(&root.data().scope);
        scope.quiet.set(true);
        QuietWakes(scope)
    }
}

impl Drop for QuietWakes {
    fn drop(&mut self) {
        self.0.quiet.set(false);
    }
}
