//! An image that stretches over its rectangle.
//!
//! The pixels travel as [`Image`], which the application builds from RGBA
//! data it decoded off the UI thread. The widget paints no image while its
//! source is empty, so an image that is still decoding holds a `None` until
//! the pixels arrive. The style's background and border paint as they do on
//! a panel, the image fills the content box inside the padding, and the
//! style's [`corner_radius`](Element::corner_radius) rounds both.

use smudge_core::element::{Element, Value};
use smudge_core::geometry::{Corners, Size};
use smudge_core::image::Image;
use smudge_core::paint::PaintOp;
use smudge_core::tree::Dirty;
use smudge_core::widget::{MeasureInput, PaintCx, Panel, Widget};

/// Creates an image that stretches its source over the rectangle its style
/// gives it.
///
/// Without a size from its style it measures as the image's pixel size
/// divided by the window's scale factor, so one image pixel is one logical
/// pixel. With one axis fixed the other keeps the image's aspect ratio, and
/// with both fixed the image stretches. The image fills the content box,
/// inside any padding and border, and the style's `corner_radius` rounds it.
/// An empty source paints no image, only the style's background and border.
///
/// The source can be bound to a signal:
///
/// ```
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_widgets::image;
///
/// let source = RwSignal::new(None);
/// let picture = image(move || source.get());
/// ```
pub fn image(source: impl Into<Value<Option<Image>>>) -> Element<ImageView> {
    Element::new(ImageView { source: None }).bind(SourceKey, source.into(), set_source)
}

/// The key of an image's source binding.
struct SourceKey;

/// The stretch-one-image widget.
pub struct ImageView {
    /// The pixels to draw, or `None` while there is nothing to show.
    source: Option<Image>,
}

/// Replaces the image, and relays the node out when the replacement changes
/// what the widget measures.
///
/// The widget measures as the image's pixel size, so a source that appears,
/// disappears or changes pixel size must relay the node out: a paint-only
/// update would leave the node's rectangle stale. A swap between images of
/// the same size changes no layout answer, so it repaints alone.
fn set_source(view: &mut ImageView, source: Option<Image>) -> Dirty {
    let resized = match (&view.source, &source) {
        (None, None) => false,
        (Some(old), Some(new)) => old.width() != new.width() || old.height() != new.height(),
        _ => true,
    };
    if view.source == source {
        return Dirty::Clean;
    }
    view.source = source;
    if resized { Dirty::Layout } else { Dirty::Paint }
}

impl Widget for ImageView {
    /// Measures as the image's pixel size divided by the scale factor, so
    /// one image pixel is one logical pixel.
    ///
    /// When the style fixes one axis, the other follows from the image's
    /// aspect ratio. When it fixes both, the image stretches to them.
    ///
    /// The answer depends on the source alone, so it is the same on every
    /// call with the same input, which the engine's caching relies on.
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        let image = self.source.as_ref()?;
        let (width, height) = (image.width() as f32, image.height() as f32);
        Some(match (input.known_width, input.known_height) {
            (Some(known_width), Some(known_height)) => Size {
                width: known_width,
                height: known_height,
            },
            (Some(known_width), None) => Size {
                width: known_width,
                height: known_width * height / width,
            },
            (None, Some(known_height)) => Size {
                width: known_height * width / height,
                height: known_height,
            },
            (None, None) => Size {
                width: width / input.scale,
                height: height / input.scale,
            },
        })
    }

    /// Paints the style's background and border the way a panel does, then
    /// the image stretched over the content box.
    ///
    /// The image's corners are the style's corner radius shrunk by the
    /// border and padding next to them, so they stay concentric with the
    /// outer corners.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        Panel.paint(cx, out);
        let Some(image) = &self.source else {
            return;
        };
        let radius = cx.style.paint.corner_radius.resolve(cx.theme);
        let left = cx.content.x0 - cx.rect.x0;
        let top = cx.content.y0 - cx.rect.y0;
        let right = cx.rect.x1 - cx.content.x1;
        let bottom = cx.rect.y1 - cx.content.y1;
        let inner = |a: f32, b: f32| (radius - a.max(b)).max(0.0);
        out.push(PaintOp::Image {
            rect: cx.content,
            radius: Corners {
                top_left: inner(left, top),
                top_right: inner(right, top),
                bottom_right: inner(right, bottom),
                bottom_left: inner(left, bottom),
            },
            image: image.clone(),
        });
    }
}
