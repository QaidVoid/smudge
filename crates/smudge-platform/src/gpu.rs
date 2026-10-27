//! The GPU presentation path: choosing an adapter and presenting frames on a
//! wgpu surface.
//!
//! The window opens with this path when the renderer choice asks for the GPU
//! and an adapter that is not a software rasterizer can present to it. The
//! frames are full redraws; the CPU path stays in charge of damage.

use std::sync::Arc;
use std::time::Duration;

use futures_executor::block_on;
use smudge_core::app::FrameOutput;
use smudge_gpu::wgpu::{AdapterInfo, DeviceType};
use smudge_gpu::{GpuSurface, wgpu};
use web_time::Instant;
use winit::window::Window;

use super::present::{PresentStats, present_skipped};
use crate::error::PlatformError;
use crate::window::SharedWindow;

/// The names that mark a software rasterizer, matched case-insensitively
/// against the adapter's name and driver.
///
/// Some GL adapters do not report a device type of CPU: llvmpipe on the GL
/// backend of wgpu reports a name of `llvmpipe (...)` with a device type of
/// CPU on recent Mesa, and a device type of Other on older ones. The names
/// catch both, and the same rasterizers under their other names.
const SOFTWARE_MARKERS: [&str; 5] = [
    "llvmpipe",
    "softpipe",
    "lavapipe",
    "swiftshader",
    "microsoft basic render driver",
];

/// Returns whether `info` describes a software rasterizer.
///
/// This is the check the automatic renderer choice exists for: on a machine
/// without GPU passthrough, the adapter is a CPU rasterizer, and drawing on
/// it through a GPU renderer is slower than the CPU renderer.
pub(crate) fn adapter_is_software(info: &AdapterInfo) -> bool {
    if info.device_type == DeviceType::Cpu {
        return true;
    }
    let name = info.name.to_lowercase();
    let driver = info.driver.to_lowercase();
    SOFTWARE_MARKERS
        .iter()
        .any(|marker| name.contains(marker) || driver.contains(marker))
}

/// The GPU presentation path of a window.
pub(crate) struct GpuPath {
    /// The presenter that draws the frames onto the surface.
    surface: GpuSurface,
    /// The window whose frames are presented, notified before every present.
    window: Arc<Window>,
}

impl GpuPath {
    /// Sets the GPU path up for `window`.
    ///
    /// With `explicit` set, this is the caller's chosen renderer, so any
    /// failure to set it up fails the run. Otherwise the choice is automatic,
    /// and every failure of this function returns `None` for the CPU path:
    /// no adapter, a software rasterizer, a device that cannot be created, a
    /// surface without a supported format, or a window larger than the
    /// device's texture limit.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Renderer`] when `explicit` is set and the
    /// setup fails, and for surface creation failures of either mode.
    pub(crate) fn new(
        window: &Arc<Window>,
        width: u32,
        height: u32,
        explicit: bool,
    ) -> Result<Option<GpuPath>, PlatformError> {
        let fail = |message: String| {
            if explicit {
                Err(PlatformError::Renderer(message))
            } else {
                Ok(None)
            }
        };
        if width == 0 || height == 0 {
            return fail(String::from(
                "a window without pixels cannot present GPU frames",
            ));
        }
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(Box::new(SharedWindow(Arc::clone(window))))
            .map_err(|error| {
                PlatformError::Renderer(format!(
                    "the window surface could not be created for the GPU renderer: {error}"
                ))
            })?;
        let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        })) {
            Ok(adapter) => adapter,
            Err(error) => {
                return fail(format!("no adapter can present to the window: {error}"));
            }
        };
        // The automatic choice inspects the adapter before anything is built
        // on it, so a software rasterizer costs nothing but the request.
        if !explicit && adapter_is_software(&adapter.get_info()) {
            return Ok(None);
        }
        let (device, queue) =
            match block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
                Ok(device) => device,
                Err(error) => {
                    return fail(format!("the adapter could not provide a device: {error}"));
                }
            };
        let surface = match GpuSurface::new(surface, &adapter, device, queue, width, height) {
            Ok(surface) => surface,
            Err(error) => return fail(format!("the window cannot present GPU frames: {error}")),
        };
        Ok(Some(GpuPath {
            surface,
            window: Arc::clone(window),
        }))
    }

    /// Renders `frame` onto the surface and presents it.
    ///
    /// A frame whose damage is empty and whose app runs a tick changed
    /// nothing, so it is not presented, exactly as on the CPU path. Every
    /// presented frame is a full redraw: [`PresentStats::buffer_age`] stays
    /// at its unknown value, [`PresentStats::with_damage`] is off, and
    /// [`PresentStats::bytes_copied`] reports nothing because the GPU copies
    /// no pixmap. [`PresentStats::raster`] holds the whole draw, whose
    /// submission reaches the GPU asynchronously.
    ///
    /// Before the draw that ends in the present, the window is told that a
    /// frame is about to show. That is the order winit needs on Wayland: it
    /// schedules a frame callback that paces the next redraw, and presenting
    /// without it lets frames outrun the compositor until a blocked present
    /// stalls the window.
    ///
    /// Returns `Ok(None)` when nothing was presented: a skipped frame as on
    /// the CPU path, or a surface that had no texture to give, which the
    /// next redraw retries after a resize.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Renderer`] when the surface or the device was
    /// lost, or the device rejected the render.
    pub(crate) fn render_and_present(
        &mut self,
        frame: &FrameOutput<'_>,
        has_tick: bool,
    ) -> Result<Option<PresentStats>, PlatformError> {
        if present_skipped(frame.damage, has_tick, false) {
            return Ok(None);
        }
        let acquire_start = Instant::now();
        let texture = match self.surface.acquire() {
            Ok(Some(texture)) => texture,
            Ok(None) => return Ok(None),
            Err(error) => {
                return Err(PlatformError::Renderer(format!(
                    "the GPU surface failed: {error}"
                )));
            }
        };
        let acquire = acquire_start.elapsed();
        self.window.pre_present_notify();
        let start = Instant::now();
        self.surface.draw(frame, texture).map_err(|error| {
            PlatformError::Renderer(format!("the GPU renderer failed: {error}"))
        })?;
        let raster = start.elapsed();
        Ok(Some(PresentStats {
            buffer_age: 0,
            with_damage: false,
            bytes_copied: 0,
            surface_width: self.surface.width(),
            surface_height: self.surface.height(),
            raster,
            acquire,
            present: Duration::ZERO,
            renderer: "gpu",
        }))
    }

    /// Resizes the surface and the presenter to the window's new extent.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Renderer`] when the new extent exceeds the
    /// device's texture limit. The run fails, because the renderer choice is
    /// fixed when the window opens.
    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<(), PlatformError> {
        self.surface.resize(width, height).map_err(|error| {
            PlatformError::Renderer(format!("the GPU surface could not be resized: {error}"))
        })
    }
}

#[cfg(test)]
#[path = "gpu_tests.rs"]
mod tests;
