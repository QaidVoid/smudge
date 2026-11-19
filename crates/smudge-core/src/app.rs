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
use crate::reactive::prelude::{Get, Set};
use crate::reactive::runtime::Scope;
use crate::reactive::{self, Owner, RenderEffect, RwSignal};
use crate::semantics::{A11yChanges, SemanticsNode};
use crate::theme::{ColorScheme, Theme};
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
    /// The channels other threads send values through, and the book the
    /// frame drains them from.
    channels: Rc<RefCell<crate::channel::ChannelBook>>,
    /// The window commands handlers queue and the state the window reports,
    /// shared with every owner of the tree.
    window: Rc<RefCell<crate::window::WindowBook>>,
    /// The hot patch hooks the app registered, which the root owner reaches
    /// through a weak link, like the timers and the channels.
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    hooks: Rc<RefCell<crate::hot::HotPatchBook>>,
    /// Whether the next frame repaints the whole surface.
    force_full: bool,
    /// The old build's signal values, still on offer to the parts of the
    /// new build that mount in the frames after a remount.
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    carrying: Option<crate::reactive::carry::Offered>,
    /// The system color scheme the last report named, even when the app does
    /// not follow it, so a follower that arrives later starts on it.
    scheme: Option<ColorScheme>,
    /// What the app paints with when it follows the system color scheme.
    scheme_follow: Option<SchemeFollow>,
    /// Whether motion is reduced, as the app itself set it, which wins over
    /// the system's setting.
    reduced_motion_app: Option<bool>,
    /// Whether the system asks for reduced motion, as the windowing layer
    /// last reported it.
    reduced_motion_system: bool,
    /// Whether a remount replaced the text fields since the window last
    /// asked, so the input method's composition belongs to none of them.
    ime_reset: bool,
    /// The owner that the build closure, the signals it created and every
    /// node's owner live under.
    ///
    /// A child of the root owner. [`App::remount`] cleans it up and replaces
    /// it, which disposes everything the previous build created.
    build_owner: Owner,
    /// The owner that the build owner, the theme bindings and the books of
    /// timers, channels, window commands and the clipboard live under.
    ///
    /// Held so those stay alive for as long as the app does, across every
    /// remount, and so the cleanup at the end stops the theme bindings.
    _root: Owner,
    /// The clipboard the app's handlers copy and paste through, when one is
    /// installed.
    clipboard: Option<crate::clipboard::Slot>,
}

/// What an app that follows the system color scheme paints with.
struct SchemeFollow {
    /// The scheme as it was last reported, whose writes drive the binding.
    scheme: RwSignal<ColorScheme>,
    /// The themes and clear colors, shared with the binding so a later
    /// follow swaps them under it.
    pair: Rc<RefCell<SchemePair>>,
}

/// The themes an app that follows the color scheme paints with.
#[derive(Clone)]
struct SchemePair {
    /// The theme a light report paints with.
    light: Theme,
    /// The color a light report starts the surface from.
    light_clear: Color,
    /// The theme a dark report paints with.
    dark: Theme,
    /// The color a dark report starts the surface from.
    dark_clear: Color,
}

impl App {
    /// Creates an app and mounts the element that `build` returns.
    ///
    /// Installs the reactive runtime on the calling thread, then runs `build`
    /// and mounts its element inside an owner that lives until the app drops
    /// or [`App::remount`] replaces the tree, so every binding sees the
    /// context that `build` provides. The mounted tree is laid out and
    /// painted by the first call to [`App::frame`], which repaints the whole
    /// surface.
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
        let channels = Rc::new(RefCell::new(crate::channel::ChannelBook::default()));
        root.data()
            .channel_book
            .replace(Some(Rc::downgrade(&channels)));
        let window = crate::window::new_book(&root);
        root.data()
            .window_book
            .replace(Some(Rc::downgrade(&window)));
        #[cfg(all(feature = "hot-reload", target_os = "linux"))]
        let hooks = Rc::new(RefCell::new(crate::hot::HotPatchBook::new()));
        #[cfg(all(feature = "hot-reload", target_os = "linux"))]
        root.data()
            .hot_patch_book
            .replace(Some(Rc::downgrade(&hooks)));
        let mut tree = WidgetTree::new();
        tree.set_scale(config.scale);
        let queue = UpdateQueue::default();
        let build_owner = root.child();
        {
            // A first build made inside a value's drop still registers its
            // signals for carrying.
            #[cfg(all(feature = "hot-reload", target_os = "linux"))]
            let _mounting = crate::reactive::carry::Mounting::enter(build_owner.data());
            build_owner.with(|| mount(build().into_any(), &mut tree, &queue, None));
        }

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
            channels,
            window,
            #[cfg(all(feature = "hot-reload", target_os = "linux"))]
            hooks,
            force_full: true,
            #[cfg(all(feature = "hot-reload", target_os = "linux"))]
            carrying: None,
            scheme: None,
            scheme_follow: None,
            reduced_motion_app: None,
            reduced_motion_system: false,
            ime_reset: false,
            build_owner,
            _root: root,
            clipboard: None,
        }
    }

    /// Runs `setup` with the app under its root owner, which lives for as
    /// long as the app does and survives every remount.
    ///
    /// This is the step the window calls once after the first build mounted
    /// and before the first frame. Timers and hot patch hooks that `setup`
    /// creates belong to the root owner, so they outlive every build: a
    /// remount cleans up the build's owner and leaves these running. A
    /// repeating timer created here should be detached, so dropping the
    /// `setup` scope does not cancel it.
    ///
    /// Outside the window this gives long-lived registration a scope of its
    /// own, for the same reason.
    pub fn configure(&mut self, setup: impl FnOnce(&mut App)) {
        let root = self._root.clone();
        root.with(|| setup(self));
    }

    /// Throws the whole tree away and mounts the element that `build` returns
    /// in its place.
    ///
    /// Everything the previous build created goes with its tree: the
    /// signals, memos and effects, the timers, and the channels, whose
    /// senders answer every later `send` with an error. The node owners'
    /// cleanup functions run as they do when a [`show`](crate::element::show)
    /// child unmounts. What was created outside the build stays: signals
    /// created before the app or with no owner, the theme and the bindings of
    /// [`App::bind_theme`], the tick callback, the clipboard, the wake
    /// callbacks and the window state. So a build that reads its state from
    /// signals kept outside it draws that state again, through whatever code
    /// `build` runs now.
    ///
    /// The tree's own state starts over. A held press ends as
    /// [`App::release_press`] ends it, so its node hears `on_pointer_up`
    /// before it goes and the release that follows reaches no handler. The
    /// hover, the focus traps and the autofocus requests of the old tree are
    /// dropped, and the remount hovers the new node under the pointer
    /// before it returns.
    /// The focus moves to the node that sits at the same child indices from
    /// the root as the focused node did, when that node is focusable, can
    /// hold the focus and lies inside the active focus trap, if any. Its
    /// `on_focus` hears `true`, the remount scrolls it into view before it
    /// returns, as [`Element::on_reveal`](crate::element::Element::on_reveal)
    /// describes, and the autofocus requests of the new build's visible
    /// nodes are dropped, so the restored focus stays. A request
    /// whose node is hidden waits as usual. Without a restored focus, an
    /// autofocus request takes it as on a first mount. The keyboard
    /// navigation mode is kept.
    ///
    /// Updates queued before the call reach no node, because every id they
    /// name is gone, except a theme that [`App::bind_theme`] queued, which
    /// still lands. The updates the new build queues at mount are applied
    /// at once, so the restored focus can find a node that a
    /// [`show`](crate::element::show) or [`for_each`](crate::element::for_each)
    /// mounts. The new tree is laid out before this returns, so input that
    /// arrives before the next frame hits it. The next frame lays it out
    /// again after its flush and scrolls the focused node into view again,
    /// unless a press comes first, so the signals that handlers wrote during
    /// the remount move the layout and the scroll before anything paints.
    /// That frame repaints the whole surface, and the next
    /// [`App::take_a11y_changes`] reports a reset, so an accessibility
    /// backend rebuilds its tree.
    ///
    /// The call needs the app itself, so it happens between frames and
    /// between events, where the windowing layer and a test hold the app.
    /// A handler, a timer, a channel handler, an effect or the tick callback
    /// runs while the app is borrowed for a frame or an event and cannot
    /// reach it, so a remount never interrupts a running frame.
    ///
    /// A remount is how a patched build replaces a running one, and it is
    /// just as useful without that: an action that resets the interface to
    /// its first state keeps the application's data in signals outside the
    /// build and remounts the same build.
    ///
    /// With the `hot-reload` feature on Linux, the remount that follows a
    /// hot patch carries the values of the signals the old build created
    /// into the new one. A signal the new build creates while it mounts
    /// takes over the old build's signal with the same key: the path of its
    /// owner from the build's owner, each owner by its place among its
    /// parent's live children when it was created, the source file that
    /// called [`RwSignal::new`] or [`signal_local`], its ordinal among the
    /// signals of that file and type under that owner, and its type. The
    /// owners of a [`for_each`](crate::element::for_each) row go by the
    /// row's key, so a row finds its values wherever the rows moved, and a
    /// value crosses only to the row of a key equal to its old row's. Two
    /// siblings that stand alike, two rows whose keys hash alike or two
    /// owners that took one place at different times, carry nothing. The
    /// line is not part of the key, so a line added above a signal keeps its
    /// value, while two signals of one type in one owner and file that swap
    /// places swap their values. The new signal adopts the old one's
    /// storage, so a handle kept from the old build names it too.
    ///
    /// A value is carried only when its type kept its size, its alignment
    /// and, as the patch builder reports through `hot::note_kept_types`, the
    /// layout the live values hold. The builder never vouches for a type
    /// that holds a signal, an owner, an effect, a timer or a channel, whose
    /// handles would point into the old build, nor for one that holds a
    /// trait object, whose contents no layout of the holder describes, nor
    /// for a type whose probe
    /// the binary the app started with lacks, such as one first used after
    /// a patch, or one a patch reaches through a copy of the probe no
    /// earlier build compiled. The parts of the new build that mount in the
    /// frames after the remount, such as a list's rows, claim their values
    /// then: the offer lasts until every value was claimed or two frames in
    /// a row claim none. A signal a handler, an effect or a timer creates
    /// never claims a value. Every other remount, and a signal no key
    /// matches, starts from the value the new build gives it.
    ///
    /// [`RwSignal::new`]: crate::reactive::RwSignal::new
    /// [`signal_local`]: crate::reactive::signal_local
    ///
    /// # Panics
    ///
    /// Panics when `build` panics, or when code the remount runs for the
    /// application panics: a cleanup function, the `on_pointer_up` handler of
    /// a held press, an `on_hover` handler of the old tree when that press
    /// ends or of the new tree under the pointer, an `on_focus` handler, an
    /// `on_reveal` handler or a widget's bind function.
    ///
    /// # Examples
    ///
    /// ```
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::reactive::{RwSignal, prelude::*};
    /// use smudge_core::style::px;
    /// use smudge_core::widget::panel;
    ///
    /// let width = RwSignal::new(30.0);
    /// let build = {
    ///     let width = width.clone();
    ///     move || panel().child(panel().width(move || px(width.get())).height(px(10.0)))
    /// };
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
    ///     build.clone(),
    /// );
    /// app.frame();
    ///
    /// width.set(50.0);
    /// app.remount(build);
    /// assert!(app.frame().full, "a remount repaints the whole surface");
    ///
    /// let mut widths = Vec::new();
    /// app.visit_nodes(|_, _, rect| widths.push(rect.x1 - rect.x0));
    /// assert_eq!(widths[1], 50.0, "the signal kept outside the build kept its value");
    /// ```
    pub fn remount<E: IntoElement>(&mut self, build: impl FnOnce() -> E) {
        // The values are taken before the tree is unmounted, which cleans
        // up the owners of the nodes it removes.
        // An offer an earlier remount left ends first, so its values never
        // reach this build. The patch's list of kept types is taken before,
        // since ending the offer drops values whose drop can run any code,
        // a remount of another app included.
        #[cfg(all(feature = "hot-reload", target_os = "linux"))]
        let carried = {
            let kept = crate::hot::take_kept();
            self.carrying = None;
            kept.map(|kept| crate::reactive::carry::take(self.build_owner.data(), kept))
        };
        let focus = self.tree.focus_path();
        self.tree.unmount_root();
        self.build_owner.cleanup();
        self.build_owner = self._root.child();
        #[cfg(all(feature = "hot-reload", target_os = "linux"))]
        {
            self.carrying = carried.map(|taken| taken.offer(self.build_owner.data()));
        }
        {
            #[cfg(all(feature = "hot-reload", target_os = "linux"))]
            let _mounting = crate::reactive::carry::Mounting::enter(self.build_owner.data());
            self.build_owner
                .with(|| mount(build().into_any(), &mut self.tree, &self.queue, None));
            self.queue.apply(&mut self.tree);
        }
        if let Some(path) = focus {
            self.tree.restore_focus(&path);
        }
        // Input that arrives before the next frame hits the new tree, so it
        // is laid out now, and that frame lays it out again after its flush.
        self.tree.layout_ahead(self.config.viewport);
        self.force_full = true;
        self.ime_reset = true;
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
        self.tree.set_scale(scale);
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
        let get = crate::hot::wrap(get);
        let queue = self.queue.clone();
        self._root.with(|| {
            RenderEffect::new_unwrapped(move || {
                let theme = get();
                let queue = queue.clone();
                queue.push(Box::new(move |tree| tree.set_theme(theme)));
            });
        });
    }

    /// Follows the system color scheme, switching the app's theme and the
    /// color the surface starts from when it changes.
    ///
    /// The windowing layer reports the scheme the desktop holds, and reports
    /// again whenever it changes; while the app follows, a light report
    /// paints `light` over `light_clear` and a dark report paints `dark`
    /// over `dark_clear`. Both clear colors are used opaque, whatever alpha
    /// they carry, because presentation copies only the color channels, so
    /// a translucent one cannot fail a frame later. Following
    /// replaces what [`App::set_theme`] last
    /// set: from here on the scheme's changes choose the theme, until
    /// another call to this one names new themes.
    ///
    /// An app that never calls this paints the themes it sets itself. The
    /// desktop holds no scheme an app can read everywhere, so on a platform
    /// that reports none the app keeps the light side until a report says
    /// otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::theme::{ColorScheme, Theme};
    /// use smudge_core::widget::panel;
    ///
    /// # let mut app = App::new(
    /// #     AppConfig {
    /// #         viewport: Size { width: 120.0, height: 80.0 },
    /// #         scale: 1.0,
    /// #         clear_color: Color::WHITE,
    /// #         repaint: RepaintMode::Damage,
    /// #     },
    /// #     || panel(),
    /// # );
    /// app.follow_color_scheme(
    ///     Theme::light(),
    ///     Color::WHITE,
    ///     Theme::dark(),
    ///     Color::rgb(16, 17, 21),
    /// );
    /// app.set_color_scheme(ColorScheme::Dark);
    /// assert_eq!(app.frame().clear_color, Color::rgb(16, 17, 21));
    /// ```
    pub fn follow_color_scheme(
        &mut self,
        light: Theme,
        light_clear: Color,
        dark: Theme,
        dark_clear: Color,
    ) {
        let light_clear = Color {
            a: 255,
            ..light_clear
        };
        let dark_clear = Color {
            a: 255,
            ..dark_clear
        };
        if let Some(follow) = &self.scheme_follow {
            // A later follow swaps the pair under the standing binding; the
            // write below wakes it, and it reads the new themes next frame.
            let mut pair = follow.pair.borrow_mut();
            *pair = SchemePair {
                light,
                light_clear,
                dark,
                dark_clear,
            };
        } else {
            // The signal lives under the root owner, like the binding, so
            // it dies with the app and survives every remount. The pair is
            // shared so the binding reads whatever the latest follow named.
            let pair = Rc::new(RefCell::new(SchemePair {
                light,
                light_clear,
                dark,
                dark_clear,
            }));
            let scheme = self
                ._root
                .with(|| RwSignal::new(self.scheme.unwrap_or(ColorScheme::Light)));
            let signal = scheme.clone();
            let read = pair.clone();
            let queue = self.queue.clone();
            self._root.with(|| {
                RenderEffect::new_unwrapped(move || {
                    let pair = read.borrow().clone();
                    let theme = if signal.get() == ColorScheme::Dark {
                        pair.dark
                    } else {
                        pair.light
                    };
                    let queue = queue.clone();
                    queue.push(Box::new(move |tree| tree.set_theme(theme)));
                });
            });
            self.scheme_follow = Some(SchemeFollow { scheme, pair });
        }
        // A report may have arrived before the app started following, and a
        // first follow names the clear color the window has not applied yet.
        let start = self.scheme.unwrap_or(ColorScheme::Light);
        self.set_color_scheme(start);
    }

    /// Reports the system color scheme the desktop holds.
    ///
    /// The windowing layer calls it when the scheme changes. An app that
    /// follows the scheme through [`App::follow_color_scheme`] switches its
    /// theme and clear color; any app keeps the report, and a later call to
    /// [`App::follow_color_scheme`] starts on it.
    pub fn set_color_scheme(&mut self, scheme: ColorScheme) {
        self.scheme = Some(scheme);
        let Some(follow) = &self.scheme_follow else {
            return;
        };
        let clear = match scheme {
            ColorScheme::Dark => follow.pair.borrow().dark_clear,
            ColorScheme::Light => follow.pair.borrow().light_clear,
        };
        follow.scheme.set(scheme);
        self.set_clear_color(clear);
    }

    /// Makes springs show every new value at once, or move again.
    ///
    /// With reduced motion, a node with a
    /// [`transition`](crate::element::Element::transition) paints a changed
    /// value in the next frame, as a node without one does. The windowing
    /// layer reports the system's own setting through
    /// [`App::set_system_reduced_motion`], and a call here overrides it for
    /// as long as the app runs.
    ///
    /// ```
    /// use smudge_core::app::{App, AppConfig, RepaintMode};
    /// use smudge_core::geometry::{Color, Size};
    /// use smudge_core::widget::panel;
    ///
    /// let config = AppConfig {
    ///     viewport: Size {
    ///         width: 120.0,
    ///         height: 80.0,
    ///     },
    ///     scale: 1.0,
    ///     clear_color: Color::WHITE,
    ///     repaint: RepaintMode::Damage,
    /// };
    /// let mut app = App::new(config, panel);
    /// app.set_reduced_motion(true);
    /// app.set_system_reduced_motion(false);
    /// assert!(app.reduced_motion());
    /// ```
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion_app = Some(reduced);
        self.tree.set_reduced_motion(self.reduced_motion());
    }

    /// Reports whether the system asks for reduced motion.
    ///
    /// The windowing layer calls it at start and whenever the setting
    /// changes. It takes effect unless the app chose for itself through
    /// [`App::set_reduced_motion`].
    pub fn set_system_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion_system = reduced;
        self.tree.set_reduced_motion(self.reduced_motion());
    }

    /// Whether springs show every new value at once: the app's own choice
    /// when it made one, and the system's setting otherwise.
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion_app
            .unwrap_or(self.reduced_motion_system)
    }

    /// Whether a spring is still moving, so the next frame has motion to
    /// paint.
    ///
    /// A window keeps drawing paced frames while it is true, as it does for
    /// a tick, and goes idle once every spring has arrived.
    pub fn is_animating(&self) -> bool {
        self.tree.is_animating()
    }

    /// Whether the app follows the system color scheme.
    ///
    /// The windowing layer starts the watcher that reports scheme changes
    /// only when it does, so an app that shows no interest holds no
    /// connection to the desktop.
    pub fn follows_color_scheme(&self) -> bool {
        self.scheme_follow.is_some()
    }

    /// Changes the color the surface starts from, and makes the next frame
    /// repaint the whole surface, since no pixel's paint names it.
    fn set_clear_color(&mut self, color: Color) {
        self.config.clear_color = color;
        self.force_full = true;
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

    /// Sets the callback that a message from another thread runs to ask for
    /// the frame that applies it.
    ///
    /// The platform installs one that posts a wake event to the event loop,
    /// so a thread that sends does not have to poll and the window does not
    /// wake for anything else. A headless app needs none: its next
    /// [`App::frame`] call finds the values on its own. Channels created
    /// before this call get the waker too, and later ones start with it.
    pub fn set_remote_waker(&self, waker: impl Fn() + Send + Sync + 'static) {
        crate::channel::install_waker(&self.channels, std::sync::Arc::new(waker));
    }

    /// Ends the held press as if it had been cancelled, without running the
    /// pressed node's `on_press`.
    ///
    /// The windowing layer calls this when the window system takes over the
    /// press, as it does for an interactive move or resize: the release of
    /// that press never reaches the app, so without this the press would stay
    /// captured and hover would stop following the pointer. The pressed node
    /// loses its pressed state and hears `on_pointer_up`, so a drag it started
    /// ends. A window system may still refuse the gesture after the call that
    /// starts it succeeded, and then the release arrives after all: a primary
    /// release that comes with no press held, before the next primary press,
    /// reaches no handler. With no press held this does nothing.
    pub fn release_press(&mut self) {
        self.tree.release_press();
    }

    /// Takes every window command queued so far, in the order the handlers
    /// queued them.
    ///
    /// The windowing layer calls this after it hands an input event over, so
    /// a command starts its move or resize while the press that queued it is
    /// still being handled. A headless application has no windowing layer and
    /// no need to take the commands, so they stay queued for a test to
    /// inspect. Taking them empties the queue.
    pub fn take_window_commands(&mut self) -> Vec<crate::window::WindowCommand> {
        crate::window::take_commands(&self.window)
    }

    /// Replaces the window state the application's
    /// [`maximized`](crate::window::maximized) reads.
    ///
    /// The windowing layer calls this when the window's maximized state may
    /// have changed. An equal report changes nothing, so a binding that reads
    /// the state reruns only on a real change.
    pub fn set_window_state(&mut self, state: crate::window::WindowState) {
        crate::window::set_state(&self.window, state);
    }

    /// Returns the cursor the pointer should show.
    ///
    /// This is the cursor of the deepest enabled node under the pointer that
    /// set one with the [`cursor`](crate::element::Element::cursor) builder,
    /// or [`Cursor::Default`](crate::window::Cursor::Default) when no node
    /// under the pointer sets one or the pointer left the window. While a
    /// press is held, the walk starts at the pressed node instead. The
    /// windowing layer reads it after every input event and every frame and
    /// shows it on the window.
    pub fn cursor(&self) -> crate::window::Cursor {
        self.tree.cursor()
    }

    /// Sets a callback that runs once at the start of every frame with the
    /// current time.
    ///
    /// The time comes from the app's [`Clock`], the same one its timers
    /// read, so a test that runs the app on a virtual clock drives both.
    pub fn set_tick(&mut self, tick: impl FnMut(Instant) + 'static) {
        self.tick = Some(Box::new(crate::hot::wrap1_mut(tick)));
    }

    /// Runs the hot patch hooks of the app, inside the owners they belong
    /// to.
    ///
    /// This is for the windowing layer, which runs it after it applied a
    /// patch and remounted, so an app's hooks restart the workers they
    /// own. Applications do not call it.
    #[cfg(all(feature = "hot-reload", target_os = "linux"))]
    pub fn run_hot_patch_hooks(&self) {
        crate::hot::run_hot_patch_hooks(&self.hooks);
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
    /// callback never hears of it. A message from another thread also leaves
    /// such work: the values its channel queues are the frame's to deliver.
    /// A platform calls this after handing events over and asks for a frame
    /// when it is `true`. Most pointer moves change
    /// nothing, and asking for a frame every time would present the window
    /// for nothing.
    pub fn needs_frame(&self) -> bool {
        // An open offer of the old build's values ends only with frames, so
        // an idle app draws the few it takes rather than keep them.
        #[cfg(all(feature = "hot-reload", target_os = "linux"))]
        if self.carrying.is_some() {
            return true;
        }
        self.tree.has_pending_work() || crate::channel::has_pending(&self.channels)
    }

    /// Handles one input event between frames.
    ///
    /// The pointer events are routed to the tree: a move moves the hovered
    /// chain, a primary press captures the node under the pointer and focuses
    /// it when it can take the focus, and a release runs the captured node's
    /// `on_press` when it happens over it. A press elsewhere clears the focus,
    /// except while a focus trap is mounted and the press lands outside it,
    /// and any press turns the keyboard navigation mode off. A wheel scroll
    /// goes to the deepest node in the hovered chain that has a wheel handler.
    /// Key events are routed to the focused node and bubble to its ancestors
    /// until a handler takes them, starting at the root when nothing anchors
    /// the key and at the active focus trap when the key would otherwise
    /// start outside it, and an unhandled Tab or Shift+Tab moves the focus,
    /// staying inside the active focus trap when one is mounted. A change of
    /// the window's input focus clears a composition and updates what
    /// [`window::focused`](crate::window::focused) reads, while the focused
    /// node keeps the focus and hears no `on_focus`. Handlers run at once,
    /// inside their node's owner, and a handler that writes a signal makes
    /// the change visible with the next call to [`App::frame`]. The app
    /// also records the pointer position from pointer events and the keyboard
    /// modifiers from key and modifiers events.
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
                crate::window::set_focused(&self.window, focused);
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
    /// focus, and Tab moves it. While a focus trap is mounted, a press
    /// outside the trap leaves the focus where it is, and Tab moves it within
    /// the trap only. Removing the focused node clears the focus, and
    /// removing or hiding a trap returns the focus to the node that held it
    /// when the trap mounted or last showed.
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
    /// while nothing is focused, and while the focused node sits outside an
    /// active focus trap. A window presents the area to the
    /// operating system so the input method can anchor its candidate window to
    /// it.
    pub fn ime_area(&self) -> Option<Rect> {
        self.tree.ime_area()
    }

    /// Returns whether the input method has to start over, and clears the
    /// request.
    ///
    /// [`App::remount`] replaces every text field, so a composition the
    /// operating system still holds belongs to an editor that no longer
    /// exists, and committing it would write into its replacement. A window
    /// that reads `true` turns the input method off for a moment and drops
    /// the composition events that arrive meanwhile, before it reports the
    /// next [`App::ime_area`]. On Wayland that drops the composition. On
    /// X11 a late composition event can still arrive after it, and macOS
    /// keeps the composition when its input method turns off.
    pub fn take_ime_reset(&mut self) -> bool {
        std::mem::take(&mut self.ime_reset)
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
    /// 1. The values other threads sent through the app's channels: each
    ///    handler runs inside the owner that created its channel, delivering
    ///    at most what the queue held when the drain reached it.
    /// 2. The timers that are due on the app's clock, then the nodes whose
    ///    springs were still moving, which repaint, then the tick callback,
    ///    when one is set.
    /// 3. Reactive effects and the updates they queue, until neither has work
    ///    left. A signal write therefore lands before layout, including one a
    ///    channel handler or a timer made at the start of the frame.
    /// 4. The layout pass, which runs the layout engine only when a node needs
    ///    it.
    /// 5. The paint pass, which regenerates the operations of the nodes that
    ///    changed and reports their old and new bounds as damage.
    /// 6. The display list, rebuilt only when something may have changed.
    /// 7. Damage conversion and merging, then the output.
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
        // The frame runs every effect its channels, its timers, its tick and
        // its updates wake before it lays out, so reporting those wakes would
        // only ask the platform for a second frame with nothing to do.
        let quiet = QuietWakes::enter(&self._root);
        crate::channel::drain_all(&self.channels);
        timer::fire_due(&self.timers);
        self.tree.queue_animations();

        let now = self.now();
        self.tree.set_now(now);
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

        #[cfg(all(feature = "hot-reload", target_os = "linux"))]
        if self
            .carrying
            .as_mut()
            .is_some_and(|carrying| !carrying.frame_ended())
        {
            self.carrying = None;
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

    /// Projects every visible node that reports a role, in paint order.
    ///
    /// This is the tree an accessibility backend reads. A node is projected
    /// when its element set a role or its widget reports one; the children
    /// of a node without a role attach to the nearest projected ancestor.
    /// A hidden node and its subtree are skipped, because they paint
    /// nothing. The bounds are in logical pixels, as the last layout pass
    /// left them.
    pub fn semantics_nodes(&self) -> Vec<SemanticsNode> {
        self.tree.semantics_nodes()
    }

    /// Returns the projected nodes that no projected ancestor holds, in paint
    /// order.
    ///
    /// These are the nodes an accessibility backend attaches to the root it
    /// adds for the window.
    pub fn semantics_roots(&self) -> Vec<WidgetId> {
        self.tree.semantics_roots()
    }

    /// Projects one node with the children the projection gives it.
    ///
    /// Returns [`None`] when the id is stale, when the node or an ancestor
    /// is hidden, or when neither the element nor the widget reports a role
    /// for it.
    pub fn semantics_node(&self, id: WidgetId) -> Option<SemanticsNode> {
        self.tree.semantics_node(id)
    }

    /// Returns whether the projection has unread changes.
    ///
    /// A backend that only rebuilds when this returns true skips frames that
    /// changed nothing projected.
    pub fn has_a11y_changes(&self) -> bool {
        self.tree.has_a11y_changes()
    }

    /// Takes the projected nodes that changed since the last call.
    ///
    /// A change to a node the projection leaves out is reported as its
    /// nearest projected ancestor, whose child list it changed, or as
    /// [`A11yChanges::roots_changed`] when no ancestor projects. A repaint
    /// of such a node, such as a caret's blink, changes nothing projected
    /// and is not reported. Stale and
    /// hidden ids drop out, and a frame that changed too much sets
    /// [`A11yChanges::reset`] instead of listing every node, so a backend can
    /// rebuild from scratch. Take these only when no frame is pending: a
    /// node that changes again while it waits for a frame is not queued
    /// twice, so changes taken before that frame miss what it does.
    pub fn take_a11y_changes(&mut self) -> A11yChanges {
        self.tree.take_a11y_changes()
    }

    /// Runs the press handler of `id`, as an accessibility action would.
    ///
    /// This is the same path a press of Enter or Space takes, so a disabled
    /// or hidden node runs nothing and `false` is returned. `true` means a
    /// handler ran.
    pub fn press(&mut self, id: WidgetId) -> bool {
        self.tree.run_press(id)
    }

    /// Moves the keyboard focus to `id`, as an accessibility action would.
    ///
    /// A node that is not focusable, or that cannot hold the focus because
    /// it is disabled or hidden, is left alone and `false` is returned. A
    /// node that already holds the focus keeps it and `true` is returned.
    /// Unlike the autofocus request, an active focus trap does not confine
    /// this one: the trap binds Tab movement and pointer presses, not an
    /// explicit request that names its node. When the node holds the focus
    /// afterwards, the next frame scrolls it into view as
    /// [`Element::on_reveal`](crate::element::Element::on_reveal) describes.
    pub fn request_focus(&mut self, id: WidgetId) -> bool {
        self.tree.request_focus(id)
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
