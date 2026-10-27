//! Presenting frames on a wgpu surface, one full redraw at a time.
//!
//! Vello renders through a compute pass into a storage texture, and a
//! surface texture cannot be bound as a storage texture on most backends.
//! This module therefore renders the frame into an intermediate RGBA8
//! texture of the surface's size and copies it onto the surface texture with
//! a blit pipeline.

use std::error::Error;
use std::fmt;

use smudge_core::app::FrameOutput;
use vello::RenderParams;
use vello::wgpu::util::TextureBlitter;
use vello::wgpu::{
    Adapter, CommandEncoderDescriptor, CompositeAlphaMode, CurrentSurfaceTexture, Device, Extent3d,
    PresentMode, Queue, Surface, SurfaceColorSpace, SurfaceConfiguration, SurfaceTexture, Texture,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

use crate::encoder::{Encoder, peniko_color};

/// The surface formats the blit pattern supports: the intermediate texture is
/// always RGBA8 and the blit converts while it samples.
fn is_vello_format(format: &TextureFormat) -> bool {
    matches!(
        format,
        TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm
    )
}

/// Returns the first of `formats` the presenter can render into, keeping the
/// surface's own preference order.
fn pick_format(formats: &[TextureFormat]) -> Option<TextureFormat> {
    formats.iter().copied().find(is_vello_format)
}

/// Something that stopped a [`GpuSurface`] from being created or from drawing
/// a frame.
#[derive(Debug)]
pub enum GpuError {
    /// The surface offered no format the presenter can render into.
    UnsupportedFormat,
    /// The surface is larger than the device can texture.
    SurfaceTooLarge {
        /// The width of the surface in physical pixels.
        width: u32,
        /// The height of the surface in physical pixels.
        height: u32,
        /// The largest side length the device supports.
        limit: u32,
    },
    /// The window surface or the device was lost.
    ///
    /// The window must be rebuilt with a new surface to keep presenting.
    SurfaceLost,
    /// The device rejected a vello render, which means the device cannot run
    /// the shaders vello needs or has become unusable.
    Render(vello::Error),
}

impl fmt::Display for GpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::UnsupportedFormat => write!(
                formatter,
                "the window surface offers no format the GPU renderer supports"
            ),
            GpuError::SurfaceTooLarge {
                width,
                height,
                limit,
            } => write!(
                formatter,
                "a {width}x{height} surface exceeds the device's texture limit of {limit}"
            ),
            GpuError::SurfaceLost => {
                write!(formatter, "the window surface or the device was lost")
            }
            GpuError::Render(error) => write!(formatter, "the vello render failed: {error}"),
        }
    }
}

impl Error for GpuError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            GpuError::Render(error) => Some(error),
            GpuError::UnsupportedFormat
            | GpuError::SurfaceTooLarge { .. }
            | GpuError::SurfaceLost => None,
        }
    }
}

/// A GPU presenter that draws every frame onto a wgpu surface.
///
/// The presenter renders the frame's whole display list on every
/// [`GpuSurface::draw`] call, whatever its damage says: the GPU path redraws
/// the full frame. It owns the surface's configuration and hands out surface
/// textures through [`GpuSurface::acquire`], so a caller can time the wait
/// for a texture separately from the draw.
///
/// The surface is configured with the first format in the adapter's
/// capability order that the presenter supports, with FIFO present mode, a
/// latency of two frames, and automatic alpha composition.
pub struct GpuSurface {
    /// The device the textures and the pipelines live on.
    device: Device,
    /// The queue draws are submitted to.
    queue: Queue,
    /// The surface the frames present to.
    surface: Surface<'static>,
    /// The configuration the surface is presented under.
    config: SurfaceConfiguration,
    /// The vello renderer, which holds the compiled pipelines.
    renderer: vello::Renderer,
    /// The scene encoder of the current frame, reused across frames.
    encoder: Encoder,
    /// The pipeline that copies the rendered texture onto the surface.
    blitter: TextureBlitter,
    /// The intermediate texture vello renders into and its view.
    target: (Texture, TextureView),
    /// Whether the next [`GpuSurface::acquire`] must reconfigure the surface
    /// first, set when a texture came back suboptimal. That texture must be
    /// dropped before a reconfigure, and wgpu rejects a configure while one
    /// is alive.
    reconfigure: bool,
}

impl GpuSurface {
    /// Configures `surface` for `adapter` and returns a presenter for it.
    ///
    /// The device's texture limit must fit the surface; a larger surface
    /// returns [`GpuError::SurfaceTooLarge`] instead of a renderer.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::UnsupportedFormat`] when the surface offers no
    /// format the presenter can render into, and
    /// [`GpuError::SurfaceTooLarge`] when `width` or `height` exceeds the
    /// device's `max_texture_dimension_2d` limit.
    ///
    /// # Panics
    ///
    /// Panics when the device cannot create the vello pipelines, which means
    /// the device cannot run the shaders vello needs. That is a broken device
    /// state rather than input a caller can provoke.
    pub fn new(
        surface: Surface<'static>,
        adapter: &Adapter,
        device: Device,
        queue: Queue,
        width: u32,
        height: u32,
    ) -> Result<GpuSurface, GpuError> {
        let limit = device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(GpuError::SurfaceTooLarge {
                width,
                height,
                limit,
            });
        }
        let capabilities = surface.get_capabilities(adapter);
        let format = pick_format(&capabilities.formats).ok_or(GpuError::UnsupportedFormat)?;
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);
        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .expect("the device cannot create the vello pipelines");
        let blitter = TextureBlitter::new(&device, format);
        let target = create_target(&device, width, height);
        Ok(GpuSurface {
            device,
            queue,
            surface,
            config,
            renderer,
            encoder: Encoder::new(),
            blitter,
            target,
            reconfigure: false,
        })
    }

    /// Reconfigures the surface and the intermediate texture for a new size.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::SurfaceTooLarge`] when `width` or `height`
    /// exceeds the device's `max_texture_dimension_2d` limit. The presenter
    /// keeps its old size in that case.
    ///
    /// # Panics
    ///
    /// Panics when `width` or `height` is zero, which cannot present: the
    /// caller keeps the surface at its last size while the window has no
    /// extent. Also panics, inside wgpu, when a texture from
    /// [`GpuSurface::acquire`] is still alive, because a surface cannot be
    /// reconfigured while one of its textures is out.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        assert!(
            width > 0 && height > 0,
            "a resized surface must keep a positive extent"
        );
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(GpuError::SurfaceTooLarge {
                width,
                height,
                limit,
            });
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.reconfigure = false;
        self.target = create_target(&self.device, width, height);
        Ok(())
    }

    /// Returns the width the surface presents at, in physical pixels.
    pub fn width(&self) -> u32 {
        self.config.width
    }

    /// Returns the height the surface presents at, in physical pixels.
    pub fn height(&self) -> u32 {
        self.config.height
    }

    /// Waits for the next surface texture to draw onto.
    ///
    /// Draw or drop the returned texture before the next
    /// [`GpuSurface::resize`], which cannot reconfigure the surface while it
    /// is out. A surface that went out of date is reconfigured and asked once
    /// more.
    /// Returns `Ok(None)` when the frame must be skipped: the surface is
    /// still out of date, is temporarily out of textures, or is covered.
    /// Returns [`GpuError::SurfaceLost`] when the surface cannot present
    /// anymore.
    pub fn acquire(&mut self) -> Result<Option<SurfaceTexture>, GpuError> {
        // No surface texture of the last frame is alive here: a drawn one was
        // handed to the queue, a skipped one was dropped, so a reconfigure is
        // safe now.
        if self.reconfigure {
            self.reconfigure = false;
            self.surface.configure(&self.device, &self.config);
        }
        match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                self.texture_from(self.surface.get_current_texture())
            }
            other => self.texture_from(other),
        }
    }

    /// Turns one answer of the surface into the texture to draw onto, a
    /// skipped frame, or a lost surface.
    fn texture_from(
        &mut self,
        current: CurrentSurfaceTexture,
    ) -> Result<Option<SurfaceTexture>, GpuError> {
        match current {
            CurrentSurfaceTexture::Success(texture) => Ok(Some(texture)),
            CurrentSurfaceTexture::Suboptimal(texture) => {
                // The surface no longer matches its configuration, most often
                // after a resize. Reconfigure once the texture is dropped, so
                // the next frame goes out at the right size.
                self.reconfigure = true;
                Ok(Some(texture))
            }
            CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Outdated => Ok(None),
            _ => Err(GpuError::SurfaceLost),
        }
    }

    /// Renders `frame` onto `texture` and presents it.
    ///
    /// The whole display list is drawn and the whole surface is presented,
    /// whatever [`FrameOutput::damage`] holds. The draw reaches the screen
    /// asynchronously: the submission is queued when `draw` returns.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::Render`] when the device rejects the vello render,
    /// which means it cannot run the shaders vello needs or has become
    /// unusable. Nothing is presented then.
    pub fn draw(
        &mut self,
        frame: &FrameOutput<'_>,
        texture: SurfaceTexture,
    ) -> Result<(), GpuError> {
        let params = RenderParams {
            base_color: peniko_color(frame.clear_color),
            width: self.config.width,
            height: self.config.height,
            antialiasing_method: vello::AaConfig::Area,
        };
        let GpuSurface {
            device,
            queue,
            renderer,
            encoder,
            blitter,
            target,
            ..
        } = self;
        encoder.encode(frame);
        renderer
            .render_to_texture(device, queue, encoder.scene(), &target.1, &params)
            .map_err(GpuError::Render)?;
        let surface_view = texture
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut command = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("smudge present"),
        });
        blitter.copy(device, &mut command, &target.1, &surface_view);
        queue.submit([command.finish()]);
        queue.present(texture);
        Ok(())
    }
}

/// Creates the intermediate texture vello renders into.
///
/// The caller keeps the extent above zero, because a surface without pixels
/// has nothing to present and wgpu rejects empty textures.
fn create_target(device: &Device, width: u32, height: u32) -> (Texture, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("smudge gpu surface"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
