//! The GPU renderer that encodes a display list into a vello scene and
//! renders it to an owned texture.

use futures_executor::block_on;
use smudge_core::app::FrameOutput;
use smudge_core::renderer::Renderer;
use vello::RenderParams;
use vello::wgpu;
use vello::wgpu::{
    Device, Extent3d, Instance, InstanceDescriptor, MapMode, PollType, Queue,
    RequestAdapterOptions, TexelCopyBufferInfo, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};

use crate::encoder::{Encoder, peniko_color};

/// How many bytes one row of a readback buffer is aligned to.
///
/// wgpu requires the row stride of a texture copy to be a multiple of this.
const ROW_ALIGNMENT: u32 = 256;

/// A renderer that draws every frame on a GPU through vello.
///
/// The renderer owns one texture of its configured size and repaints the
/// whole texture on every [`Renderer::render`] call. Damage is ignored: the
/// GPU path redraws the full frame, so [`FrameOutput::damage`] and
/// [`FrameOutput::full`] carry no meaning for it. [`FrameOutput::clear_color`]
/// becomes the color the frame is cleared with, and [`FrameOutput::scale`]
/// scales the logical display list coordinates into physical pixels.
///
/// To draw onto a window instead, use [`GpuSurface`], which presents through
/// a wgpu surface.
///
/// # Panics
///
/// Panics when the device cannot create or run the vello pipelines, which
/// means the device cannot run the shaders vello needs. That is a broken
/// device state rather than input a caller can provoke.
///
/// # Examples
///
/// ```
/// use smudge_core::app::FrameOutput;
/// use smudge_core::geometry::{Color, Corners, Rect};
/// use smudge_core::paint::{DisplayList, PaintOp};
/// use smudge_core::renderer::Renderer;
/// use smudge_gpu::GpuRenderer;
///
/// // On a machine without a GPU adapter this draws nothing.
/// let Some(mut renderer) = GpuRenderer::headless(64, 64) else {
///     return;
/// };
/// let mut list = DisplayList::new();
/// list.push_op(PaintOp::FillRect {
///     rect: Rect::new(0.0, 0.0, 64.0, 64.0),
///     radius: Corners::ZERO,
///     color: Color::BLACK,
/// });
/// list.finish();
/// renderer.render(&FrameOutput {
///     list: &list,
///     damage: &[],
///     full: true,
///     clear_color: Color::WHITE,
///     scale: 1.0,
/// });
/// let pixels = renderer.read_pixels();
/// assert_eq!(pixels.len(), 64 * 64 * 4);
/// ```
///
/// [`FrameOutput`]: smudge_core::app::FrameOutput
/// [`GpuSurface`]: crate::GpuSurface
pub struct GpuRenderer {
    /// The device the texture and the pipelines live on.
    device: Device,
    /// The queue renders are submitted to.
    queue: Queue,
    /// The width of the texture in physical pixels.
    width: u32,
    /// The height of the texture in physical pixels.
    height: u32,
    /// The texture vello renders into and its view, or `None` when the
    /// surface has no pixel.
    target: Option<(Texture, TextureView)>,
    /// The vello renderer, which holds the compiled pipelines.
    renderer: vello::Renderer,
    /// The scene encoder of the current frame, reused across frames.
    encoder: Encoder,
}

impl GpuRenderer {
    /// Creates a renderer for a device, drawing onto a texture of `width` by
    /// `height` physical pixels.
    ///
    /// A surface with a zero extent holds no texture, and
    /// [`GpuRenderer::render`] and [`GpuRenderer::read_pixels`] then do
    /// nothing.
    ///
    /// # Panics
    ///
    /// Panics when `width` or `height` exceeds the device's
    /// `max_texture_dimension_2d` limit, which a caller can read from
    /// [`Device::limits`] beforehand, and under the conditions of the
    /// [`# Panics`](Self#panics) section.
    pub fn new(device: Device, queue: Queue, width: u32, height: u32) -> GpuRenderer {
        let limit = device.limits().max_texture_dimension_2d;
        assert!(
            width <= limit && height <= limit,
            "a {width}x{height} surface exceeds the device's texture limit of {limit}"
        );
        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .expect("the device cannot create the vello pipelines");
        let target = create_target(&device, width, height);
        GpuRenderer {
            device,
            queue,
            width,
            height,
            target,
            renderer,
            encoder: Encoder::new(),
        }
    }

    /// Creates a renderer on a GPU adapter chosen without a window, so tests
    /// and offscreen uses need no surface.
    ///
    /// The adapter comes from an instance whose options, among them the
    /// `WGPU_BACKEND` variable, are read from the environment, exactly as the
    /// window path does. That lets a machine pin the backend without a
    /// rebuild, for example to measure a software rasterizer.
    ///
    /// Returns `None` when no adapter can be created, or when the chosen
    /// adapter cannot provide a device, which is how a machine without a GPU
    /// reports itself.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`GpuRenderer::new`]. The device has
    /// the default limits, which allow 8192 pixels on each side.
    pub fn headless(width: u32, height: u32) -> Option<GpuRenderer> {
        // The environment override matches the window path, so one set of
        // variables picks the backend for both.
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default())).ok()?;
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(GpuRenderer::new(device, queue, width, height))
    }

    /// Returns the rendered frame as straight-alpha RGBA8 pixels, row-major
    /// with no padding between rows, which is the layout vello writes.
    ///
    /// The result is empty when the surface has no pixel.
    ///
    /// # Panics
    ///
    /// Panics when the device is lost, or when the readback buffer cannot be
    /// mapped after the copy completed. Both are broken device states rather
    /// than input a caller can provoke.
    pub fn read_pixels(&self) -> Vec<u8> {
        let Some((texture, _)) = &self.target else {
            return Vec::new();
        };
        let stride = self.stride();
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smudge readback"),
            size: u64::from(stride) * u64::from(self.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("smudge readback"),
            });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(self.height),
                },
            },
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(PollType::wait_indefinitely())
            .expect("the device was lost while reading pixels back");
        receiver
            .recv()
            .expect("the readback callback ran during the poll")
            .expect("the readback buffer could not be mapped");

        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("the readback buffer was already mapped");
        let mut pixels = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for row in 0..self.height {
            let start = row as usize * stride as usize;
            pixels.extend_from_slice(&mapped[start..start + self.width as usize * 4]);
        }
        drop(mapped);
        buffer.unmap();
        pixels
    }

    /// Blocks until the GPU has executed every render submitted so far.
    ///
    /// A render returns once its work is submitted, so a caller that times a
    /// frame calls this to count the GPU's execution and not only the
    /// submission.
    ///
    /// # Panics
    ///
    /// Panics when the device is lost, which is a broken device state rather
    /// than input a caller can provoke.
    pub fn wait(&self) {
        self.device
            .poll(PollType::wait_indefinitely())
            .expect("the device was lost while waiting for renders");
    }

    /// Returns the distance in bytes between the start of two consecutive
    /// rows of a readback buffer.
    fn stride(&self) -> u32 {
        (self.width * 4).next_multiple_of(ROW_ALIGNMENT)
    }
}

impl Renderer for GpuRenderer {
    fn render(&mut self, frame: &FrameOutput<'_>) {
        let Some((_, view)) = &self.target else {
            return;
        };
        let params = RenderParams {
            base_color: peniko_color(frame.clear_color),
            width: self.width,
            height: self.height,
            antialiasing_method: vello::AaConfig::Area,
        };
        let GpuRenderer {
            device,
            queue,
            renderer,
            encoder,
            ..
        } = self;
        encoder.encode(frame);
        renderer
            .render_to_texture(device, queue, encoder.scene(), view, &params)
            .expect("the device rejected a vello render");
    }
}

/// Creates the texture vello renders into.
///
/// Returns `None` when the surface has no pixel, because wgpu cannot create a
/// texture with a zero extent.
fn create_target(device: &Device, width: u32, height: u32) -> Option<(Texture, TextureView)> {
    if width == 0 || height == 0 {
        return None;
    }
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
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    Some((texture, view))
}
