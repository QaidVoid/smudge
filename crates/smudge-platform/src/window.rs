//! The window, its run loop and the frames it presents.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::app::{App, AppConfig, FrameStats, RepaintMode};
use smudge_core::element::IntoElement;
use smudge_core::geometry::{Color, PixelRect, Size};
use smudge_core::reactive;
use smudge_cpu::CpuRenderer;
use softbuffer::{Context, Surface};
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::{Window, WindowId};

use crate::error::PlatformError;
use crate::present::{DamageHistory, PresentStats, PresentStatsHandle, copy_rect, damage_region};

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
    /// the surface into the window.
    pub clear_color: Color,
    /// How much of the surface each frame repaints.
    pub repaint: RepaintMode,
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
/// # Errors
///
/// Returns [`PlatformError::EventLoop`] when the event loop cannot be created
/// or run, [`PlatformError::Window`] when the window cannot be created, and
/// [`PlatformError::Surface`] when the presentation surface cannot be created,
/// resized or presented.
///
/// # Panics
///
/// Panics in debug builds when [`WindowOptions::clear_color`] is not opaque.
/// Panics when another async executor was installed in this process before,
/// because the reactive runtime installs a process-wide one.
///
/// # Examples
///
/// ```no_run
/// use smudge_core::app::RepaintMode;
/// use smudge_core::geometry::{Color, Size};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_platform::{WindowOptions, run};
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
/// # Errors
///
/// Returns the same errors as [`run`].
///
/// # Panics
///
/// Panics in debug builds when [`WindowOptions::clear_color`] is not opaque.
/// Panics when another async executor was installed in this process before,
/// because the reactive runtime installs a process-wide one.
///
/// # Examples
///
/// ```no_run
/// use smudge_core::app::RepaintMode;
/// use smudge_core::geometry::{Color, Size};
/// use smudge_core::reactive::{self, prelude::*};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_platform::{WindowOptions, run_with};
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
///         stats: None,
///         on_frame: None,
///         frame_limit: None,
///     },
///     move || panel().width(px(120.0)).background(move || color.get()),
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
    debug_assert!(
        options.clear_color.is_opaque(),
        "presentation copies only the color channels of the surface, so the \
         clear color must be opaque"
    );

    let event_loop = EventLoop::new().map_err(PlatformError::EventLoop)?;
    let mut platform = Platform {
        options,
        build: Some(build),
        configure: Some(Box::new(configure)),
        state: None,
        error: None,
    };
    event_loop
        .run_app(&mut platform)
        .map_err(PlatformError::EventLoop)?;
    match platform.error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The state the event loop drives.
struct Platform<E, B>
where
    E: IntoElement,
    B: FnOnce() -> E,
{
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
}

/// The setup a caller hands to [`run_with`], boxed so the platform state does
/// not become generic over it.
type Configure = Box<dyn FnOnce(&mut App)>;

/// One live window and everything it presents.
struct WindowState {
    /// The window the surface presents to.
    window: Rc<Window>,
    /// The surface that holds the pixels the window shows.
    surface: Surface<SharedWindow, SharedWindow>,
    /// The app whose frames the window runs.
    app: App,
    /// The renderer that paints the app's damage into a pixmap.
    renderer: CpuRenderer,
    /// The damage of the frames the window presented, most recent first.
    history: DamageHistory,
    /// The frame that was rendered and waits to be presented.
    pending: PendingFrame,
    /// Scratch for the region the next present copies.
    region: Vec<PixelRect>,
    /// Scratch for the damage rectangles the next present reports.
    reported: Vec<softbuffer::Rect>,
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
    /// Creates the window, its app and its renderer.
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<WindowState, PlatformError> {
        let attributes = Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(LogicalSize::new(
                f64::from(self.options.size.width),
                f64::from(self.options.size.height),
            ));
        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .map_err(PlatformError::Window)?,
        );

        let shared = SharedWindow(Rc::clone(&window));
        let context = Context::new(shared.clone()).map_err(PlatformError::Surface)?;
        let mut surface = Surface::new(&context, shared).map_err(PlatformError::Surface)?;
        drop(context);

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
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            surface
                .resize(width, height)
                .map_err(PlatformError::Surface)?;
        }
        let renderer = CpuRenderer::new(width, height);

        let weak = Rc::downgrade(&window);
        reactive::set_wake_callback(move || {
            if let Some(window) = weak.upgrade() {
                window.request_redraw();
            }
        });
        window.request_redraw();

        Ok(WindowState {
            window,
            surface,
            app,
            renderer,
            history: DamageHistory::default(),
            pending: PendingFrame::default(),
            region: Vec::new(),
            reported: Vec::new(),
            stats: PresentStats::default(),
            stats_handle: self.options.stats.clone(),
            on_frame: self.options.on_frame.take(),
            frame_limit: self.options.frame_limit,
            frames_presented: 0,
            scale,
            physical: (physical.width, physical.height),
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

        state.app.set_viewport(logical_size(physical, scale), scale);
        let (width, height) = state.app.surface_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return Ok(());
        };
        state
            .surface
            .resize(width, height)
            .map_err(PlatformError::Surface)?;
        state.renderer.resize(width.get(), height.get());
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
        let raster;
        {
            let output = state.app.frame();
            let start = Instant::now();
            state.renderer.render(&output);
            raster = start.elapsed();
            let pending = &mut state.pending;
            pending.damage.clear();
            pending.damage.extend_from_slice(output.damage);
            pending.full = output.full;
        }
        if state.present(raster)? {
            state.frames_presented += 1;
            if let Some(on_frame) = &state.on_frame {
                on_frame(state.app.stats().clone(), state.stats);
            }
        }
        if tick {
            state.window.request_redraw();
        }
        Ok(())
    }

    /// Records `error` and stops the event loop.
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: PlatformError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
        event_loop.exit();
    }
}

impl<E, B> ApplicationHandler for Platform<E, B>
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

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
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
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }
}

impl WindowState {
    /// Returns whether the window has presented the number of frames its frame
    /// limit sets, or `false` when no limit is set.
    fn finished(&self) -> bool {
        self.frame_limit
            .is_some_and(|limit| self.frames_presented >= limit)
    }

    /// Presents the frame the last redraw rendered.
    ///
    /// The frame's damage and the recorded damage of the frames still in a
    /// window buffer decide what has to be copied: when the buffer's age says
    /// which frame it holds and no full repaint happened since, only their
    /// union is copied and the present reports it as damage. Every other case
    /// copies the whole pixmap. `raster` is the time the renderer took for the
    /// frame and is recorded with the present's statistics. Returns `false`
    /// when the surface has no pixel, so nothing was presented.
    fn present(&mut self, raster: Duration) -> Result<bool, PlatformError> {
        let (width, height) = (self.renderer.width(), self.renderer.height());
        if width == 0 || height == 0 {
            return Ok(false);
        }
        let acquire_start = Instant::now();
        let mut buffer = self.surface.buffer_mut().map_err(PlatformError::Surface)?;
        let acquire = acquire_start.elapsed();
        let start = Instant::now();
        debug_assert_eq!(
            buffer.len(),
            self.renderer.pixels().len() / 4,
            "the window buffer must hold one entry per rendered pixel"
        );
        let age = buffer.age();
        let with_damage = damage_region(
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
            buffer.present().map_err(PlatformError::Surface)?;
        }
        self.history.push(&self.pending.damage, self.pending.full);
        self.stats = PresentStats {
            buffer_age: age,
            with_damage,
            bytes_copied,
            surface_width: width,
            surface_height: height,
            raster,
            acquire,
            present: start.elapsed(),
        };
        if let Some(handle) = &self.stats_handle {
            handle.store(self.stats);
        }
        Ok(true)
    }
}

/// Converts a physical size to logical pixels for `scale`.
fn logical_size(physical: PhysicalSize<u32>, scale: f32) -> Size {
    Size {
        width: physical.width as f32 / scale,
        height: physical.height as f32 / scale,
    }
}

/// A window handle that the presentation surface can own.
///
/// The surface stores the window it presents to and the platform state keeps
/// the window too, so both hold the same allocation.
#[derive(Clone)]
struct SharedWindow(Rc<Window>);

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
