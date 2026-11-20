//! Drawing display items into the surface.
//!
//! Every pixel's coverage is computed from its own absolute position and the
//! shape that covers it, never from the area being drawn. Drawing any part of
//! the surface on its own therefore produces exactly the bytes a full repaint
//! would, however the damage is split. Glyphs meet the same rule through their
//! masks, which are rasterized whole and depend on nothing but the glyph, its
//! size and its subpixel phase.

use std::ops::Range;

use smudge_core::geometry::{Color, Corners, PixelRect, Point, Rect};
use smudge_core::paint::{DisplayItem, GlyphRun, PaintOp};
use smudge_core::path::{FillRule, LineCap, LineJoin, Path, PathPaint, Segment, Stroke};
use smudge_core::style::Edges;

use crate::glyph_cache::{GlyphCache, MAX_GLYPH_SIZE, MaskKey};

/// The largest glyph position, in physical pixels from the surface origin,
/// that is placed at all.
///
/// It only keeps the integer placement of a glyph from overflowing. Ordinary
/// fonts keep their bearings within a few tens of thousands of pixels, so no
/// glyph they shape onto a surface comes near it. Culling by distance is left
/// to the clip, because a crafted font's bearing has no useful bound.
const PLACEMENT_LIMIT: f32 = 1.0e9;

/// The surface a display list walk converts logical coordinates against.
#[derive(Copy, Clone)]
pub(crate) struct Surface {
    /// Physical pixels per logical pixel.
    pub(crate) scale: f32,
    /// The surface width in physical pixels.
    pub(crate) width: u32,
    /// The surface height in physical pixels.
    pub(crate) height: u32,
}

/// Translates `rect` by `offset`, unless the offset is zero, which is what
/// operations outside every offset scope carry.
fn maybe_translated(rect: Rect, offset: Point) -> Rect {
    if offset == Point::default() {
        rect
    } else {
        rect.translated(offset)
    }
}

/// What a walk of a display list hands its caller, in paint order.
pub(crate) enum Walked<'a> {
    /// A drawable operation.
    Op {
        /// The index of the operation's item in the display list.
        index: usize,
        /// The operation.
        op: &'a PaintOp,
        /// The physical pixels the operation can change.
        area: PixelRect,
        /// The logical offset the operation is drawn at.
        offset: Point,
    },
    /// The start of a group drawn at this opacity.
    Open(f32),
    /// The end of the group the last unclosed [`Walked::Open`] started.
    Close,
}

/// Walks `items` in paint order under `root`, calling `on_item` for every
/// drawable operation with the physical pixels it can change and the logical
/// offset the operation is drawn at, and for the start and the end of every
/// group.
///
/// `root` is the rectangle the walk repaints, usually a damage rectangle or
/// the whole surface: every area handed over lies inside it. Each operation's
/// area is its [`PaintOp::bounds`] in physical pixels, intersected with every
/// clip around it and clamped to the surface, so an operation outside `root`,
/// outside the surface, or under an empty clip draws nothing and is never
/// handed over. A clip whose logical rectangle has no area is empty, even
/// where rounding its edges outward would cover pixels. An offset scope moves
/// the operations and the clips it wraps, and the offsets it sits inside
/// stack up. The walk visits clips, offsets and operations once, in list
/// order, so a caller pays one bounds conversion per operation however many
/// rectangles it repaints.
pub(crate) fn walk<'a>(
    mut on_item: impl FnMut(Walked<'a>),
    clips: &mut Vec<Option<PixelRect>>,
    offsets: &mut Vec<Point>,
    items: &'a [DisplayItem],
    surface: Surface,
    root: PixelRect,
) {
    clips.clear();
    clips.push(Some(root));
    offsets.clear();
    // The clip that operations see, always the top of `clips`, and the offset
    // operations and clips are translated by, the sum of the open scopes.
    let mut current = Some(root);
    let mut offset = Point::default();
    for (index, item) in items.iter().enumerate() {
        match item {
            DisplayItem::Op(op) => {
                if let Some(clip) = current
                    && let Some(area) = PixelRect::from_logical(
                        maybe_translated(op.bounds(), offset),
                        surface.scale,
                        surface.width,
                        surface.height,
                    )
                    .and_then(|area| area.intersect(clip))
                {
                    on_item(Walked::Op {
                        index,
                        op,
                        area,
                        offset,
                    });
                }
            }
            DisplayItem::PushClip(rect) => {
                // A clip without area lets nothing through, even where its
                // edges would round outward to a column or row of pixels.
                let inner = current.filter(|_| !rect.is_empty()).and_then(|outer| {
                    PixelRect::from_logical(
                        maybe_translated(*rect, offset),
                        surface.scale,
                        surface.width,
                        surface.height,
                    )
                    .and_then(|inner| inner.intersect(outer))
                });
                clips.push(inner);
                current = inner;
            }
            DisplayItem::PopClip => {
                // The root clip stays, so a list with more pops than pushes
                // keeps `root` as the clip.
                if clips.len() > 1 {
                    clips.pop();
                    current = clips.last().copied().flatten();
                }
            }
            DisplayItem::PushOffset(delta) => {
                offsets.push(offset);
                offset = Point {
                    x: offset.x + delta.x,
                    y: offset.y + delta.y,
                };
            }
            DisplayItem::PopOffset => {
                // An unbalanced list keeps the offset it has, as with clips.
                offset = offsets.pop().unwrap_or(offset);
            }
            DisplayItem::PushGroup(opacity) => on_item(Walked::Open(*opacity)),
            DisplayItem::PopGroup => on_item(Walked::Close),
            // An item added after this renderer draws nothing here.
            _ => {}
        }
    }
}

/// One operation of a [`RasterPlan`].
#[derive(Copy, Clone)]
pub(crate) struct PlannedOp {
    /// The index of the operation's [`DisplayItem::Op`] in the display list.
    pub(crate) index: usize,
    /// The physical pixels the operation can change, already intersected with
    /// every clip around it and clamped to the surface.
    pub(crate) area: PixelRect,
    /// The logical offset the operation's geometry is drawn at, the sum of
    /// the offset scopes around it.
    pub(crate) offset: Point,
}

/// One step of a [`RasterPlan`].
#[derive(Copy, Clone)]
pub(crate) enum Step {
    /// A drawable operation.
    Op(PlannedOp),
    /// The start of a group drawn at `opacity`.
    Open {
        /// The opacity the group blends at, as the display list gives it,
        /// which the layer clamps to the range from 0.0 to 1.0.
        opacity: f32,
        /// The physical pixels the group's operations can change, or `None`
        /// when they change none.
        extent: Option<PixelRect>,
    },
    /// The end of the group the last unclosed [`Step::Open`] started.
    Close,
}

/// A display list's drawable operations with the pixels each can change,
/// and its groups with the pixels theirs can, collected in one walk.
///
/// Replaying the list once per damage rectangle costs a walk of every item
/// and a bounds conversion of every operation per rectangle, and every clip
/// rescan costs a walk of its subtree. Collecting the operations once per
/// frame turns both into a single walk plus an integer intersection test per
/// operation and rectangle. On the scroll scene, where sixteen damage
/// rectangles cover a list of about two thousand items, that cut the raster
/// time from about 435 to 108 microseconds per frame, below the 156 of a
/// full repaint.
pub(crate) struct RasterPlan {
    /// The steps, in paint order, with every group closed.
    steps: Vec<Step>,
    /// The index in `steps` of each group still open while the plan builds.
    open: Vec<usize>,
    /// The clip stack scratch of the walk, reused between builds and walks.
    pub(crate) clips: Vec<Option<PixelRect>>,
    /// The offset stack scratch of the walk, reused between builds and walks.
    pub(crate) offsets: Vec<Point>,
}

impl RasterPlan {
    /// Creates an empty plan.
    pub(crate) fn new() -> RasterPlan {
        RasterPlan {
            steps: Vec::new(),
            open: Vec::new(),
            clips: Vec::new(),
            offsets: Vec::new(),
        }
    }

    /// Collects the steps of `items` for a surface of `width` by `height`
    /// physical pixels at `scale`.
    ///
    /// A group's extent is the union of the areas of the operations inside
    /// it, nested groups included. A group the list leaves open is closed at
    /// the end, and a close with no group open is dropped, so the steps
    /// always balance. The caller treats the index of a planned operation
    /// as an operation item, which holds because only [`DisplayItem::Op`]
    /// entries become [`Step::Op`].
    pub(crate) fn build(&mut self, items: &[DisplayItem], scale: f32, width: u32, height: u32) {
        self.steps.clear();
        self.open.clear();
        let steps = &mut self.steps;
        let open = &mut self.open;
        walk(
            |item| match item {
                Walked::Op {
                    index,
                    area,
                    offset,
                    ..
                } => {
                    steps.push(Step::Op(PlannedOp {
                        index,
                        area,
                        offset,
                    }));
                    if let Some(&group) = open.last() {
                        widen(steps, group, area);
                    }
                }
                Walked::Open(opacity) => {
                    open.push(steps.len());
                    steps.push(Step::Open {
                        opacity,
                        extent: None,
                    });
                }
                Walked::Close => close(steps, open),
            },
            &mut self.clips,
            &mut self.offsets,
            items,
            Surface {
                scale,
                width,
                height,
            },
            PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            },
        );
        while !self.open.is_empty() {
            close(&mut self.steps, &mut self.open);
        }
    }

    /// Returns the steps in paint order.
    pub(crate) fn steps(&self) -> &[Step] {
        &self.steps
    }
}

/// Adds `area` to the extent of the group whose start is `steps[group]`.
fn widen(steps: &mut [Step], group: usize, area: PixelRect) {
    if let Step::Open { extent, .. } = &mut steps[group] {
        *extent = Some(extent.map_or(area, |extent| extent.union(area)));
    }
}

/// Closes the innermost group of `open`, if one is open, and adds its
/// extent to the group around it, which draws its pixels.
fn close(steps: &mut Vec<Step>, open: &mut Vec<usize>) {
    let Some(group) = open.pop() else {
        return;
    };
    steps.push(Step::Close);
    if let (
        Some(&parent),
        Step::Open {
            extent: Some(area), ..
        },
    ) = (open.last(), steps[group])
    {
        widen(steps, parent, area);
    }
}

/// The layers the open groups draw into while a rectangle is repainted,
/// with their buffers kept between frames.
pub(crate) struct Layers {
    /// The open layers, innermost last.
    open: Vec<Layer>,
    /// Buffers of closed layers, kept for the next group.
    spare: Vec<Vec<u8>>,
}

/// One open group's pixels.
struct Layer {
    /// Premultiplied RGBA8 pixels of `region`, row-major with no padding.
    pixels: Vec<u8>,
    /// The part of the surface the layer covers, or `None` when the group
    /// changes no pixel of the rectangle being repainted.
    region: Option<PixelRect>,
    /// The opacity the layer blends at, out of 255.
    opacity: u32,
}

impl Layers {
    /// Creates an empty stack.
    pub(crate) fn new() -> Layers {
        Layers {
            open: Vec::new(),
            spare: Vec::new(),
        }
    }

    /// Opens a transparent layer over `region` that blends at `opacity`.
    pub(crate) fn open(&mut self, region: Option<PixelRect>, opacity: f32) {
        let mut pixels = self.spare.pop().unwrap_or_default();
        pixels.clear();
        if let Some(region) = region {
            pixels.resize(region.area() as usize * 4, 0);
        }
        let opacity = if opacity.is_nan() {
            255
        } else {
            (opacity.clamp(0.0, 1.0) * 255.0).round() as u32
        };
        self.open.push(Layer {
            pixels,
            region,
            opacity,
        });
    }

    /// Closes the innermost layer and blends it into the layer under it, or
    /// into `base` when it was the outermost.
    pub(crate) fn close(&mut self, base: &mut Canvas<'_>) {
        let Some(layer) = self.open.pop() else {
            return;
        };
        if let Some(region) = layer.region {
            match self.open.last_mut() {
                Some(parent) => {
                    if let Some(parent_region) = parent.region {
                        composite(&layer, region, &mut parent.pixels, parent_region);
                    }
                }
                None => {
                    let surface = PixelRect {
                        x: 0,
                        y: 0,
                        width: base.width,
                        height: (base.pixels.len() / 4 / base.width.max(1) as usize) as u32,
                    };
                    composite(&layer, region, base.pixels, surface);
                }
            }
        }
        self.spare.push(layer.pixels);
    }

    /// Draws `op` into `area` of the innermost open layer, or of `base` when
    /// no layer is open. A layer that covers none of `area` takes nothing.
    pub(crate) fn draw(
        &mut self,
        base: &mut Canvas<'_>,
        op: &PaintOp,
        area: PixelRect,
        offset: Point,
    ) {
        let Some(layer) = self.open.last_mut() else {
            base.draw(op, area, offset);
            return;
        };
        let Some(region) = layer.region else {
            return;
        };
        let Some(area) = area.intersect(region) else {
            return;
        };
        let mut canvas = Canvas {
            pixels: &mut layer.pixels,
            width: region.width,
            left: region.x,
            top: region.y,
            surface: base.surface,
            scale: base.scale,
            glyphs: base.glyphs,
            paths: base.paths,
        };
        canvas.draw(op, area, offset);
    }
}

/// Blends the pixels of `layer`, which cover `region`, over the pixels of
/// `target`, which cover `target_region`, at the layer's opacity.
fn composite(layer: &Layer, region: PixelRect, target: &mut [u8], target_region: PixelRect) {
    let Some(region) = region.intersect(target_region) else {
        return;
    };
    let layer_region = layer.region.unwrap_or(region);
    for y in region.y..region.y + region.height {
        let source_start = ((y - layer_region.y) as usize * layer_region.width as usize
            + (region.x - layer_region.x) as usize)
            * 4;
        let target_start = ((y - target_region.y) as usize * target_region.width as usize
            + (region.x - target_region.x) as usize)
            * 4;
        let length = region.width as usize * 4;
        let source = layer.pixels[source_start..source_start + length]
            .as_chunks::<4>()
            .0;
        let target = target[target_start..target_start + length]
            .as_chunks_mut::<4>()
            .0;
        for (target, source) in target.iter_mut().zip(source) {
            if source[3] != 0 {
                blend_image(target, *source, layer.opacity);
            }
        }
    }
}

/// The pixels a draw lands in and the scale that logical coordinates
/// convert with.
pub(crate) struct Canvas<'a> {
    /// Premultiplied RGBA8 pixels, row-major with no padding.
    pub(crate) pixels: &'a mut [u8],
    /// The width of the pixels, in physical pixels.
    pub(crate) width: u32,
    /// The surface column of the pixels' first column.
    pub(crate) left: u32,
    /// The surface row of the pixels' first row.
    pub(crate) top: u32,
    /// The width and height of the whole surface, which the pixels lie in.
    pub(crate) surface: (u32, u32),
    /// Physical pixels per logical pixel.
    pub(crate) scale: f32,
    /// The cache of rasterized glyph masks, shared across frames.
    pub(crate) glyphs: &'a mut GlyphCache,
    /// The pixels a path is rasterized into, shared across frames.
    pub(crate) paths: &'a mut Vec<u8>,
}

impl Canvas<'_> {
    /// Fills `rect` with `color`, replacing what was there.
    pub(crate) fn clear(&mut self, rect: PixelRect, color: Color) {
        let alpha = u32::from(color.a);
        let pixel = [
            premultiply(color.r, alpha),
            premultiply(color.g, alpha),
            premultiply(color.b, alpha),
            color.a,
        ];
        for y in rect.y..rect.y + rect.height {
            let start = self.index(rect.x, y);
            let row = &mut self.pixels[start..start + rect.width as usize * 4];
            row.as_chunks_mut::<4>().0.fill(pixel);
        }
    }

    /// Draws `op` into `area`, the part of the operation's bounds inside the
    /// clip and the rectangle being repainted.
    ///
    /// `offset` is the logical translation the offset scopes around the
    /// operation apply to it: the operation's geometry is drawn moved by it,
    /// while `area` is where the moved operation can land.
    pub(crate) fn draw(&mut self, op: &PaintOp, area: PixelRect, offset: Point) {
        match *op {
            PaintOp::FillRect {
                rect,
                radius,
                color,
            } => {
                if let Some(shape) = Shape::new(maybe_translated(rect, offset), radius, self.scale)
                {
                    self.fill(
                        area,
                        color,
                        |x, y| shape.coverage(x, y),
                        |y| shape.steady(y),
                    );
                }
            }
            PaintOp::StrokeRect {
                rect,
                radius,
                width,
                color,
            } => {
                if !width.is_finite() || width <= 0.0 {
                    return;
                }
                self.ring(
                    area,
                    maybe_translated(rect, offset),
                    radius,
                    Edges::all(width),
                    color,
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
                self.ring(area, maybe_translated(rect, offset), radius, widths, color);
            }
            PaintOp::Glyphs(ref run) => self.draw_glyphs(run.as_ref(), area, offset),
            PaintOp::Image {
                rect,
                radius,
                ref image,
            } => self.draw_image(maybe_translated(rect, offset), radius, image, area),
            PaintOp::Path(ref paint) => self.draw_path(paint, area, offset),
            PaintOp::External {
                rect,
                radius,
                ref content,
            } => {
                if let Some(image) = &content.fallback {
                    self.draw_image(maybe_translated(rect, offset), radius, image, area);
                }
            }
            // An operation added after this renderer draws nothing here.
            _ => {}
        }
    }

    /// Draws `paint`, moved by the logical `offset`, into `area`.
    ///
    /// The path is rasterized with tiny-skia into a scratch buffer that
    /// covers the paint's bounds on the surface, and the part of it inside
    /// `area` blends over the pixels. The buffer's place depends on the
    /// paint and the surface alone, never on `area`, so a damaged repaint
    /// gives the same bytes as a full one: rasterizing just the area would
    /// move the path by the area's corner, which rounds its coordinates
    /// differently.
    fn draw_path(&mut self, paint: &PathPaint, area: PixelRect, offset: Point) {
        if !paint.path.is_drawable() {
            return;
        }
        let scale = self.scale;
        let bounds = maybe_translated(paint.bounds(), offset);
        let (surface_width, surface_height) = self.surface;
        let x0 = ((bounds.x0 * scale).floor().max(0.0) as u32).min(surface_width);
        let y0 = ((bounds.y0 * scale).floor().max(0.0) as u32).min(surface_height);
        let x1 = ((bounds.x1 * scale).ceil().max(0.0) as u32).min(surface_width);
        let y1 = ((bounds.y1 * scale).ceil().max(0.0) as u32).min(surface_height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let Some(path) = skia_path(&paint.path) else {
            return;
        };
        let (width, height) = (x1 - x0, y1 - y0);
        self.paths.clear();
        self.paths.resize(width as usize * height as usize * 4, 0);
        let Some(mut pixmap) = tiny_skia::PixmapMut::from_bytes(self.paths, width, height) else {
            return;
        };
        let transform = tiny_skia::Transform::from_row(
            scale,
            0.0,
            0.0,
            scale,
            offset.x * scale - x0 as f32,
            offset.y * scale - y0 as f32,
        );
        if let Some(color) = paint.fill {
            let rule = match paint.fill_rule {
                FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
                _ => tiny_skia::FillRule::Winding,
            };
            pixmap.fill_path(&path, &skia_paint(color), rule, transform, None);
        }
        if let Some(stroke) = paint.stroke.as_ref().filter(|stroke| stroke.draws()) {
            pixmap.stroke_path(
                &path,
                &skia_paint(stroke.color),
                &skia_stroke(stroke, &paint.path),
                transform,
                None,
            );
        }
        let Some(area) = area.intersect(PixelRect {
            x: x0,
            y: y0,
            width,
            height,
        }) else {
            return;
        };
        let source = self.paths.as_chunks::<4>().0;
        for y in area.y..area.y + area.height {
            let from = ((y - y0) * width + (area.x - x0)) as usize;
            let start = self.index(area.x, y);
            let row = &mut self.pixels[start..start + area.width as usize * 4];
            for (target, pixel) in row
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(&source[from..from + area.width as usize])
            {
                if pixel[3] != 0 {
                    blend_image(target, *pixel, 255);
                }
            }
        }
    }

    /// Draws `image` stretched over a rounded rectangle, into `area`.
    ///
    /// Each pixel of the area inside the shape is shaded with the image
    /// sampled bilinearly at the pixel's center mapped into image space, and
    /// multiplied by the rounded-rect coverage, which the fill path computes
    /// from the pixel's absolute position. Nothing here reads the edges of
    /// `area`, so a damaged repaint gives the same bytes as a full one.
    fn draw_image(
        &mut self,
        rect: Rect,
        radius: Corners,
        image: &smudge_core::image::Image,
        area: PixelRect,
    ) {
        let Some(shape) = Shape::new(rect, radius, self.scale) else {
            return;
        };
        let (img_width, img_height) = (image.width(), image.height());
        if img_width == 0 || img_height == 0 {
            return;
        }
        // The image stretches over the shape's bounds, so one texel is
        // `bounds / pixels` physical pixels wide, and a pixel center maps to
        // the sample position between the texel centers it sits between.
        let to_source_x = img_width as f32 / (shape.x1 - shape.x0);
        let to_source_y = img_height as f32 / (shape.y1 - shape.y0);
        let pixels = image.pixels();
        for y in area.y..area.y + area.height {
            let v = (y as f32 + 0.5 - shape.y0) * to_source_y - 0.5;
            for x in area.x..area.x + area.width {
                let coverage = shape.coverage(x, y);
                if coverage == 0.0 {
                    continue;
                }
                let u = (x as f32 + 0.5 - shape.x0) * to_source_x - 0.5;
                let sample = sample_bilinear(pixels, img_width, img_height, u, v);
                let cov = (coverage * 255.0 + 0.5) as u32;
                let start = self.index(x, y);
                let target = &mut self.pixels[start..start + 4];
                blend_image(
                    target.try_into().expect("a pixel is four bytes"),
                    sample,
                    cov,
                );
            }
        }
    }

    /// Blits every glyph of `run` that reaches into `area`, the part of the
    /// run's ink bounds inside the clip, with the run drawn moved by the
    /// logical `offset`.
    ///
    /// Each glyph is drawn from its cached alpha mask at its own physical
    /// position. A glyph's mask depends on the glyph, its variable instance,
    /// its quantized size, embolden and slant, and its subpixel x phase only,
    /// so the same text blits the same bytes whichever part of the surface is
    /// drawn. Glyphs whose ink bounds claim no pixels draw nothing, which keeps
    /// a run inside the damage its bounds produced even when a font's metrics
    /// disagree with its outlines.
    ///
    /// A glyph whose size or position is not finite, whose size exceeds
    /// [`MAX_GLYPH_SIZE`] physical pixels, or whose origin lies more than
    /// [`PLACEMENT_LIMIT`] physical pixels from the surface origin draws
    /// nothing, and so does a glyph whose mask, widened and slanted, would be
    /// wider or taller than the largest mask the cache rasterizes.
    fn draw_glyphs(&mut self, run: &GlyphRun, area: PixelRect, offset: Point) {
        let size_px = run.size * self.scale;
        if !run.size.is_finite()
            || run.size <= 0.0
            || !size_px.is_finite()
            || size_px <= 0.0
            || size_px > MAX_GLYPH_SIZE
        {
            return;
        }
        // The size the masks are rasterized at, quantized to 1/64 of a pixel,
        // is what the cache keys a glyph by.
        let size = (size_px * 64.0).round() as i32;
        if size <= 0 {
            return;
        }
        let (embolden, skew) = run.synthesis();
        let embolden = (embolden * self.scale * 64.0).round() as i32;
        let skew = (skew * 64.0).round() as i32;
        let instance = self.glyphs.instance(&run.coords);
        let Canvas {
            pixels,
            width,
            left,
            top,
            scale,
            glyphs,
            ..
        } = self;
        let scale = *scale;
        let shift_x = offset.x * scale;
        let shift_y = offset.y * scale;
        for glyph in &run.glyphs {
            let x = glyph.x * scale + shift_x;
            let y = (glyph.y * scale + shift_y).round();
            // Only positions far past any surface are skipped, so the integer
            // placement below cannot overflow. A bearing is not bounded by
            // the mask size, so nothing closer may be skipped, and the clip to
            // `area` handles everything in between.
            if !(-PLACEMENT_LIMIT..=PLACEMENT_LIMIT).contains(&x)
                || !(-PLACEMENT_LIMIT..=PLACEMENT_LIMIT).contains(&y)
            {
                continue;
            }
            let origin_x = x.floor();
            let subpixel = ((x - origin_x) * 4.0) as u32 % 4;
            let key = MaskKey {
                font_id: run.font.id,
                font_index: run.font.index,
                glyph_id: glyph.id,
                instance,
                size,
                embolden,
                skew,
                subpixel,
            };
            let Some(entry) = glyphs.entry(key, &run.font, &run.coords) else {
                continue;
            };
            let (mask_width, mask_height) = (entry.width(), entry.height());
            if mask_width == 0 || mask_height == 0 {
                continue;
            }

            // The mask is placed at the glyph's baseline origin, offset by its
            // bearings. Intersecting with `area` keeps every write inside the
            // run's ink bounds and the clip, which is what the damage covers.
            let x0 = origin_x as i64 + i64::from(entry.left());
            let y0 = y as i64 + i64::from(entry.top());
            let x1 = x0 + i64::from(mask_width);
            let y1 = y0 + i64::from(mask_height);
            let ix0 = x0.max(i64::from(area.x));
            let iy0 = y0.max(i64::from(area.y));
            let ix1 = x1.min(i64::from(area.x) + i64::from(area.width));
            let iy1 = y1.min(i64::from(area.y) + i64::from(area.height));
            if ix0 >= ix1 || iy0 >= iy1 {
                continue;
            }

            let mut mask_offset = ((iy0 - y0) as usize) * mask_width as usize + (ix0 - x0) as usize;
            let row_bytes = (ix1 - ix0) as usize * 4;
            for y in iy0..iy1 {
                let start = ((y as usize - *top as usize) * *width as usize
                    + (ix0 as usize - *left as usize))
                    * 4;
                let row = &mut pixels[start..start + row_bytes];
                let coverage = &entry.data()[mask_offset..mask_offset + (ix1 - ix0) as usize];
                for (target, &coverage) in row.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
                    if coverage == 0 {
                        continue;
                    }
                    let alpha = (u32::from(coverage) * u32::from(run.color.a) + 127) / 255;
                    blend(target, run.color, alpha);
                }
                mask_offset += mask_width as usize;
            }
        }
    }

    /// Draws the border inside the logical `rect` whose sides are `widths`
    /// thick: the coverage of the rounded rectangle less that of the shape
    /// [`Shape::inset`] leaves inside it, or the whole rectangle when nothing
    /// is left inside.
    fn ring(
        &mut self,
        area: PixelRect,
        rect: Rect,
        radius: Corners,
        widths: Edges<f32>,
        color: Color,
    ) {
        let Some(outer) = Shape::new(rect, radius, self.scale) else {
            return;
        };
        let scale = self.scale;
        let by = Edges {
            top: widths.top * scale,
            right: widths.right * scale,
            bottom: widths.bottom * scale,
            left: widths.left * scale,
        };
        match outer.inset(by) {
            Some(inner) => self.fill(
                area,
                color,
                |x, y| (outer.coverage(x, y) - inner.coverage(x, y)).max(0.0),
                // A run inside the inner shape can reach into a corner of the
                // outer one when the sides differ, where the outer coverage
                // falls below one, so the run keeps clear of both.
                |y| {
                    let (inner, outer) = (inner.steady(y), outer.steady(y));
                    let start = inner.start.max(outer.start);
                    start..inner.end.min(outer.end).max(start)
                },
            ),
            None => self.fill(
                area,
                color,
                |x, y| outer.coverage(x, y),
                |y| outer.steady(y),
            ),
        }
    }

    /// Blends `color` into every pixel of `area`, weighted by the coverage
    /// that `coverage` gives for the pixel's absolute position.
    ///
    /// `steady` returns, for a row, a run of pixels that all share one
    /// coverage, so the coverage is computed once for the whole run. Which
    /// pixels fall in the run depends only on their position, so a pixel gets
    /// the same value whichever part of the surface is drawn. Filling the run
    /// this way took a full repaint of a 1920 by 1080 scene of 200 rounded,
    /// bordered panels from 18.6 ms to 2.9 ms in a release build.
    fn fill(
        &mut self,
        area: PixelRect,
        color: Color,
        coverage: impl Fn(u32, u32) -> f32,
        steady: impl Fn(u32) -> Range<u32>,
    ) {
        if color.a == 0 {
            return;
        }
        let to_alpha = |coverage: f32| (coverage * f32::from(color.a) + 0.5) as u32;
        let right = area.x + area.width;
        for y in area.y..area.y + area.height {
            let run = steady(y);
            let start = run.start.clamp(area.x, right);
            let end = run.end.clamp(start, right);
            let row_start = self.index(area.x, y);
            let row = &mut self.pixels[row_start..row_start + area.width as usize * 4];
            let (before, rest) = row.split_at_mut((start - area.x) as usize * 4);
            let (middle, after) = rest.split_at_mut((end - start) as usize * 4);

            for (x, target) in (area.x..).zip(before.as_chunks_mut::<4>().0) {
                put(target, color, to_alpha(coverage(x, y)));
            }
            if start < end {
                let middle = middle.as_chunks_mut::<4>().0;
                match to_alpha(coverage(start, y)) {
                    0 => {}
                    255 => middle.fill([color.r, color.g, color.b, 255]),
                    alpha => middle
                        .iter_mut()
                        .for_each(|target| blend(target, color, alpha)),
                }
            }
            for (x, target) in (end..).zip(after.as_chunks_mut::<4>().0) {
                put(target, color, to_alpha(coverage(x, y)));
            }
        }
    }

    /// Returns the byte offset of the surface pixel `(x, y)`.
    fn index(&self, x: u32, y: u32) -> usize {
        ((y - self.top) as usize * self.width as usize + (x - self.left) as usize) * 4
    }
}

/// A rectangle in physical pixels whose corners are quarter ellipses.
///
/// Each corner's ellipse is centered `radius` in and `vertical` down or up
/// from its corner, and the two corners along each side together fit that
/// side, so no two corners overlap. A corner with either radius at zero is
/// square. A filled shape's corners are quarter circles at most half the
/// shorter side. The inner edge of a border can have elliptical corners, and
/// a corner whose ellipse reaches past the edge across from it, which then
/// cuts it off.
#[derive(Copy, Clone)]
struct Shape {
    /// The left edge.
    x0: f32,
    /// The top edge.
    y0: f32,
    /// The right edge.
    x1: f32,
    /// The bottom edge.
    y1: f32,
    /// The horizontal radius of each corner.
    radius: Corners,
    /// The vertical radius of each corner.
    vertical: Corners,
}

impl Shape {
    /// Converts a logical rectangle and its logical radii to physical pixels.
    ///
    /// Returns `None` when the rectangle is empty or holds a coordinate that is
    /// not finite.
    fn new(rect: Rect, radius: Corners, scale: f32) -> Option<Shape> {
        let scaled = Corners {
            top_left: radius.top_left * scale,
            top_right: radius.top_right * scale,
            bottom_right: radius.bottom_right * scale,
            bottom_left: radius.bottom_left * scale,
        };
        Shape::fitted(
            rect.x0 * scale,
            rect.y0 * scale,
            rect.x1 * scale,
            rect.y1 * scale,
            scaled,
        )
    }

    /// Returns the shape that lies each side's pixels of `by` inside this
    /// one, or `None` when nothing is left inside.
    ///
    /// Each inner corner is the quarter ellipse centered where this shape's
    /// corner circle is centered, reaching the two inner sides: its
    /// horizontal radius is this corner's radius less the width of the side
    /// beside it, and its vertical radius this radius less the width of the
    /// side above or below it. It is square when either comes to zero or
    /// less. Equal sides give quarter circles `by` inside this shape's, and
    /// the inner shape always lies inside this one.
    fn inset(&self, by: Edges<f32>) -> Option<Shape> {
        let (x0, y0) = (self.x0 + by.left, self.y0 + by.top);
        let (x1, y1) = (self.x1 - by.right, self.y1 - by.bottom);
        let finite = x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite();
        if !finite || x0 >= x1 || y0 >= y1 {
            return None;
        }
        let radius = self.radius;
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
        Some(Shape {
            x0,
            y0,
            x1,
            y1,
            radius: Corners {
                top_left: top_left.0,
                top_right: top_right.0,
                bottom_right: bottom_right.0,
                bottom_left: bottom_left.0,
            },
            vertical: Corners {
                top_left: top_left.1,
                top_right: top_right.1,
                bottom_right: bottom_right.1,
                bottom_left: bottom_left.1,
            },
        })
    }

    /// Builds a shape from physical edges with quarter circle corners,
    /// clamping each radius to between zero and half the shorter side.
    fn fitted(x0: f32, y0: f32, x1: f32, y1: f32, radius: Corners) -> Option<Shape> {
        let finite = x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite();
        if !finite || x0 >= x1 || y0 >= y1 {
            return None;
        }
        let limit = (x1 - x0).min(y1 - y0) / 2.0;
        let fit = |radius: f32| radius.max(0.0).min(limit);
        let radius = Corners {
            top_left: fit(radius.top_left),
            top_right: fit(radius.top_right),
            bottom_right: fit(radius.bottom_right),
            bottom_left: fit(radius.bottom_left),
        };
        Some(Shape {
            x0,
            y0,
            x1,
            y1,
            radius,
            vertical: radius,
        })
    }

    /// Returns the pixels of row `y` that lie wholly inside the shape's
    /// straight edges and outside every corner's box, which all share the
    /// row's vertical coverage.
    fn steady(&self, y: u32) -> Range<u32> {
        let top = y as f32;
        let down = self.y1.min(top + 1.0) - self.y0.max(top);
        if down <= 0.0 {
            return 0..0;
        }
        let cy = top + 0.5;
        let (radius, vertical) = (self.radius, self.vertical);
        let band = |upper: (f32, f32), lower: (f32, f32)| {
            if cy < self.y0 + upper.1 {
                upper.0
            } else if cy > self.y1 - lower.1 {
                lower.0
            } else {
                0.0
            }
        };
        let start = (self.x0
            + band(
                (radius.top_left, vertical.top_left),
                (radius.bottom_left, vertical.bottom_left),
            ))
        .ceil() as u32;
        let end = (self.x1
            - band(
                (radius.top_right, vertical.top_right),
                (radius.bottom_right, vertical.bottom_right),
            ))
        .floor() as u32;
        start..end.max(start)
    }

    /// Returns how much of pixel `(x, y)` the shape covers, from 0 to 1.
    ///
    /// Along the straight edges this is the exact area of the pixel inside the
    /// shape. Inside a corner's box it is also limited by the distance from
    /// the pixel center to the corner's curve, which approximates the area
    /// the curve cuts off.
    fn coverage(&self, x: u32, y: u32) -> f32 {
        let (left, top) = (x as f32, y as f32);
        let across = (self.x1.min(left + 1.0) - self.x0.max(left)).clamp(0.0, 1.0);
        let down = (self.y1.min(top + 1.0) - self.y0.max(top)).clamp(0.0, 1.0);
        let area = across * down;
        if area == 0.0 {
            return 0.0;
        }

        let (cx, cy) = (left + 0.5, top + 0.5);
        let (radius, vertical) = (self.radius, self.vertical);
        let (rx, ry, ox, oy) = if cx < self.x0 + radius.top_left && cy < self.y0 + vertical.top_left
        {
            let (rx, ry) = (radius.top_left, vertical.top_left);
            (rx, ry, self.x0 + rx, self.y0 + ry)
        } else if cx > self.x1 - radius.top_right && cy < self.y0 + vertical.top_right {
            let (rx, ry) = (radius.top_right, vertical.top_right);
            (rx, ry, self.x1 - rx, self.y0 + ry)
        } else if cx > self.x1 - radius.bottom_right && cy > self.y1 - vertical.bottom_right {
            let (rx, ry) = (radius.bottom_right, vertical.bottom_right);
            (rx, ry, self.x1 - rx, self.y1 - ry)
        } else if cx < self.x0 + radius.bottom_left && cy > self.y1 - vertical.bottom_left {
            let (rx, ry) = (radius.bottom_left, vertical.bottom_left);
            (rx, ry, self.x0 + rx, self.y1 - ry)
        } else {
            return area;
        };
        let inside = if rx == ry {
            rx - (cx - ox).hypot(cy - oy)
        } else {
            // The level of the ellipse's equation over the length of its
            // gradient, which is the distance to the curve near it.
            let (u, v) = ((cx - ox) / rx, (cy - oy) / ry);
            let gradient = 2.0 * ((u / rx).powi(2) + (v / ry).powi(2)).sqrt();
            if gradient > 0.0 {
                (1.0 - u * u - v * v) / gradient
            } else {
                rx.min(ry)
            }
        };
        area.min((inside + 0.5).clamp(0.0, 1.0))
    }
}

/// Draws `color` at `alpha` into a premultiplied pixel.
fn put(target: &mut [u8; 4], color: Color, alpha: u32) {
    match alpha {
        0 => {}
        255 => *target = [color.r, color.g, color.b, 255],
        _ => blend(target, color, alpha),
    }
}

/// Blends straight-alpha `color` at `alpha` over a premultiplied pixel.
fn blend(target: &mut [u8; 4], color: Color, alpha: u32) {
    let rest = 255 - alpha;
    let mix = |source: u8, target: u8| {
        ((u32::from(source) * alpha + u32::from(target) * rest + 127) / 255) as u8
    };
    target[0] = mix(color.r, target[0]);
    target[1] = mix(color.g, target[1]);
    target[2] = mix(color.b, target[2]);
    target[3] = mix(255, target[3]);
}

/// Samples a premultiplied RGBA8 image bilinearly at `u` and `v`, which are
/// in texel coordinates where texel `i`'s center sits at `i`.
///
/// Coordinates outside the image clamp to the edge texels, so a rect that
/// reaches past the image stretches its edge texels.
fn sample_bilinear(pixels: &[u8], width: u32, height: u32, u: f32, v: f32) -> [u8; 4] {
    // Clamping the coordinate, not the texel pair, keeps the fraction in
    // step with the clamped texel pair at the edges.
    let (fw, fh) = (width as f32, height as f32);
    let u = u.clamp(0.0, fw - 1.0);
    let v = v.clamp(0.0, fh - 1.0);
    // The index clamp is redundant for dimensions up to 2^24, where the
    // dimension and its last texel index are both exactly representable in
    // f32, and keeps a larger image inside its buffer past that.
    let left = (u.floor() as u32).min(width - 1);
    let top = (v.floor() as u32).min(height - 1);
    let right = (left + 1).min(width - 1);
    let bottom = (top + 1).min(height - 1);
    let fx = u - u.floor();
    let fy = v - v.floor();
    let texel = |x: u32, y: u32| {
        let start = (y as usize * width as usize + x as usize) * 4;
        [
            pixels[start],
            pixels[start + 1],
            pixels[start + 2],
            pixels[start + 3],
        ]
    };
    let a = texel(left, top);
    let b = texel(right, top);
    let c = texel(left, bottom);
    let d = texel(right, bottom);
    let mut sample = [0u8; 4];
    for i in 0..4 {
        let top_row = f32::from(a[i]) * (1.0 - fx) + f32::from(b[i]) * fx;
        let bottom_row = f32::from(c[i]) * (1.0 - fx) + f32::from(d[i]) * fx;
        sample[i] = (top_row * (1.0 - fy) + bottom_row * fy + 0.5) as u8;
    }
    sample
}

/// Blends a premultiplied RGBA `sample` over a premultiplied pixel at
/// `alpha`, the shape's coverage in 0..=255.
///
/// The coverage scales the premultiplied sample, and the pixel keeps the
/// part of itself that the effective source alpha, the sample's own alpha
/// times the coverage, leaves.
fn blend_image(target: &mut [u8; 4], sample: [u8; 4], alpha: u32) {
    let a = (u32::from(sample[3]) * alpha + 127) / 255;
    let rest = 255 - a;
    let mix =
        |source: u32, target: u8| ((source * 255 + u32::from(target) * rest + 127) / 255) as u8;
    for i in 0..3 {
        let src = (u32::from(sample[i]) * alpha + 127) / 255;
        target[i] = mix(src, target[i]);
    }
    target[3] = mix(a, target[3]);
}

/// Returns `channel` multiplied by `alpha` out of 255, rounded.
fn premultiply(channel: u8, alpha: u32) -> u8 {
    ((u32::from(channel) * alpha + 127) / 255) as u8
}
/// Builds a tiny-skia path from `path`, or `None` when it has no segment
/// tiny-skia can draw.
fn skia_path(path: &Path) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();
    for segment in path.segments() {
        match *segment {
            Segment::MoveTo(to) => builder.move_to(to.x, to.y),
            Segment::LineTo(to) => builder.line_to(to.x, to.y),
            Segment::QuadTo(control, to) => builder.quad_to(control.x, control.y, to.x, to.y),
            Segment::CubicTo(first, second, to) => {
                builder.cubic_to(first.x, first.y, second.x, second.y, to.x, to.y);
            }
            Segment::Close => builder.close(),
            // A segment added after this renderer draws nothing here.
            _ => {}
        }
    }
    builder.finish()
}

/// Builds an antialiased tiny-skia paint of `color`.
fn skia_paint(color: Color) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint.anti_alias = true;
    paint
}

/// Builds the tiny-skia stroke `stroke` describes.
fn skia_stroke(stroke: &Stroke, path: &Path) -> tiny_skia::Stroke {
    tiny_skia::Stroke {
        width: stroke.width,
        miter_limit: stroke.effective_miter_limit(),
        line_cap: match stroke.cap {
            LineCap::Round => tiny_skia::LineCap::Round,
            LineCap::Square => tiny_skia::LineCap::Square,
            _ => tiny_skia::LineCap::Butt,
        },
        line_join: match stroke.join {
            LineJoin::Round => tiny_skia::LineJoin::Round,
            LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
            _ => tiny_skia::LineJoin::Miter,
        },
        dash: stroke.dash.as_ref().and_then(|dash| {
            tiny_skia::StrokeDash::new(dash.pattern(path)?.collect(), dash.start())
        }),
    }
}
