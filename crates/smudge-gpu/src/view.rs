//! App-drawn GPU content: a widget whose closure draws into a texture with
//! the renderer's own device, which the renderer composites where the
//! widget is.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use smudge_core::app::FrameOutput;
use smudge_core::element::{Element, Value};
use smudge_core::geometry::Corners;
use smudge_core::image::Image;
use smudge_core::paint::{DisplayItem, External, PaintOp};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Widget};
use vello::peniko::ImageData;
use vello::wgpu::{
    Device, Extent3d, Queue, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor,
};

/// The format of the textures a [`gpu_view`] draws into.
///
/// vello copies each one into its image atlas, which holds this format, so
/// the closure draws with straight, not premultiplied, alpha, and its
/// values count as sRGB encoded, as every other color of the interface
/// does. A pipeline that computes linear light encodes it to sRGB in its
/// shader before writing.
pub const VIEW_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// What a [`gpu_view`]'s closure draws with: the renderer's device and
/// queue, and the texture to draw into, as large as the view in physical
/// pixels, but at most 8192 pixels on a side, or the device's texture
/// limit when that is smaller.
#[non_exhaustive]
pub struct GpuFrame<'a> {
    /// The device the renderer draws with.
    pub device: &'a Device,
    /// The queue to submit the drawing to. The renderer composites after
    /// what was submitted, on the same queue.
    pub queue: &'a Queue,
    /// The texture to draw into, which keeps what was drawn until the next
    /// draw.
    pub target: &'a TextureView,
    /// The texture's format, [`VIEW_FORMAT`].
    pub format: TextureFormat,
    /// The texture's width in physical pixels.
    pub width: u32,
    /// The texture's height in physical pixels.
    pub height: u32,
}

/// The closure a [`gpu_view`] draws with.
type Render = Box<dyn FnMut(GpuFrame<'_>)>;

/// The closure of a [`gpu_view`], which the GPU renderer finds through
/// [`External::source`].
struct ViewSource {
    /// The closure that draws.
    render: RefCell<Render>,
}

/// The widget [`gpu_view`] builds.
pub struct GpuView {
    /// Tells the view's content apart from every other.
    id: u64,
    /// Changes whenever the view needs drawing again.
    generation: u64,
    /// The image the CPU renderer shows.
    fallback: Option<Image>,
    /// The closure, as the content's source.
    source: Rc<dyn Any>,
}

impl Widget for GpuView {
    /// Paints the view's content over its content box, rounded inside its
    /// corners like an image.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        smudge_core::widget::Panel.paint(cx, out);
        let radius = cx.style.paint.corner_radius.resolve(cx.theme);
        let left = cx.content.x0 - cx.rect.x0;
        let top = cx.content.y0 - cx.rect.y0;
        let right = cx.rect.x1 - cx.content.x1;
        let bottom = cx.rect.y1 - cx.content.y1;
        let inner = |radius: f32, a: f32, b: f32| (radius - a.max(b)).max(0.0);
        out.push(PaintOp::External {
            rect: cx.content,
            radius: Corners {
                top_left: inner(radius.top_left, left, top),
                top_right: inner(radius.top_right, right, top),
                bottom_right: inner(radius.bottom_right, right, bottom),
                bottom_left: inner(radius.bottom_left, left, bottom),
            },
            content: Rc::new(External::new(
                self.id,
                self.generation,
                self.fallback.clone(),
                Some(Rc::clone(&self.source)),
            )),
        });
    }
}

/// Hands out the ids of views, unique for the process.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Creates a view whose content `render` draws with the GPU renderer's own
/// device and queue, for a 3D view or anything else an app renders with
/// wgpu itself.
///
/// `render` draws into a texture as large as the view's content box in
/// physical pixels, at most 8192 or the device's texture limit on a
/// side, in [`VIEW_FORMAT`], before the renderer composites the
/// frame, and the renderer then draws the texture where the view is, under
/// its clips, opacity and rounded corners. It runs when the view first
/// shows, when its size changes, and when what
/// [`redraw_when`](GpuViewExt::redraw_when) reads changes, so the app draws
/// at the rate it chooses. The texture keeps the last drawing in between.
///
/// The CPU renderer has no GPU, so there the view shows the image
/// [`fallback`](GpuViewExt::fallback) gives, which the app draws on the CPU,
/// or nothing without one. That is the slow path: Smudge creates no device
/// of its own for it.
///
/// ```no_run
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::style::px;
/// use smudge_gpu::{GpuViewExt, gpu_view, wgpu};
///
/// let time = RwSignal::new(0.0_f32);
/// let shown = time.clone();
/// let view = gpu_view(move |frame| {
///     let mut encoder = frame.device.create_command_encoder(&Default::default());
///     {
///         let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
///             label: None,
///             color_attachments: &[Some(wgpu::RenderPassColorAttachment {
///                 view: frame.target,
///                 depth_slice: None,
///                 resolve_target: None,
///                 ops: wgpu::Operations {
///                     load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
///                     store: wgpu::StoreOp::Store,
///                 },
///             })],
///             ..Default::default()
///         });
///     }
///     frame.queue.submit([encoder.finish()]);
/// })
/// .redraw_when(move || shown.get())
/// .width(px(320.0))
/// .height(px(240.0));
/// ```
#[track_caller]
pub fn gpu_view(render: impl FnMut(GpuFrame<'_>) + 'static) -> Element<GpuView> {
    Element::new(GpuView {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        generation: 0,
        fallback: None,
        source: Rc::new(ViewSource {
            render: RefCell::new(Box::new(render)),
        }),
    })
}

/// The builders of a [`gpu_view`].
pub trait GpuViewExt: Sized {
    /// Draws the view again whenever the value `inputs` returns changes,
    /// such as a camera or a time. `inputs` reads the signals the drawing
    /// depends on.
    fn redraw_when<T: PartialEq + 'static>(self, inputs: impl Fn() -> T + 'static) -> Self;

    /// Shows `image` under the CPU renderer, which cannot draw the view.
    fn fallback(self, image: impl Into<Value<Option<Image>>>) -> Self;
}

/// The key of a view's redraw binding.
struct Redraw;

/// The key of a view's fallback binding.
struct Fallback;

impl GpuViewExt for Element<GpuView> {
    fn redraw_when<T: PartialEq + 'static>(self, inputs: impl Fn() -> T + 'static) -> Self {
        let last: RefCell<Option<T>> = RefCell::new(None);
        let changes = std::cell::Cell::new(0_u64);
        self.bind(
            Redraw,
            move || {
                let value = inputs();
                let mut last = last.borrow_mut();
                if last.as_ref() != Some(&value) {
                    *last = Some(value);
                    changes.set(changes.get() + 1);
                }
                changes.get()
            },
            |view, generation| {
                if view.generation == generation {
                    Dirty::Clean
                } else {
                    view.generation = generation;
                    Dirty::Paint
                }
            },
        )
    }

    fn fallback(self, image: impl Into<Value<Option<Image>>>) -> Self {
        self.bind(Fallback, image, |view, image| {
            view.fallback = image;
            Dirty::Paint
        })
    }
}

/// A texture a view draws into, registered with vello as an image.
struct Target {
    /// The texture.
    texture: Texture,
    /// The view of the texture the closure draws into.
    view: TextureView,
    /// The image vello composites, backed by the texture.
    image: ImageData,
    /// The generation of the content last drawn, or `None` before the first
    /// draw.
    drawn: Option<u64>,
    /// The frame the texture was last used in.
    used: u64,
}

/// The textures of the views of the frames being rendered.
#[derive(Default)]
pub(crate) struct Views {
    /// The textures, by view id.
    targets: HashMap<u64, Target>,
    /// How many frames were prepared, for the stamps.
    frame: u64,
}

impl Views {
    /// Has every view of `frames` that needs it draw into its texture, with
    /// textures at most `max_side` pixels on a side, and releases the
    /// textures of views the frames no longer show.
    pub(crate) fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        renderer: &mut vello::Renderer,
        frames: &[&FrameOutput<'_>],
        max_side: u32,
    ) {
        self.frame += 1;
        let items = frames.iter().flat_map(|frame| {
            frame
                .list
                .items()
                .iter()
                .map(move |item| (frame.scale, item))
        });
        for (scale, item) in items {
            let DisplayItem::Op(PaintOp::External { rect, content, .. }) = item else {
                continue;
            };
            let Some(source) = content
                .source
                .as_ref()
                .and_then(|source| source.downcast_ref::<ViewSource>())
            else {
                continue;
            };
            let side = |length: f32| {
                let pixels = (length * scale).ceil();
                (pixels.is_finite() && pixels >= 1.0).then(|| (pixels as u32).min(max_side))
            };
            let (Some(width), Some(height)) = (side(rect.width()), side(rect.height())) else {
                continue;
            };
            let target = self.targets.get(&content.id).filter(|target| {
                target.texture.width() == width && target.texture.height() == height
            });
            if target.is_none() {
                if let Some(old) = self.targets.remove(&content.id) {
                    renderer.override_image(&old.image, None);
                }
                let texture = device.create_texture(&TextureDescriptor {
                    label: Some("smudge gpu view"),
                    size: Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: VIEW_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::COPY_SRC
                        | TextureUsages::COPY_DST
                        | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&TextureViewDescriptor::default());
                let image = renderer.register_texture(texture.clone());
                self.targets.insert(
                    content.id,
                    Target {
                        texture,
                        view,
                        image,
                        drawn: None,
                        used: 0,
                    },
                );
            }
            let target = self
                .targets
                .get_mut(&content.id)
                .expect("the target was just made");
            target.used = self.frame;
            if target.drawn != Some(content.generation) {
                (source.render.borrow_mut())(GpuFrame {
                    device,
                    queue,
                    target: &target.view,
                    format: VIEW_FORMAT,
                    width,
                    height,
                });
                target.drawn = Some(content.generation);
                renderer.mark_override_image_dirty(&target.image);
            }
        }
        let frame = self.frame;
        self.targets.retain(|_, target| {
            let keep = target.used == frame;
            if !keep {
                renderer.override_image(&target.image, None);
            }
            keep
        });
    }

    /// Returns the image vello composites for the view `id`, when the last
    /// prepared frame drew it.
    pub(crate) fn image(&self, id: u64) -> Option<&ImageData> {
        self.targets.get(&id).map(|target| &target.image)
    }
}
