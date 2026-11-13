//! Encoding a display list into a vello scene, shared by the offscreen
//! renderer and the surface presenter.

use std::collections::HashMap;
use std::sync::Arc;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::{DisplayItem, FontBlob, PaintOp};
use vello::kurbo::{Affine, BezPath, Rect as KurboRect, RoundedRect, RoundedRectRadii, Shape};
use vello::peniko::{
    Blob, Color as PenikoColor, Fill, FontData, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
};
use vello::{Glyph, Scene};

/// A vello scene with the font data of the runs encoded into it.
///
/// The font cache is required for more than saving a copy: vello keys its
/// glyph cache by the font data identity, so a fresh blob per frame would
/// re-upload and re-rasterize every glyph each frame.
pub(crate) struct Encoder {
    /// The scene of the current frame, reused across frames.
    scene: Scene,
    /// The font data handed to vello, keyed by the paint operation's font id
    /// and face index, or `None` for a face that does not parse.
    fonts: HashMap<(u64, u32), Option<FontData>>,
    /// The images handed to vello.
    images: ImageCache,
    /// Scratch for the outline of a border ring, reused across operations.
    ring: BezPath,
    /// The transform of each open offset scope, reused across frames.
    offsets: Vec<Affine>,
    /// The visible area outside each open clip, in physical pixels, reused
    /// across frames.
    clips: Vec<KurboRect>,
}

impl Encoder {
    /// Creates an empty encoder for a device whose textures may be
    /// `max_texture_side` pixels on a side.
    pub(crate) fn new(max_texture_side: u32) -> Encoder {
        Encoder {
            scene: Scene::new(),
            fonts: HashMap::new(),
            images: ImageCache {
                brushes: HashMap::new(),
                generation: 0,
                max_side: max_texture_side.clamp(1, ATLAS_SIDE),
            },
            ring: BezPath::new(),
            offsets: Vec::new(),
            clips: Vec::new(),
        }
    }

    /// Replaces the scene's contents with `frame`'s display list, for a
    /// surface of `width` by `height` physical pixels.
    ///
    /// An image outside the surface or its clips is left out of the scene,
    /// because vello packs every image of a scene into one atlas, visible or
    /// not, and skips the images that no longer fit.
    pub(crate) fn encode(&mut self, frame: &FrameOutput<'_>, width: u32, height: u32) {
        self.scene.reset();
        // Images the last two frames did not use leave the cache at the
        // start of the third frame after their last use. That releases only
        // this cache's reference: vello's atlas keeps its own until it
        // needs the space for another image.
        self.images.generation += 1;
        let generation = self.images.generation;
        self.images
            .brushes
            .retain(|_, (_, stamp)| generation - *stamp <= 2);
        let transform = Affine::scale(f64::from(frame.scale));
        // The display list is balanced by construction, but a stray `PopClip`
        // in a release build would pop a vello layer that was never pushed,
        // so the depth is tracked here and leftover clips are closed before
        // rendering.
        let mut depth = 0usize;
        // The translation the offset scopes around an item apply to it, which
        // composes with the scale instead of costing a layer per scope.
        let mut offset = Affine::IDENTITY;
        self.offsets.clear();
        // The part of the surface the clips around an item leave visible.
        let mut visible = KurboRect::new(0.0, 0.0, f64::from(width), f64::from(height));
        self.clips.clear();
        for item in frame.list.items() {
            match item {
                DisplayItem::Op(op) => encode_op(
                    &mut self.scene,
                    &mut self.fonts,
                    &mut self.ring,
                    &mut self.images,
                    transform * offset,
                    visible,
                    op,
                ),
                DisplayItem::PushClip(rect) => {
                    depth += 1;
                    let clip = kurbo_rect(*rect);
                    self.scene
                        .push_clip_layer(Fill::NonZero, transform * offset, &clip);
                    self.clips.push(visible);
                    visible = visible.intersect((transform * offset).transform_rect_bbox(clip));
                }
                DisplayItem::PopClip => {
                    if depth > 0 {
                        depth -= 1;
                        self.scene.pop_layer();
                        visible = self.clips.pop().unwrap_or(visible);
                    }
                }
                DisplayItem::PushOffset(delta) => {
                    self.offsets.push(offset);
                    offset *= Affine::translate((f64::from(delta.x), f64::from(delta.y)));
                }
                DisplayItem::PopOffset => {
                    // A stray pop in a release build keeps the offset it has,
                    // as with clips.
                    offset = self.offsets.pop().unwrap_or(offset);
                }
            }
        }
        for _ in 0..depth {
            self.scene.pop_layer();
        }
    }

    /// Returns the encoded scene of the last [`Encoder::encode`] call.
    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }
}

/// Encodes one paint operation.
///
/// The transform scales logical coordinates to physical pixels and carries
/// the translation of the offset scopes around the operation. `visible` is
/// the part of the surface the clips around the operation leave, in physical
/// pixels.
fn encode_op(
    scene: &mut Scene,
    fonts: &mut HashMap<(u64, u32), Option<FontData>>,
    ring: &mut BezPath,
    images: &mut ImageCache,
    transform: Affine,
    visible: KurboRect,
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
            // The border is the ring between the rectangle and the
            // rectangle shrunk by the width, filled even-odd, which is the
            // coverage the CPU renderer gives it: square corners stay square
            // where a stroke would join them round. A border that leaves no
            // inner rectangle covers the rectangle whole.
            match inset(rect, radius, width) {
                Some((inner, inner_radius)) => {
                    ring.truncate(0);
                    ring.extend(rounded(rect, radius).path_elements(PATH_TOLERANCE));
                    ring.extend(rounded(inner, inner_radius).path_elements(PATH_TOLERANCE));
                    scene.fill(Fill::EvenOdd, transform, peniko_color(color), None, &*ring);
                }
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
        PaintOp::Image {
            rect,
            radius,
            ref image,
        } => {
            let finite = [rect.x0, rect.y0, rect.x1, rect.y1]
                .iter()
                .all(|edge| edge.is_finite());
            if !(finite && rect.width() > 0.0 && rect.height() > 0.0) {
                return;
            }
            let drawn = transform.transform_rect_bbox(kurbo_rect(rect));
            let shown = drawn.intersect(visible);
            if !(shown.width() > 0.0 && shown.height() > 0.0) {
                return;
            }
            let brush = images.brush(image, drawn.width(), drawn.height());
            let sx = f64::from(rect.width()) / f64::from(brush.image.width);
            let sy = f64::from(rect.height()) / f64::from(brush.image.height);
            let local = transform
                * Affine::translate((f64::from(rect.x0), f64::from(rect.y0)))
                * Affine::scale_non_uniform(sx, sy);
            if radius != Corners::ZERO {
                scene.push_clip_layer(Fill::NonZero, transform, &rounded(rect, radius));
            }
            scene.draw_image(brush, local);
            if radius != Corners::ZERO {
                scene.pop_layer();
            }
        }
        // An operation added after this renderer draws nothing here.
        _ => {}
    }
}

/// How far a flattened corner arc may stray from the true arc, in logical
/// pixels.
const PATH_TOLERANCE: f64 = 0.01;

/// Returns `rect` with `radius` as kurbo's rounded rectangle.
fn rounded(rect: Rect, radius: Corners) -> RoundedRect {
    RoundedRect::from_rect(kurbo_rect(rect), kurbo_radii(radius))
}

/// The longest side vello's image atlas can hold.
///
/// Vello packs every image of a frame into one atlas texture that grows to
/// at most this many pixels on a side, and skips an image that does not fit.
const ATLAS_SIDE: u32 = 8192;

/// The images handed to vello, kept across frames.
///
/// A brush is kept because vello keys its atlas by the blob's identity, and
/// a fresh blob per frame would upload every image again each frame.
struct ImageCache {
    /// The brushes, keyed by the smudge image's id and the longer side of
    /// the brush, each stamped with the generation it was last used at.
    brushes: HashMap<(u64, u32), (ImageBrush, u64)>,
    /// How many frames the encoder has encoded, for the stamps.
    generation: u64,
    /// The longest side an image is handed to vello at, because vello skips
    /// an image its atlas cannot hold.
    max_side: u32,
}

impl ImageCache {
    /// Returns the vello image for `image` drawn over `drawn_width` by
    /// `drawn_height` physical pixels, building it the first time and
    /// stamping the current generation on it.
    ///
    /// The stamp is what drops the entries an application stopped drawing:
    /// the encoder drops an image at the start of the third frame after the
    /// last frame that drew it. An image larger than [`fitted_side`] allows
    /// is handed over as a copy downscaled to fit, because one vello atlas
    /// holds every image of a frame and a full-size photo fills a quarter of
    /// it. The copy is made when its entry is built, so an image that leaves
    /// the cache and comes back is downscaled again. An image that fits
    /// shares its pixels with the blob rather than copying them.
    fn brush(
        &mut self,
        image: &smudge_core::image::Image,
        drawn_width: f64,
        drawn_height: f64,
    ) -> &ImageBrush {
        let (width, height) = (image.width(), image.height());
        let side = fitted_side(width, height, drawn_width, drawn_height, self.max_side);
        let entry = self.brushes.entry((image.id(), side)).or_insert_with(|| {
            let (to_width, to_height) = fitted_size(width, height, side);
            let data = if (to_width, to_height) == (width, height) {
                Blob::new(Arc::new(SharedPixels(image.clone())))
            } else {
                Blob::new(Arc::new(downscale(
                    image.pixels(),
                    width,
                    height,
                    to_width,
                    to_height,
                )))
            };
            let brush = ImageBrush::new(ImageData {
                data,
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::AlphaPremultiplied,
                width: to_width,
                height: to_height,
            });
            (brush, 0)
        });
        entry.1 = self.generation;
        &entry.0
    }
}

/// Returns the longer side a `width` by `height` image is handed to vello
/// at when it is drawn over `drawn_width` by `drawn_height` physical pixels.
///
/// The side keeps at least one texel per drawn pixel on both axes, rounded
/// up to a power of two so a small change of the drawn size reuses the same
/// copy, and is capped at `max_side` and at the image's own longer side.
fn fitted_side(width: u32, height: u32, drawn_width: f64, drawn_height: f64, max_side: u32) -> u32 {
    let longer = width.max(height);
    let ratio = (drawn_width / f64::from(width)).max(drawn_height / f64::from(height));
    let needed = (f64::from(longer) * ratio).ceil();
    // A NaN fails the comparison and falls back to the full size.
    let bucket = if needed < f64::from(max_side) {
        (needed as u32).max(1).next_power_of_two()
    } else {
        max_side
    };
    bucket.min(max_side).min(longer)
}

/// Returns the size a `width` by `height` image is handed to vello at: its
/// own when both sides fit within `max_side`, otherwise the size whose
/// longer side is `max_side` and whose shorter side keeps the aspect ratio,
/// rounded to the nearest pixel and never below one.
fn fitted_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    if width <= max_side && height <= max_side {
        return (width, height);
    }
    let scaled = |short: u32, long: u32| {
        let side = (u64::from(short) * u64::from(max_side) + u64::from(long) / 2) / u64::from(long);
        u32::try_from(side).unwrap_or(max_side).clamp(1, max_side)
    };
    if width >= height {
        (max_side, scaled(height, width))
    } else {
        (scaled(width, height), max_side)
    }
}

/// Shrinks premultiplied RGBA8 pixels from `width` by `height` to `to_width`
/// by `to_height`, each no larger than the source, by averaging the block of
/// source pixels every target pixel covers.
///
/// Premultiplied channels average without weighting by alpha. The blocks
/// partition the source, so the work is one pass over its pixels, and the
/// returned buffer is the only allocation.
fn downscale(pixels: &[u8], width: u32, height: u32, to_width: u32, to_height: u32) -> Vec<u8> {
    let (width, height) = (u64::from(width), u64::from(height));
    let (to_width, to_height) = (u64::from(to_width), u64::from(to_height));
    let mut out = Vec::with_capacity((to_width * to_height * 4) as usize);
    for ty in 0..to_height {
        let (y0, y1) = (ty * height / to_height, (ty + 1) * height / to_height);
        for tx in 0..to_width {
            let (x0, x1) = (tx * width / to_width, (tx + 1) * width / to_width);
            let mut sums = [0u64; 4];
            for y in y0..y1 {
                let row = (y * width) as usize * 4;
                let block = &pixels[row + x0 as usize * 4..row + x1 as usize * 4];
                for pixel in block.as_chunks::<4>().0 {
                    for (sum, &channel) in sums.iter_mut().zip(pixel) {
                        *sum += u64::from(channel);
                    }
                }
            }
            let count = (x1 - x0) * (y1 - y0);
            out.extend(sums.map(|sum| ((sum + count / 2) / count) as u8));
        }
    }
    out
}

/// An image's pixels handed to vello as blob data, shared with the image
/// rather than copied.
struct SharedPixels(smudge_core::image::Image);

impl AsRef<[u8]> for SharedPixels {
    fn as_ref(&self) -> &[u8] {
        self.0.pixels()
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
/// A non-finite extent counts as nothing remaining, so the border of a broken
/// rectangle fills the rectangle whole instead of reaching vello as a ring.
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
/// The face is parsed once here because vello unwraps that parse, and a run
/// in a face that does not parse draws nothing, as on the CPU.
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
pub(crate) fn peniko_color(color: Color) -> PenikoColor {
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

#[cfg(test)]
#[path = "encoder_tests.rs"]
mod tests;
