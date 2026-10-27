//! The window, its run loop and the frames it presents.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use smudge_core::app::{App, AppConfig, FrameOutput, FrameStats, RepaintMode};
use smudge_core::clipboard::{Clipboard, MemoryClipboard};
use smudge_core::element::IntoElement;
use smudge_core::geometry::{Color, PixelRect, Rect, Size};
use smudge_cpu::CpuRenderer;
use softbuffer::{Context, Surface};
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::{Window, WindowId};

#[cfg(not(target_arch = "wasm32"))]
use crate::clipboard::SystemClipboard;
use crate::error::PlatformError;
use crate::events::EventTranslator;
use crate::present::{
    DamageHistory, PresentStats, PresentStatsHandle, copy_rect, damage_region, present_skipped,
    tint_rects,
};
use crate::renderer::RendererChoice;

/// The user event the run loop carries, of which the accessibility adapter
/// sends the only kind.
///
/// With the `a11y` feature off the run loop carries `()` and nothing is ever
/// sent.
#[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
pub(crate) enum PlatformEvent {
    /// An event from the accessibility adapter: a request for the initial
    /// tree, an action a user ran in an assistive tool, or a notice that the
    /// platform stopped reading the tree.
    A11y(accesskit_winit::Event),
}

#[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
impl From<accesskit_winit::Event> for PlatformEvent {
    fn from(event: accesskit_winit::Event) -> PlatformEvent {
        PlatformEvent::A11y(event)
    }
}

/// The user event type the run loop is built with.
#[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
type UserEvent = PlatformEvent;
#[cfg(any(not(feature = "a11y"), target_arch = "wasm32"))]
type UserEvent = ();

/// The measurement hook that receives one pair of statistics per presented
/// frame.
///
/// The platform calls it after each present with the frame's pipeline
/// statistics and what the present did. Nothing in the library sets it: it
/// exists so a benchmark can record one entry per frame while the frames run.
pub type FrameHook = Rc<dyn Fn(FrameStats, PresentStats)>;

/// The settings a window opens with.
#[derive(Clone)]
pub struct WindowOptions {
    /// The text shown in the window's title bar.
    pub title: String,
    /// The size of the window's content area in logical pixels.
    pub size: Size,
    /// The color the renderer fills the surface with before it replays the
    /// display list.
    ///
    /// Must be opaque, because presentation copies only the color channels of
    /// the surface into the window. A run with a color that is not opaque
    /// returns [`PlatformError::ClearColor`] before a window opens.
    pub clear_color: Color,
    /// How much of the surface each frame repaints.
    pub repaint: RepaintMode,
    /// Whether every present tints the pixels the frame repainted.
    ///
    /// A debugging aid for damage regressions: while it is on, every present
    /// copies the whole pixmap and blends a translucent tint over the frame's
    /// damage rectangles in the window buffer alone, so the window shows what
    /// each frame paid for. Damage presentation is off while the overlay is
    /// on, and neither the renderer's pixmap nor a later frame keeps a tint.
    ///
    /// The overlay exists on the CPU renderer only, so it makes
    /// [`RendererChoice::Auto`] pick the CPU renderer, and it cannot be
    /// combined with [`RendererChoice::Gpu`].
    pub show_damage: bool,
    /// Which renderer presents the frames.
    ///
    /// [`RendererChoice::Auto`] uses the GPU renderer only when the window's
    /// adapter is a real GPU, and falls back to the CPU renderer on a software
    /// rasterizer, without one, or on a window the GPU cannot present to. It
    /// is the default in builds with the `gpu` feature. [`RendererChoice::Gpu`]
    /// never falls back: it fails the run when the `gpu` feature is off or no
    /// usable adapter exists.
    ///
    /// The [`crate::renderer::RENDERER_ENVIRONMENT`] variable, set to `cpu`,
    /// `gpu` or `auto`, overrides the value here when the window opens, so a
    /// machine can switch renderer without a rebuild.
    pub renderer: RendererChoice,
    /// Where the platform publishes presentation statistics, when given.
    ///
    /// Pass a [`PresentStatsHandle`] and keep the original to read the
    /// [`PresentStats`] of the most recent present while the window runs.
    /// Without one the platform skips publishing.
    pub stats: Option<PresentStatsHandle>,
    /// Measurement hook: runs after every presented frame.
    ///
    /// The hook receives the frame's [`FrameStats`] and the present's
    /// [`PresentStats`], in that order. It runs on the main thread between
    /// frames and must not touch the window. A frame rendered while the
    /// surface has no pixel, for example while the window is minimized, is not
    /// presented, so it neither calls the hook nor counts toward the frame
    /// limit.
    pub on_frame: Option<FrameHook>,
    /// Measurement hook: closes the window after this many presented frames.
    ///
    /// A run that must last a fixed number of frames sets this instead of
    /// waiting for the user to close the window.
    pub frame_limit: Option<u64>,
}

impl std::fmt::Debug for WindowOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowOptions")
            .field("title", &self.title)
            .field("size", &self.size)
            .field("clear_color", &self.clear_color)
            .field("repaint", &self.repaint)
            .field("show_damage", &self.show_damage)
            .field("renderer", &self.renderer)
            .field("stats", &self.stats)
            .field("on_frame", &self.on_frame.is_some())
            .field("frame_limit", &self.frame_limit)
            .finish()
    }
}

/// Opens a window and runs frames until the window closes.
///
/// `build` runs once, when the window first appears, and mounts the element it
/// returns. A frame runs after a reactive signal wakes an effect, after a
/// resize or a scale change, and once per frame while a tick is installed.
/// Otherwise the window waits for the next event.
///
/// Call this from the main thread. The event loop takes over the calling
/// thread until the window closes.
///
/// Platform-specific:
///
/// - **Web:** the loop runs on the browser's own event loop, so this returns
///   `Ok(())` as soon as the loop is started, before the window opens. See
///   [`run_with`] for the details.
///
/// # Errors
///
/// Returns [`PlatformError::ClearColor`] when [`WindowOptions::clear_color`]
/// is not opaque, [`PlatformError::Renderer`] when the
/// [`WindowOptions::renderer`] choice or its environment override cannot be
/// used, [`PlatformError::EventLoop`] when the event loop cannot be
/// created or run, [`PlatformError::Window`] when the window cannot be
/// created, and [`PlatformError::Surface`] when the presentation surface
/// cannot be created, resized or presented.
///
/// # Examples
///
/// ```no_run
/// use smudge_core::app::RepaintMode;
/// use smudge_core::geometry::{Color, Size};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_platform::{RendererChoice, WindowOptions, run};
///
/// run(
///     WindowOptions {
///         title: String::from("Smudge"),
///         size: Size {
///             width: 320.0,
///             height: 200.0,
///         },
///         clear_color: Color::WHITE,
///         repaint: RepaintMode::Damage,
///         show_damage: false,
///         renderer: RendererChoice::default(),
///         stats: None,
///         on_frame: None,
///         frame_limit: None,
///     },
///     || panel().width(px(120.0)).height(px(40.0)).background(Color::BLACK),
/// )?;
/// # Ok::<(), smudge_platform::PlatformError>(())
/// ```
pub fn run<E: IntoElement + 'static>(
    options: WindowOptions,
    build: impl FnOnce() -> E + 'static,
) -> Result<(), PlatformError> {
    run_with(options, build, |_| {})
}

/// Opens a window, hands the app to `configure`, and runs frames until the
/// window closes.
///
/// Behaves like [`run`], and calls `configure` once after `build` has mounted
/// its element and before the first frame. Use it to install a per-frame
/// driver with [`App::set_tick`], or to set up measurement that must run
/// alongside the frames.
///
/// Platform-specific:
///
/// - **Web:** the window is a canvas appended to the page's body, at
///   [`WindowOptions::size`] unless the page's style sheet sizes it with
///   `!important`, and it follows the size the page gives it. The loop runs
///   on the browser's own event loop, so this returns `Ok(())` as soon as
///   the loop is started, before the window opens. Only
///   failures before that point, such as a clear color that is not opaque,
///   come back as an error; a failure later in the run stops the loop
///   without a way to report it.
///
/// # Errors
///
/// Returns [`PlatformError::ClearColor`] when [`WindowOptions::clear_color`]
/// is not opaque, checked before the event loop is created,
/// [`PlatformError::Renderer`] when the [`WindowOptions::renderer`] choice
/// or its environment override cannot be used, and otherwise the same errors
/// as [`run`].
///
/// # Examples
///
/// ```no_run
/// use smudge_core::app::RepaintMode;
/// use smudge_core::geometry::{Color, Size};
/// use smudge_core::reactive::{self, prelude::*};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_platform::{RendererChoice, WindowOptions, run_with};
///
/// let (color, set_color) = reactive::signal_local(Color::BLACK);
/// run_with(
///     WindowOptions {
///         title: String::from("Smudge"),
///         size: Size {
///             width: 320.0,
///             height: 200.0,
///         },
///         clear_color: Color::WHITE,
///         repaint: RepaintMode::Damage,
///         show_damage: false,
///         renderer: RendererChoice::default(),
///         stats: None,
///         on_frame: None,
///         frame_limit: None,
///     },
///     move || panel().width(px(120.0)).background(move || color.get().into()),
///     move |app| {
///         let mut dark = false;
///         app.set_tick(move |_| {
///             dark = !dark;
///             set_color.set(if dark { Color::BLACK } else { Color::WHITE });
///         });
///     },
/// )?;
/// # Ok::<(), smudge_platform::PlatformError>(())
/// ```
pub fn run_with<E: IntoElement + 'static>(
    options: WindowOptions,
    build: impl FnOnce() -> E + 'static,
    configure: impl FnOnce(&mut App) + 'static,
) -> Result<(), PlatformError> {
    if !options.clear_color.is_opaque() {
        return Err(PlatformError::ClearColor);
    }
    let renderer = options.renderer.with_env_override()?;

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(PlatformError::EventLoop)?;
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    let proxy = event_loop.create_proxy();
    // The web loop takes the state by value, so nothing mutates it here.
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut platform = Platform {
        renderer,
        options,
        build: Some(build),
        configure: Some(Box::new(configure)),
        state: None,
        error: None,
        #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
        proxy,
    };
    // The web loop runs on the browser's own event loop and never runs to
    // completion, so it is started and can no longer return an error. A
    // failure after it started only stops the run.
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(platform);
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        event_loop
            .run_app(&mut platform)
            .map_err(PlatformError::EventLoop)?;
        match platform.error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// The state the event loop drives.
struct Platform<E, B>
where
    E: IntoElement,
    B: FnOnce() -> E,
{
    /// The renderer the window presents with, after the environment override.
    renderer: RendererChoice,
    /// The settings the window opens with.
    options: WindowOptions,
    /// The element to mount, until the window first appears.
    build: Option<B>,
    /// The caller's setup, run once the app exists.
    configure: Option<Configure>,
    /// The window, its app and its renderer, once the window exists.
    state: Option<WindowState>,
    /// The first error that stopped the run.
    error: Option<PlatformError>,
    /// The sender the accessibility adapter posts its events through.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
}

/// The setup a caller hands to [`run_with`], boxed so the platform state does
/// not become generic over it.
type Configure = Box<dyn FnOnce(&mut App)>;

/// One live window and everything it presents.
struct WindowState {
    /// The window the surface presents to.
    ///
    /// The shared handle is an `Arc` because the wgpu surface target must be
    /// sendable; everything else in the window stays on the main thread.
    window: Arc<Window>,
    /// The app whose frames the window runs.
    app: App,
    /// The translator that turns window events into input events.
    translator: EventTranslator,
    /// The renderer and presentation path the frames go through.
    presenter: Presenter,
    /// What the last present did.
    stats: PresentStats,
    /// Where to publish the statistics, when the caller asked for them.
    stats_handle: Option<PresentStatsHandle>,
    /// The measurement hook to call after every presented frame.
    on_frame: Option<FrameHook>,
    /// How many frames to present before the window closes.
    frame_limit: Option<u64>,
    /// How many frames the window has presented.
    frames_presented: u64,
    /// The number of physical pixels per logical pixel the app last used.
    scale: f32,
    /// The window's size in physical pixels when the app last used it.
    physical: (u32, u32),
    /// The IME area the window last reported, in logical pixels, or `None`
    /// while no widget reports one.
    ime_area: Option<Rect>,
    /// The scale the IME area was last reported under. The window system
    /// turns the area into physical pixels with the scale of the moment, so
    /// an unchanged logical area is reported again after a scale change.
    ime_scale: f32,
    /// Whether the window currently accepts input method input.
    ime_allowed: bool,
    /// The bridge that maps the app's accessibility projection onto the
    /// platform's, when the platform builds with accessibility.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    adapter: accesskit_winit::Adapter,
    /// The projection that builds the updates the adapter sends.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    a11y: smudge_a11y::Projection,
    /// Whether the accessibility backend has asked for the tree and has not
    /// stopped reading it since. While it is false, no projection work runs.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    a11y_active: bool,
    /// Whether the next update must carry the whole tree, because the
    /// backend asked for it or the scale moved every node's bounds.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    a11y_full: bool,
}

/// The frame that was rendered and waits to be presented.
#[derive(Default)]
struct PendingFrame {
    /// The rectangles the frame changed, in physical pixels.
    damage: Vec<PixelRect>,
    /// Whether the frame repainted the whole surface.
    full: bool,
}

impl<E, B> Platform<E, B>
where
    E: IntoElement + 'static,
    B: FnOnce() -> E + 'static,
{
    /// Creates the window, its app and its presentation path.
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<WindowState, PlatformError> {
        let attributes = Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(LogicalSize::new(
                f64::from(self.options.size.width),
                f64::from(self.options.size.height),
            ))
            // The accessibility adapter must be attached before the window is
            // shown, so the window starts hidden and is shown right after.
            .with_visible(false);
        // A web window draws into a canvas, which the page shows only once it
        // is part of the document.
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes.with_append(true)
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(PlatformError::Window)?,
        );
        #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
        let adapter = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        );
        window.set_visible(true);

        let scale = window.scale_factor() as f32;
        let physical = window.inner_size();
        let build = self
            .build
            .take()
            .expect("the window is created only once, so its element is still waiting");
        let mut app = App::new(
            AppConfig {
                viewport: logical_size(physical, scale),
                scale,
                clear_color: self.options.clear_color,
                repaint: self.options.repaint,
            },
            build,
        );
        if let Some(configure) = self.configure.take() {
            configure(&mut app);
        }

        let (width, height) = app.surface_size();
        let presenter = create_presenter(
            &window,
            self.renderer,
            width,
            height,
            self.options.show_damage,
        )?;

        let weak = Arc::downgrade(&window);
        app.set_wake_callback(move || {
            if let Some(window) = weak.upgrade() {
                window.request_redraw();
            }
        });
        window.request_redraw();

        // The app gets the system clipboard when it can be opened, and an
        // in-memory one when it cannot, so cut, copy and paste keep working
        // inside the app either way. The web page has no system clipboard to
        // open, so it always gets the in-memory one.
        #[cfg(not(target_arch = "wasm32"))]
        let clipboard: Box<dyn Clipboard> = match SystemClipboard::new() {
            Ok(clipboard) => Box::new(clipboard),
            Err(_) => Box::new(MemoryClipboard::default()),
        };
        #[cfg(target_arch = "wasm32")]
        let clipboard: Box<dyn Clipboard> = Box::new(MemoryClipboard::default());
        app.set_clipboard(clipboard);

        Ok(WindowState {
            window,
            app,
            translator: EventTranslator::new(),
            presenter,
            stats: PresentStats::default(),
            stats_handle: self.options.stats.clone(),
            on_frame: self.options.on_frame.take(),
            frame_limit: self.options.frame_limit,
            frames_presented: 0,
            scale,
            physical: (physical.width, physical.height),
            ime_area: None,
            ime_scale: scale,
            ime_allowed: false,
            #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
            adapter,
            #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
            a11y: smudge_a11y::Projection::new(self.options.title.clone()),
            #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
            a11y_active: false,
            #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
            a11y_full: false,
        })
    }

    /// Changes the size and the scale the app and the renderer work at.
    fn resize(&mut self, physical: PhysicalSize<u32>, scale: f64) -> Result<(), PlatformError> {
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        let scale = scale as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return Ok(());
        }
        if physical.width == 0 || physical.height == 0 {
            return Ok(());
        }
        if state.scale == scale && state.physical == (physical.width, physical.height) {
            return Ok(());
        }

        // The app learns the new viewport only after the presenter took the
        // new size, so a failed resize leaves the presenter and the app
        // consistent at the old size instead of an app whose next frame
        // reaches past the surface it renders onto.
        let (Some(width), Some(height)) = (
            NonZeroU32::new(physical.width),
            NonZeroU32::new(physical.height),
        ) else {
            return Ok(());
        };
        state.presenter.resize(width, height)?;
        state.app.set_viewport(logical_size(physical, scale), scale);
        #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
        if state.scale != scale {
            state.a11y_full = true;
        }
        debug_assert_eq!(
            state.app.surface_size(),
            (width.get(), height.get()),
            "the app's surface size must match the resized surface and renderer"
        );
        state.scale = scale;
        state.physical = (physical.width, physical.height);
        state.window.request_redraw();
        Ok(())
    }

    /// Runs one frame and presents it.
    fn redraw(&mut self) -> Result<(), PlatformError> {
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        let tick = state.app.has_tick();
        let output = state.app.frame();
        let presented = state.presenter.render_and_present(&output, tick)?;
        if let Some(stats) = presented {
            state.stats = stats;
            if let Some(handle) = &state.stats_handle {
                handle.store(stats);
            }
            state.frames_presented += 1;
            if let Some(on_frame) = &state.on_frame {
                on_frame(state.app.stats().clone(), stats);
            }
        }
        state.update_ime();
        #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
        state.update_a11y();
        if tick {
            state.window.request_redraw();
        }
        Ok(())
    }

    /// Runs the timers that are due and puts the event loop to sleep until
    /// the next one.
    ///
    /// An app with a tick keeps itself awake and needs nothing here. Any
    /// other app runs its due timers between frames: one that writes a
    /// signal wakes the window through the app's wake callback, and one that
    /// changes nothing draws no frame, so it never costs a present. The loop
    /// then waits until the soonest timer, or for events when none is left.
    /// A timer that is due again at once, such as one a callback just
    /// scheduled with no delay, asks for a frame, which runs it.
    fn wait_for_timers(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.app.has_tick() {
            return;
        }
        if wait_decision(state.app.next_deadline(), Instant::now()) == WaitDecision::Now {
            state.app.run_due_timers();
            if state.app.needs_frame() {
                state.window.request_redraw();
            }
        }
        match wait_decision(state.app.next_deadline(), Instant::now()) {
            WaitDecision::Now => state.window.request_redraw(),
            WaitDecision::Until(deadline) => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
            WaitDecision::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// Records `error` and stops the event loop.
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: PlatformError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
        event_loop.exit();
    }

    /// Answers an event the accessibility adapter posted.
    ///
    /// The first request for the tree is answered with a full update, which
    /// is also what turns the projection on: until the backend asks, no
    /// frame spends any work on it. A request that arrives while a frame is
    /// pending waits for that frame, so the tree it gets has been laid out
    /// and every later change is queued after it. An action presses or focuses the widget
    /// the request names, through the same paths a pointer press and a
    /// keyboard focus change take. When the platform stops reading the tree,
    /// the projection goes quiet until the backend asks again.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    fn handle_a11y_event(&mut self, event: accesskit_winit::Event) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                state.a11y_active = true;
                state.a11y_full = true;
                if state.app.needs_frame() {
                    state.window.request_redraw();
                } else {
                    state.update_a11y();
                }
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => {
                let id = smudge_core::tree::WidgetId::from_u64(request.target_node.0);
                match request.action {
                    accesskit::Action::Click => {
                        state.app.press(id);
                    }
                    accesskit::Action::Focus => {
                        state.app.request_focus(id);
                    }
                    _ => {}
                }
                if state.app.needs_frame() {
                    state.window.request_redraw();
                }
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                state.a11y_active = false;
            }
        }
    }
}

impl<E, B> ApplicationHandler<UserEvent> for Platform<E, B>
where
    E: IntoElement + 'static,
    B: FnOnce() -> E + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.create(event_loop) {
            Ok(state) => self.state = Some(state),
            Err(error) => self.fail(event_loop, error),
        }
    }

    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let UserEvent::A11y(event) = event;
        self.handle_a11y_event(event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // The accessibility adapter sees every event before the app does,
        // because it reacts to moves, resizes and focus changes itself.
        #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
        if let Some(state) = self.state.as_mut() {
            state.adapter.process_event(&state.window, &event);
        }
        let result = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                Ok(())
            }
            WindowEvent::Resized(physical) => {
                let scale = self
                    .state
                    .as_ref()
                    .map_or(1.0, |state| state.window.scale_factor());
                self.resize(physical, scale)
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                match self.state.as_ref().map(|state| state.window.inner_size()) {
                    Some(physical) => self.resize(physical, scale_factor),
                    None => Ok(()),
                }
            }
            WindowEvent::RedrawRequested => {
                let result = self.redraw();
                if result.is_ok() && self.state.as_ref().is_some_and(WindowState::finished) {
                    event_loop.exit();
                }
                result
            }
            other => {
                if let Some(state) = self.state.as_mut() {
                    let input = state.translator.translate(&other, state.scale);
                    if let Some(input) = input {
                        state.app.handle_event(input);
                        if state.app.needs_frame() {
                            state.window.request_redraw();
                        }
                    }
                    state.update_ime();
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.wait_for_timers(event_loop);
    }
}

impl WindowState {
    /// Reports the area the focused widget offers the input method to the
    /// window system.
    ///
    /// This runs after every window event and after every presented frame,
    /// because the area can move without a pixel changing anywhere: the
    /// window asks for a frame only when the tree has work that changes
    /// pixels, so a caret move inside composed text would otherwise leave
    /// the input method's candidate window where it was. A widget that
    /// reports no area turns the input method off, which is what a window
    /// does while no text field holds the focus.
    fn update_ime(&mut self) {
        let area = self.app.ime_area();
        if area == self.ime_area && self.scale == self.ime_scale {
            return;
        }
        self.ime_area = area;
        self.ime_scale = self.scale;
        match area {
            Some(area) => {
                if !self.ime_allowed {
                    self.ime_allowed = true;
                    self.window.set_ime_allowed(true);
                }
                self.window.set_ime_cursor_area(
                    LogicalPosition::new(f64::from(area.x0), f64::from(area.y0)),
                    LogicalSize::new(f64::from(area.width()), f64::from(area.height())),
                );
            }
            None => {
                if self.ime_allowed {
                    self.ime_allowed = false;
                    self.window.set_ime_allowed(false);
                }
            }
        }
    }

    /// Sends the accessibility projection to the adapter when the backend is
    /// reading the tree and something changed.
    ///
    /// This runs after every frame. The update is whole when the backend
    /// asked for the tree or the scale changed, and it is also whole when
    /// the changes say a frame changed too much to describe one node at a
    /// time. While the backend is not reading, or while nothing projected
    /// changed, this builds nothing, and the adapter calls the closure that
    /// builds an update only while an assistive technology listens.
    #[cfg(all(feature = "a11y", not(target_arch = "wasm32")))]
    fn update_a11y(&mut self) {
        if !self.a11y_active || !(self.a11y_full || self.app.has_a11y_changes()) {
            return;
        }
        let full = std::mem::take(&mut self.a11y_full);
        let (width, height) = self.physical;
        let (projection, app, scale) = (&mut self.a11y, &mut self.app, self.scale);
        self.adapter.update_if_active(|| {
            projection.update(app, f64::from(width), f64::from(height), scale, full)
        });
    }

    /// Returns whether the window has presented the number of frames its frame
    /// limit sets, or `false` when no limit is set.
    fn finished(&self) -> bool {
        self.frame_limit
            .is_some_and(|limit| self.frames_presented >= limit)
    }
}

/// Builds the presentation path the window opens with.
///
/// The GPU path asks the window system for an adapter when the choice can use
/// one. An automatic choice falls back on the CPU renderer whenever that
/// fails or finds only a software rasterizer; an explicit GPU request fails
/// the run instead.
fn create_presenter(
    window: &Arc<Window>,
    choice: RendererChoice,
    width: u32,
    height: u32,
    show_damage: bool,
) -> Result<Presenter, PlatformError> {
    #[cfg(not(feature = "gpu"))]
    {
        match choice {
            RendererChoice::Gpu => Err(PlatformError::Renderer(String::from(
                "the GPU renderer needs a build with the gpu feature",
            ))),
            _ => Ok(Presenter::Cpu(Box::new(CpuPresenter::new(
                window,
                width,
                height,
                show_damage,
            )?))),
        }
    }
    #[cfg(feature = "gpu")]
    match choice {
        RendererChoice::Cpu => Ok(Presenter::Cpu(Box::new(CpuPresenter::new(
            window,
            width,
            height,
            show_damage,
        )?))),
        // The overlay tints the window buffer of the CPU path, so an
        // automatic choice keeps the CPU renderer while it is on.
        RendererChoice::Auto if show_damage => Ok(Presenter::Cpu(Box::new(CpuPresenter::new(
            window,
            width,
            height,
            show_damage,
        )?))),
        RendererChoice::Gpu | RendererChoice::Auto => {
            let explicit = choice == RendererChoice::Gpu;
            if explicit && show_damage {
                return Err(PlatformError::Renderer(String::from(
                    "the damage overlay tints the window buffer of the CPU \
                     presentation path and cannot run on the GPU renderer",
                )));
            }
            match crate::gpu::GpuPath::new(window, width, height, explicit)? {
                Some(gpu) => Ok(Presenter::Gpu(Box::new(gpu))),
                None => Ok(Presenter::Cpu(Box::new(CpuPresenter::new(
                    window,
                    width,
                    height,
                    show_damage,
                )?))),
            }
        }
    }
}

/// The renderer and presentation path a window presents with.
///
/// Both paths are boxed: the wgpu handles of the GPU path and the pixmap state
/// of the CPU path dwarf a boxed pointer, and the enum only dispatches.
enum Presenter {
    /// The CPU renderer, presenting through the software surface.
    Cpu(Box<CpuPresenter>),
    /// The GPU renderer, presenting through a wgpu surface.
    #[cfg(feature = "gpu")]
    Gpu(Box<crate::gpu::GpuPath>),
}

impl Presenter {
    /// Renders the frame and presents it, returning the statistics of a
    /// presented frame.
    ///
    /// Returns `Ok(None)` when nothing was presented: a frame that changed
    /// nothing while a tick runs, or a surface without a pixel or texture to
    /// present, both of which the next redraw retries.
    fn render_and_present(
        &mut self,
        frame: &FrameOutput<'_>,
        has_tick: bool,
    ) -> Result<Option<PresentStats>, PlatformError> {
        match self {
            Presenter::Cpu(cpu) => cpu.render_and_present(frame, has_tick),
            #[cfg(feature = "gpu")]
            Presenter::Gpu(gpu) => gpu.render_and_present(frame, has_tick),
        }
    }

    /// Takes the window's new extent.
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> Result<(), PlatformError> {
        match self {
            Presenter::Cpu(cpu) => cpu.resize(width, height),
            #[cfg(feature = "gpu")]
            Presenter::Gpu(gpu) => gpu.resize(width.get(), height.get()),
        }
    }
}

/// The CPU renderer and the software surface it presents through.
struct CpuPresenter {
    /// The surface that holds the pixels the window shows.
    surface: Surface<SharedWindow, SharedWindow>,
    /// The window whose frames are presented, notified before every present.
    window: SharedWindow,
    /// The renderer that paints the app's damage into a pixmap.
    renderer: CpuRenderer,
    /// Whether every present tints the frame's damage in the window buffer.
    show_damage: bool,
    /// The damage of the frames the window presented, most recent first.
    history: DamageHistory,
    /// The frame that was rendered and waits to be presented.
    pending: PendingFrame,
    /// Scratch for the region the next present copies.
    region: Vec<PixelRect>,
    /// Scratch for the damage rectangles the next present reports.
    reported: Vec<softbuffer::Rect>,
}

impl CpuPresenter {
    /// Creates the surface and the renderer for a window of `width` by
    /// `height` physical pixels.
    ///
    /// A window without pixels gets a renderer without pixels, and nothing
    /// presents until a resize gives the window an extent.
    fn new(
        window: &Arc<Window>,
        width: u32,
        height: u32,
        show_damage: bool,
    ) -> Result<CpuPresenter, PlatformError> {
        let shared = SharedWindow(Arc::clone(window));
        let context = Context::new(shared.clone()).map_err(PlatformError::Surface)?;
        let mut surface = Surface::new(&context, shared).map_err(PlatformError::Surface)?;
        drop(context);
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            surface
                .resize(width, height)
                .map_err(PlatformError::Surface)?;
        }
        Ok(CpuPresenter {
            surface,
            window: SharedWindow(Arc::clone(window)),
            renderer: CpuRenderer::new(width, height),
            show_damage,
            history: DamageHistory::default(),
            pending: PendingFrame::default(),
            region: Vec::new(),
            reported: Vec::new(),
        })
    }

    /// Takes the window's new extent.
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> Result<(), PlatformError> {
        self.surface
            .resize(width, height)
            .map_err(PlatformError::Surface)?;
        self.renderer.resize(width.get(), height.get());
        Ok(())
    }

    /// Renders the frame into the pixmap and presents it.
    ///
    /// The frame's damage and the recorded damage of the frames still in a
    /// window buffer decide what has to be copied: when the buffer's age says
    /// which frame it holds and no full repaint happened since, only their
    /// union is copied and the present reports it as damage. Every other case
    /// copies the whole pixmap. While the damage overlay is on, every present
    /// copies the whole pixmap and blends the tint over the frame's damage in
    /// the window buffer, so no tint reaches the pixmap a later frame repaints
    /// from. It also presents a frame with no damage, so a tint never outlives
    /// the frame that caused it.
    ///
    /// Before every present, the window is told that a frame is about to
    /// show. That is the order winit needs on Wayland: it schedules a frame
    /// callback that paces the next redraw, and presenting without it lets
    /// frames outrun the compositor until a present waits forever for the
    /// buffer it cannot get.
    ///
    /// Returns the statistics of a presented frame, or `Ok(None)` when nothing
    /// was presented: a surface without pixels, or a frame with no damage
    /// while a tick runs. Such a frame changed nothing, so the window still
    /// shows the last present, no buffer is taken, and the frame limit does
    /// not count it. An empty frame without a tick is a redraw the window
    /// system asked for, such as an X11 window being shown again, and presents
    /// the whole pixmap to restore the window's contents.
    fn render_and_present(
        &mut self,
        frame: &FrameOutput<'_>,
        has_tick: bool,
    ) -> Result<Option<PresentStats>, PlatformError> {
        let (width, height) = (self.renderer.width(), self.renderer.height());
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let raster_start = Instant::now();
        self.renderer.render(frame);
        let raster = raster_start.elapsed();
        self.pending.damage.clear();
        self.pending.damage.extend_from_slice(frame.damage);
        self.pending.full = frame.full;
        if present_skipped(&self.pending.damage, has_tick, self.show_damage) {
            return Ok(None);
        }
        let acquire_start = Instant::now();
        let mut buffer = self.surface.buffer_mut().map_err(PlatformError::Surface)?;
        let acquire = acquire_start.elapsed();
        let present_start = Instant::now();
        debug_assert_eq!(
            buffer.len(),
            self.renderer.pixels().len() / 4,
            "the window buffer must hold one entry per rendered pixel"
        );
        let age = buffer.age();
        let with_damage = !self.show_damage
            && damage_region(
                age,
                &self.pending.damage,
                self.pending.full,
                &self.history,
                &mut self.region,
            );
        let mut bytes_copied = 0;
        if with_damage {
            self.reported.clear();
            for rect in &self.region {
                let (Some(rect_width), Some(rect_height)) =
                    (NonZeroU32::new(rect.width), NonZeroU32::new(rect.height))
                else {
                    continue;
                };
                bytes_copied += u64::from(rect.width) * u64::from(rect.height) * 4;
                self.reported.push(softbuffer::Rect {
                    x: rect.x,
                    y: rect.y,
                    width: rect_width,
                    height: rect_height,
                });
            }
            for &rect in &self.region {
                copy_rect(self.renderer.pixels(), width, &mut buffer, rect);
            }
            self.window.0.pre_present_notify();
            buffer
                .present_with_damage(&self.reported)
                .map_err(PlatformError::Surface)?;
        } else {
            bytes_copied = u64::from(width) * u64::from(height) * 4;
            copy_rect(
                self.renderer.pixels(),
                width,
                &mut buffer,
                PixelRect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                },
            );
            if self.show_damage {
                tint_rects(&mut buffer, width, &self.pending.damage);
            }
            self.window.0.pre_present_notify();
            buffer.present().map_err(PlatformError::Surface)?;
        }
        self.history.push(&self.pending.damage, self.pending.full);
        Ok(Some(PresentStats {
            buffer_age: age,
            with_damage,
            bytes_copied,
            surface_width: width,
            surface_height: height,
            raster,
            acquire,
            present: present_start.elapsed(),
            renderer: "cpu",
        }))
    }
}

/// Converts a physical size to logical pixels for `scale`.
fn logical_size(physical: PhysicalSize<u32>, scale: f32) -> Size {
    Size {
        width: physical.width as f32 / scale,
        height: physical.height as f32 / scale,
    }
}

/// What the window does about the next timer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum WaitDecision {
    /// A timer is already due: run the frame that fires it now.
    Now,
    /// Sleep until the soonest timer fires.
    Until(Instant),
    /// No timer is scheduled: wait for events.
    Wait,
}

/// Decides what the window does about the next timer due at `deadline`.
fn wait_decision(deadline: Option<Instant>, now: Instant) -> WaitDecision {
    match deadline {
        Some(deadline) if deadline <= now => WaitDecision::Now,
        Some(deadline) => WaitDecision::Until(deadline),
        None => WaitDecision::Wait,
    }
}

/// A window handle that the presentation surfaces can own.
///
/// The surfaces store the window they present to and the platform state keeps
/// the window too, so both hold the same allocation. The `Arc` makes the
/// handle sendable, which the wgpu surface target requires.
#[derive(Clone)]
pub(crate) struct SharedWindow(pub(crate) Arc<Window>);

impl HasWindowHandle for SharedWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for SharedWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
