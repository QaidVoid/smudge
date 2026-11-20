//! Encoding a display list into a vello scene, shared by the offscreen
//! renderer and the surface presenter.

use std::collections::HashMap;
use std::sync::Arc;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::{DisplayItem, EMBOLDEN_MITER_LIMIT, FontBlob, PaintOp};
use smudge_core::path::{
    FillRule, LineCap, LineJoin, Path, PathPaint, Segment, Stroke as PathStroke,
};
use smudge_core::style::Edges;
use vello::kurbo::{
    Affine, BezPath, Cap, CubicBez, Diagonal2, Join, ParamCurve, Point as KurboPoint,
    Rect as KurboRect, RoundedRect, RoundedRectRadii, Stroke as KurboStroke,
};
use vello::peniko::{
    BlendMode, Blob, Color as PenikoColor, Fill, FontData, ImageAlphaType, ImageBrush, ImageData,
    ImageFormat,
};
use vello::{FontEmbolden, Glyph, Scene};

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
    /// Scratch for the outline of a border ring or a path, reused across
    /// operations.
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
        self.append(frame, width, height);
    }

    /// Adds `frame`'s display list to the scene over what it already
    /// holds, for a surface of `width` by `height` physical pixels, as a
    /// list drawn over another one, such as the devtools overlay, needs.
    pub(crate) fn append(&mut self, frame: &FrameOutput<'_>, width: u32, height: u32) {
        let transform = Affine::scale(f64::from(frame.scale));
        // The display list is balanced by construction, but a stray `PopClip`
        // or `PopGroup` in a release build would pop a vello layer that was
        // never pushed, so the depths are tracked here and leftover layers
        // are closed before rendering.
        let mut depth = 0usize;
        let mut groups = 0usize;
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
                DisplayItem::PushGroup(opacity) => {
                    groups += 1;
                    let opacity = if opacity.is_nan() {
                        1.0
                    } else {
                        opacity.clamp(0.0, 1.0)
                    };
                    self.scene.push_layer(
                        Fill::NonZero,
                        BlendMode::default(),
                        opacity,
                        Affine::IDENTITY,
                        &visible,
                    );
                }
                DisplayItem::PopGroup if groups > 0 => {
                    groups -= 1;
                    self.scene.pop_layer();
                }
                // An item added after this renderer draws nothing here.
                _ => {}
            }
        }
        for _ in 0..depth + groups {
            self.scene.pop_layer();
        }
    }

    /// Returns the scene of the last [`Encoder::encode`] call, with the
    /// lists [`Encoder::append`] added over it since.
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
            border(
                scene,
                ring,
                transform,
                color,
                rect,
                radius,
                Edges::all(width),
            );
        }
        PaintOp::StrokeSides {
            rect,
            radius,
            widths,
            color,
        } => {
            let side = |width: f32| {
                if width.is_finite() && width > 0.0 {
                    width
                } else {
                    0.0
                }
            };
            let widths = Edges {
                top: side(widths.top),
                right: side(widths.right),
                bottom: side(widths.bottom),
                left: side(widths.left),
            };
            if widths == Edges::all(0.0) {
                return;
            }
            border(scene, ring, transform, color, rect, radius, widths);
        }
        PaintOp::Glyphs(ref run) => {
            if !run.size.is_finite() || run.size <= 0.0 {
                return;
            }
            let Some(font) = font_data(fonts, &run.font) else {
                return;
            };
            let (embolden, skew) = run.synthesis();
            let embolden = f64::from(embolden);
            let slant =
                (skew != 0.0).then(|| Affine::skew(f64::from(skew).to_radians().tan(), 0.0));
            scene
                .draw_glyphs(font)
                .transform(transform)
                .font_size(run.size)
                .normalized_coords(&run.coords)
                .font_embolden(
                    FontEmbolden::new(Diagonal2::new(embolden, embolden))
                        .with_miter_limit(f64::from(EMBOLDEN_MITER_LIMIT)),
                )
                // The glyph transform applies in the outline's own space,
                // where y grows up, so a positive shear leans the top right.
                .glyph_transform(slant)
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
        PaintOp::Path(ref paint) => draw_path(scene, ring, transform, paint),
        // An operation added after this renderer draws nothing here.
        _ => {}
    }
}

/// Fills and strokes `paint`, the fill first.
fn draw_path(scene: &mut Scene, path: &mut BezPath, transform: Affine, paint: &PathPaint) {
    if !paint.path.is_drawable() {
        return;
    }
    kurbo_path(&paint.path, path);
    let path = &*path;
    if let Some(color) = paint.fill {
        let rule = match paint.fill_rule {
            FillRule::EvenOdd => Fill::EvenOdd,
            _ => Fill::NonZero,
        };
        scene.fill(rule, transform, peniko_color(color), None, path);
    }
    if let Some(stroke) = paint.stroke.as_ref().filter(|stroke| stroke.draws()) {
        scene.stroke(
            &kurbo_stroke(stroke, &paint.path),
            transform,
            peniko_color(stroke.color),
            None,
            path,
        );
    }
}

/// Writes `path` into `bez` as kurbo's path type, replacing what it held.
fn kurbo_path(path: &Path, bez: &mut BezPath) {
    let point = |point: smudge_core::geometry::Point| {
        KurboPoint::new(f64::from(point.x), f64::from(point.y))
    };
    bez.truncate(0);
    for segment in path.segments() {
        match *segment {
            Segment::MoveTo(to) => bez.move_to(point(to)),
            Segment::LineTo(to) => bez.line_to(point(to)),
            Segment::QuadTo(control, to) => bez.quad_to(point(control), point(to)),
            Segment::CubicTo(first, second, to) => {
                bez.curve_to(point(first), point(second), point(to));
            }
            Segment::Close => bez.close_path(),
            // A segment added after this renderer draws nothing here.
            _ => {}
        }
    }
}

/// Converts a stroke to kurbo's stroke style.
fn kurbo_stroke(stroke: &PathStroke, path: &Path) -> KurboStroke {
    let mut style = KurboStroke::new(f64::from(stroke.width))
        .with_miter_limit(f64::from(stroke.effective_miter_limit()))
        .with_join(match stroke.join {
            LineJoin::Round => Join::Round,
            LineJoin::Bevel => Join::Bevel,
            _ => Join::Miter,
        })
        .with_caps(match stroke.cap {
            LineCap::Round => Cap::Round,
            LineCap::Square => Cap::Square,
            _ => Cap::Butt,
        });
    if let Some(dash) = &stroke.dash
        && let Some(pattern) = dash.pattern(path)
    {
        style = style.with_dashes(f64::from(dash.start()), pattern.map(f64::from));
    }
    style
}

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

/// Fills the border inside `rect` whose sides are `widths` thick.
///
/// The border is the area between the rectangle's outline and the inner
/// outline [`inset`] gives, which runs the other way round, filled with the
/// non-zero rule. That is the coverage the CPU renderer gives it, and the
/// edges the two outlines share, along a side of width zero, cancel instead
/// of drawing a line. A border that leaves no inner shape covers the
/// rectangle whole.
fn border(
    scene: &mut Scene,
    ring: &mut BezPath,
    transform: Affine,
    color: Color,
    rect: Rect,
    radius: Corners,
    widths: Edges<f32>,
) {
    let outer_radius = fitted(rect, radius);
    let outer = outline(rect, outer_radius, outer_radius);
    let inner = inset(rect, outer_radius, widths);
    let Some(inner) = inner.and_then(|(inner, across, down)| outline(inner, across, down)) else {
        fill(scene, transform, color, rect, radius);
        return;
    };
    let Some(mut outer) = outer else {
        return;
    };
    // Where an inner corner is cut from the outer corner's own quarter, the
    // outer corner is drawn in pieces at the same cuts, so the piece the two
    // share is the same cubic in both outlines and cancels exactly.
    for (outer, inner) in outer.iter_mut().zip(&inner) {
        if let (
            Bend::Curve {
                quarter, cuts: own, ..
            },
            Bend::Curve {
                quarter: cut,
                from,
                to,
                ..
            },
        ) = (outer, inner)
            && quarter == cut
        {
            *own = Some((*from, *to));
        }
    }
    ring.truncate(0);
    push_outline(ring, &outer, false);
    push_outline(ring, &inner, true);
    scene.fill(Fill::NonZero, transform, peniko_color(color), None, &*ring);
}

/// Returns `radius` with each corner held between zero and half the shorter
/// side of `rect`, as the CPU renderer fits it.
fn fitted(rect: Rect, radius: Corners) -> Corners {
    let limit = (rect.width().min(rect.height()) / 2.0).max(0.0);
    let fit = |radius: f32| radius.max(0.0).min(limit);
    Corners {
        top_left: fit(radius.top_left),
        top_right: fit(radius.top_right),
        bottom_right: fit(radius.bottom_right),
        bottom_left: fit(radius.bottom_left),
    }
}

/// Returns `rect` shrunk by each side's width of `by`, with the horizontal
/// and the vertical radius of each inner corner, as the CPU renderer insets
/// it, or `None` when nothing remains.
///
/// Each inner corner is the quarter ellipse centered where the fitted outer
/// corner `radius` is centered: its horizontal radius is the outer radius
/// less the width of the side beside it, its vertical radius the outer
/// radius less the width of the side above or below it, and it is square
/// when either comes to zero or less. A non-finite extent counts as nothing
/// remaining, so the border of a broken rectangle fills the rectangle whole
/// instead of reaching vello as a ring.
fn inset(rect: Rect, radius: Corners, by: Edges<f32>) -> Option<(Rect, Corners, Corners)> {
    let rect = Rect::new(
        rect.x0 + by.left,
        rect.y0 + by.top,
        rect.x1 - by.right,
        rect.y1 - by.bottom,
    );
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return None;
    }
    let corner = |radius: f32, across: f32, down: f32| {
        let (horizontal, vertical) = (radius - across, radius - down);
        if horizontal > 0.0 && vertical > 0.0 {
            (horizontal, vertical)
        } else {
            (0.0, 0.0)
        }
    };
    let top_left = corner(radius.top_left, by.left, by.top);
    let top_right = corner(radius.top_right, by.right, by.top);
    let bottom_right = corner(radius.bottom_right, by.right, by.bottom);
    let bottom_left = corner(radius.bottom_left, by.left, by.bottom);
    let across = Corners {
        top_left: top_left.0,
        top_right: top_right.0,
        bottom_right: bottom_right.0,
        bottom_left: bottom_left.0,
    };
    let down = Corners {
        top_left: top_left.1,
        top_right: top_right.1,
        bottom_right: bottom_right.1,
        bottom_left: bottom_left.1,
    };
    Some((rect, across, down))
}

/// How far a cubic's control points sit from the ends of the quarter circle
/// or ellipse it stands for, as a share of each radius.
const QUARTER_ARC: f64 = 0.552_284_749_830_793_6;

/// One corner of an outline, in the order a clockwise walk meets its ends.
#[derive(Clone, Copy)]
enum Bend {
    /// A square corner, at this point.
    Square(KurboPoint),
    /// The part of the corner's quarter curve, drawn clockwise as one cubic,
    /// that lies inside the rectangle.
    Curve {
        /// The whole quarter, clockwise on the screen.
        quarter: CubicBez,
        /// Where along the quarter the part inside the rectangle starts.
        from: f64,
        /// Where along the quarter the part inside the rectangle ends.
        to: f64,
        /// Two places between `from` and `to` to draw the part in three
        /// pieces at, so a piece matches another outline's part exactly.
        cuts: Option<(f64, f64)>,
    },
}

impl Bend {
    /// Where a walk in the given direction enters this corner.
    fn entry(self, reverse: bool) -> KurboPoint {
        match self {
            Bend::Square(point) => point,
            Bend::Curve {
                quarter, from, to, ..
            } => quarter.eval(if reverse { to } else { from }),
        }
    }
}

/// Returns the part of `0..=1` where the monotonic `along` stays between
/// `low` and `high`, or an empty range, whose start is past its end, when it
/// never does.
fn within(along: impl Fn(f64) -> f64, low: f64, high: f64) -> (f64, f64) {
    const SLACK: f64 = 1e-7;
    let (start, end) = (along(0.0), along(1.0));
    let rising = end >= start;
    let (least, most) = if rising { (start, end) } else { (end, start) };
    if most < low - SLACK || least > high + SLACK {
        return (1.0, 0.0);
    }
    // The parameter where `along` reaches `value`, which it crosses once.
    let crossing = |value: f64| {
        let (mut below, mut above) = (0.0, 1.0);
        for _ in 0..50 {
            let middle = (below + above) / 2.0;
            if (along(middle) < value) == rising {
                below = middle;
            } else {
                above = middle;
            }
        }
        (below + above) / 2.0
    };
    let (enter, leave) = if rising { (low, high) } else { (high, low) };
    let first = if (start - enter) * if rising { 1.0 } else { -1.0 } >= -SLACK {
        0.0
    } else {
        crossing(enter)
    };
    let last = if (leave - end) * if rising { 1.0 } else { -1.0 } >= -SLACK {
        1.0
    } else {
        crossing(leave)
    };
    (first, last)
}

/// Returns the four corners of `rect`, clockwise from the top left, whose
/// horizontal and vertical radii are `across` and `down`, or `None` when the
/// shape they leave is empty.
///
/// Each corner's quarter curve is centered its radii in from the corner, so
/// it meets the two sides beside it. Where a side is so wide that the curve
/// reaches past the side across from it, the curve stops at that side, so
/// the outline follows the shape inside the rectangle and inside every
/// corner's curve. When a corner's curve lies wholly outside the rectangle,
/// the corner covers it all and nothing is left. Every corner is cut from
/// the one cubic its whole quarter would be, so an inner corner that
/// follows an outer one along a side of width zero is the very same curve.
fn outline(rect: Rect, across: Corners, down: Corners) -> Option<[Bend; 4]> {
    let (x0, y0) = (f64::from(rect.x0), f64::from(rect.y0));
    let (x1, y1) = (f64::from(rect.x1), f64::from(rect.y1));
    // Each corner as its point, the point a clockwise walk reaches its curve
    // at, and the point it leaves at, given its radii.
    let corner = |point: (f64, f64),
                  radii: (f32, f32),
                  ends: &dyn Fn(f64, f64) -> ((f64, f64), (f64, f64))| {
        let (rx, ry) = (f64::from(radii.0), f64::from(radii.1));
        if rx <= 0.0 || ry <= 0.0 {
            return Some(Bend::Square(KurboPoint::new(point.0, point.1)));
        }
        let (reach, leave) = ends(rx, ry);
        let toward = |end: (f64, f64)| {
            KurboPoint::new(
                end.0 + QUARTER_ARC * (point.0 - end.0),
                end.1 + QUARTER_ARC * (point.1 - end.1),
            )
        };
        let quarter = CubicBez::new(
            KurboPoint::new(reach.0, reach.1),
            toward(reach),
            toward(leave),
            KurboPoint::new(leave.0, leave.1),
        );
        let (x_from, x_to) = within(|t| quarter.eval(t).x, x0, x1);
        let (y_from, y_to) = within(|t| quarter.eval(t).y, y0, y1);
        let (from, to) = (x_from.max(y_from), x_to.min(y_to));
        (from <= to).then_some(Bend::Curve {
            quarter,
            from,
            to,
            cuts: None,
        })
    };
    Some([
        corner((x0, y0), (across.top_left, down.top_left), &|rx, ry| {
            ((x0, y0 + ry), (x0 + rx, y0))
        })?,
        corner((x1, y0), (across.top_right, down.top_right), &|rx, ry| {
            ((x1 - rx, y0), (x1, y0 + ry))
        })?,
        corner(
            (x1, y1),
            (across.bottom_right, down.bottom_right),
            &|rx, ry| ((x1, y1 - ry), (x1 - rx, y1)),
        )?,
        corner(
            (x0, y1),
            (across.bottom_left, down.bottom_left),
            &|rx, ry| ((x0 + rx, y1), (x0, y1 - ry)),
        )?,
    ])
}

/// Appends the closed outline of `bends` to `path`, clockwise on the screen,
/// or counter-clockwise when `reverse` is set.
fn push_outline(path: &mut BezPath, bends: &[Bend; 4], reverse: bool) {
    let mut started = false;
    let mut visit = |path: &mut BezPath, bend: Bend| {
        let entry = bend.entry(reverse);
        if started {
            path.line_to(entry);
        } else {
            path.move_to(entry);
            started = true;
        }
        if let Bend::Curve {
            quarter,
            from,
            to,
            cuts,
        } = bend
        {
            let (first, second) = cuts.unwrap_or((to, to));
            let mut pieces = [(from, first), (first, second), (second, to)];
            if reverse {
                pieces.reverse();
            }
            for (start, end) in pieces {
                if end <= start {
                    continue;
                }
                let part = quarter.subsegment(start..end);
                if reverse {
                    path.curve_to(part.p2, part.p1, part.p0);
                } else {
                    path.curve_to(part.p1, part.p2, part.p3);
                }
            }
        }
    };
    if reverse {
        for &bend in bends.iter().rev() {
            visit(path, bend);
        }
    } else {
        for &bend in bends {
            visit(path, bend);
        }
    }
    path.close_path();
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
