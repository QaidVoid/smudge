//! The window, its run loop and the frames it presents.

use std::num::NonZeroU32;
use std::rc::Rc;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::IntoElement;
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive;
use smudge_cpu::CpuRenderer;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::{Window, WindowId};

use crate::error::PlatformError;
use crate::present::copy_pixels;

/// The settings a window opens with.
#[derive(Clone, Debug)]
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
    /// The number of physical pixels per logical pixel the app last used.
    scale: f32,
    /// The window's size in physical pixels when the app last used it.
    physical: (u32, u32),
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
        {
            let output = state.app.frame();
            state.renderer.render(&output);
        }
        state.present()?;
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
            WindowEvent::RedrawRequested => self.redraw(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }
}

impl WindowState {
    /// Copies the renderer's surface into the window and presents it.
    fn present(&mut self) -> Result<(), PlatformError> {
        if self.renderer.width() == 0 || self.renderer.height() == 0 {
            return Ok(());
        }
        let mut buffer = self.surface.buffer_mut().map_err(PlatformError::Surface)?;
        debug_assert_eq!(
            buffer.len(),
            self.renderer.pixels().len() / 4,
            "the window buffer must hold one entry per rendered pixel"
        );
        copy_pixels(self.renderer.pixels(), &mut buffer);
        buffer.present().map_err(PlatformError::Surface)
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
