//! The GPU renderer that encodes a display list into a vello scene and
//! renders it to an owned texture.

use std::collections::HashMap;

use futures_executor::block_on;
use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::{DisplayItem, FontBlob, PaintOp};
use smudge_core::renderer::Renderer;
use vello::kurbo::{Affine, Rect as KurboRect, RoundedRect, RoundedRectRadii, Stroke};
use vello::peniko::{Blob, Color as PenikoColor, Fill, FontData};
use vello::wgpu;
use vello::wgpu::{
    Device, Extent3d, Instance, InstanceDescriptor, MapMode, PollType, Queue,
    RequestAdapterOptions, TexelCopyBufferInfo, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};
use vello::{Glyph, Scene};

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
    /// The scene of the current frame, reused across frames.
    scene: Scene,
    /// The font data handed to vello, keyed by the paint operation's font id
    /// and face index, or `None` for a face that does not parse.
    fonts: HashMap<(u64, u32), Option<FontData>>,
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
            scene: Scene::new(),
            fonts: HashMap::new(),
        }
    }

    /// Creates a renderer on a GPU adapter chosen without a window, so tests
    /// and offscreen uses need no surface.
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
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
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
        let GpuRenderer {
            device,
            queue,
            renderer,
            scene,
            fonts,
            ..
        } = self;
        encode(scene, fonts, frame);
        renderer
            .render_to_texture(
                device,
                queue,
                scene,
                view,
                &vello::RenderParams {
                    base_color: peniko_color(frame.clear_color),
                    width: self.width,
                    height: self.height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
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

/// Encodes `frame` into `scene`, replacing whatever it held.
fn encode(
    scene: &mut Scene,
    fonts: &mut HashMap<(u64, u32), Option<FontData>>,
    frame: &FrameOutput<'_>,
) {
    scene.reset();
    let transform = Affine::scale(f64::from(frame.scale));
    // The display list is balanced by construction, but a stray `PopClip` in
    // a release build would pop a vello layer that was never pushed, so the
    // depth is tracked here and leftover clips are closed before rendering.
    let mut depth = 0usize;
    for item in frame.list.items() {
        match item {
            DisplayItem::Op(op) => encode_op(scene, fonts, transform, op),
            DisplayItem::PushClip(rect) => {
                depth += 1;
                scene.push_clip_layer(Fill::NonZero, transform, &kurbo_rect(*rect));
            }
            DisplayItem::PopClip => {
                if depth > 0 {
                    depth -= 1;
                    scene.pop_layer();
                }
            }
        }
    }
    for _ in 0..depth {
        scene.pop_layer();
    }
}

/// Encodes one paint operation.
///
/// The transform scales logical coordinates to physical pixels; for strokes
/// it also scales the width, which vello applies centered on the path.
fn encode_op(
    scene: &mut Scene,
    fonts: &mut HashMap<(u64, u32), Option<FontData>>,
    transform: Affine,
    op: &PaintOp,
) {
    match *op {
        PaintOp::FillRect {
            rect,
            radius,
            color,
        } => fill(scene, transform, color, rect, radius),
        PaintOp::StrokeRect {
            rect,
            radius,
            width,
            color,
        } => {
            if !width.is_finite() || width <= 0.0 {
                return;
            }
            match inset(rect, radius, width / 2.0) {
                Some((rect, radius)) => {
                    let shape = RoundedRect::from_rect(kurbo_rect(rect), kurbo_radii(radius));
                    scene.stroke(
                        &Stroke::new(f64::from(width)),
                        transform,
                        peniko_color(color),
                        None,
                        &shape,
                    );
                }
                // A border thicker than the rectangle covers it whole, which
                // is what the CPU renderer draws in that case.
                None => fill(scene, transform, color, rect, radius),
            }
        }
        PaintOp::Glyphs(ref run) => {
            if !run.size.is_finite() || run.size <= 0.0 {
                return;
            }
            let Some(font) = font_data(fonts, &run.font) else {
                return;
            };
            scene
                .draw_glyphs(font)
                .transform(transform)
                .font_size(run.size)
                .brush(peniko_color(run.color))
                .draw(
                    Fill::NonZero,
                    run.glyphs.iter().map(|glyph| Glyph {
                        id: glyph.id,
                        x: glyph.x,
                        y: glyph.y,
                    }),
                );
        }
    }
}

/// Fills a rectangle with optional rounded corners.
fn fill(scene: &mut Scene, transform: Affine, color: Color, rect: Rect, radius: Corners) {
    let color = peniko_color(color);
    if radius == Corners::ZERO {
        scene.fill(Fill::NonZero, transform, color, None, &kurbo_rect(rect));
    } else {
        let shape = RoundedRect::from_rect(kurbo_rect(rect), kurbo_radii(radius));
        scene.fill(Fill::NonZero, transform, color, None, &shape);
    }
}

/// Returns `rect` shrunk by `inset` on every side with the corner radii
/// reduced to match, or `None` when nothing remains.
///
/// A non-finite extent counts as nothing remaining, so a broken rectangle can
/// never reach vello as a stroke.
fn inset(rect: Rect, radius: Corners, inset: f32) -> Option<(Rect, Corners)> {
    let rect = Rect::new(
        rect.x0 + inset,
        rect.y0 + inset,
        rect.x1 - inset,
        rect.y1 - inset,
    );
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return None;
    }
    let radius = Corners {
        top_left: (radius.top_left - inset).max(0.0),
        top_right: (radius.top_right - inset).max(0.0),
        bottom_right: (radius.bottom_right - inset).max(0.0),
        bottom_left: (radius.bottom_left - inset).max(0.0),
    };
    Some((rect, radius))
}

/// Returns the vello font data for `blob`, creating it once per font file and
/// face, or `None` when the bytes hold no such face.
///
/// The cache is required for more than saving a copy: vello keys its glyph
/// cache by the font data identity, so a fresh blob per frame would re-upload
/// and re-rasterize every glyph each frame. The face is parsed once here
/// because vello unwraps that parse, and a run in a face that does not parse
/// draws nothing, as on the CPU.
fn font_data<'a>(
    fonts: &'a mut HashMap<(u64, u32), Option<FontData>>,
    blob: &FontBlob,
) -> Option<&'a FontData> {
    fonts
        .entry((blob.id, blob.index))
        .or_insert_with(|| {
            let bytes: &[u8] = (*blob.data).as_ref();
            skrifa::FontRef::from_index(bytes, blob.index)
                .is_ok()
                .then(|| FontData::new(Blob::new(blob.data.clone()), blob.index))
        })
        .as_ref()
}

/// Converts a straight-alpha sRGB byte color to vello's color type.
fn peniko_color(color: Color) -> PenikoColor {
    PenikoColor::from_rgba8(color.r, color.g, color.b, color.a)
}

/// Converts a rectangle to kurbo's coordinate type.
fn kurbo_rect(rect: Rect) -> KurboRect {
    KurboRect::new(
        f64::from(rect.x0),
        f64::from(rect.y0),
        f64::from(rect.x1),
        f64::from(rect.y1),
    )
}

/// Converts corner radii to kurbo's type, clamping negatives to zero.
fn kurbo_radii(radius: Corners) -> RoundedRectRadii {
    RoundedRectRadii::new(
        f64::from(radius.top_left.max(0.0)),
        f64::from(radius.top_right.max(0.0)),
        f64::from(radius.bottom_right.max(0.0)),
        f64::from(radius.bottom_left.max(0.0)),
    )
}
