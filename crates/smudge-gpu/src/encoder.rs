//! Encoding a display list into a vello scene, shared by the offscreen
//! renderer and the surface presenter.

use std::collections::HashMap;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::{DisplayItem, FontBlob, PaintOp};
use vello::kurbo::{Affine, Rect as KurboRect, RoundedRect, RoundedRectRadii, Stroke};
use vello::peniko::{Blob, Color as PenikoColor, Fill, FontData};
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
}

impl Encoder {
    /// Creates an empty encoder.
    pub(crate) fn new() -> Encoder {
        Encoder {
            scene: Scene::new(),
            fonts: HashMap::new(),
        }
    }

    /// Replaces the scene's contents with `frame`'s display list.
    pub(crate) fn encode(&mut self, frame: &FrameOutput<'_>) {
        self.scene.reset();
        let transform = Affine::scale(f64::from(frame.scale));
        // The display list is balanced by construction, but a stray `PopClip`
        // in a release build would pop a vello layer that was never pushed,
        // so the depth is tracked here and leftover clips are closed before
        // rendering.
        let mut depth = 0usize;
        for item in frame.list.items() {
            match item {
                DisplayItem::Op(op) => encode_op(&mut self.scene, &mut self.fonts, transform, op),
                DisplayItem::PushClip(rect) => {
                    depth += 1;
                    self.scene
                        .push_clip_layer(Fill::NonZero, transform, &kurbo_rect(*rect));
                }
                DisplayItem::PopClip => {
                    if depth > 0 {
                        depth -= 1;
                        self.scene.pop_layer();
                    }
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
