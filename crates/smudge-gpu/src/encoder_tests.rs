//! Tests for the encoder's image cache.

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::image::Image;
use smudge_core::paint::{DisplayList, PaintOp};

use super::{Encoder, downscale, fitted_side, fitted_size};

/// A display list that draws each of `images` once.
fn list_of(images: &[&Image]) -> DisplayList {
    let mut list = DisplayList::new();
    for image in images {
        list.push_op(PaintOp::Image {
            rect: Rect::new(0.0, 0.0, 8.0, 8.0),
            radius: Corners::ZERO,
            image: (*image).clone(),
        });
    }
    list.finish();
    list
}

/// The side of the square surface the tests encode for.
const SURFACE: u32 = 64;

/// Encodes one frame of `list` for the test surface.
fn encode(encoder: &mut Encoder, list: &DisplayList) {
    encoder.encode(
        &FrameOutput {
            list,
            damage: &[],
            full: true,
            clear_color: Color::WHITE,
            scale: 1.0,
        },
        SURFACE,
        SURFACE,
    );
}

/// A display list that draws `image` over `rect`, inside a clip of `clip`
/// when one is given.
fn list_drawing(image: &Image, rect: Rect, clip: Option<Rect>) -> DisplayList {
    let mut list = DisplayList::new();
    if let Some(clip) = clip {
        list.push_clip(clip);
    }
    list.push_op(PaintOp::Image {
        rect,
        radius: Corners::ZERO,
        image: image.clone(),
    });
    if clip.is_some() {
        list.pop_clip();
    }
    list.finish();
    list
}

/// Returns whether the encoder holds a brush of `image` at any size.
fn cached(encoder: &Encoder, image: &Image) -> bool {
    encoder
        .images
        .brushes
        .keys()
        .any(|(id, _)| *id == image.id())
}

/// A fresh two-pixel image with an id of its own.
fn image() -> Image {
    Image::from_premultiplied_rgba(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 255])
        .expect("two pixels fill the buffer")
}

#[test]
fn an_image_stays_cached_for_two_unused_frames_and_leaves_on_the_third() {
    let kept = image();
    let dropped = image();
    let mut encoder = Encoder::new(8192);
    encode(&mut encoder, &list_of(&[&kept, &dropped]));
    assert!(cached(&encoder, &dropped));

    let without = list_of(&[&kept]);
    for frame in 1..=2 {
        encode(&mut encoder, &without);
        assert!(
            cached(&encoder, &dropped),
            "unused for {frame} frames, the image is still cached"
        );
    }
    encode(&mut encoder, &without);
    assert!(
        !cached(&encoder, &dropped),
        "unused for three frames, the image leaves the cache"
    );
    assert!(cached(&encoder, &kept), "an image drawn every frame stays");
    assert_eq!(encoder.images.brushes.len(), 1);
}

#[test]
fn an_evicted_image_is_cached_again_under_its_own_id() {
    let image = image();
    let mut encoder = Encoder::new(8192);
    encode(&mut encoder, &list_of(&[&image]));
    for _ in 0..3 {
        encode(&mut encoder, &list_of(&[]));
    }
    assert!(encoder.images.brushes.is_empty());
    encode(&mut encoder, &list_of(&[&image]));
    let (brush, _) = &encoder.images.brushes[&(image.id(), 2)];
    assert_eq!(brush.image.data.data(), image.pixels());
}

#[test]
fn the_brush_shares_the_image_pixels_instead_of_copying_them() {
    let image = image();
    let mut encoder = Encoder::new(8192);
    encode(&mut encoder, &list_of(&[&image]));
    let (brush, _) = &encoder.images.brushes[&(image.id(), 2)];
    assert_eq!(
        brush.image.data.data().as_ptr(),
        image.pixels().as_ptr(),
        "the blob points at the image's own buffer"
    );
}

#[test]
fn an_image_within_the_limit_keeps_its_size() {
    assert_eq!(fitted_size(100, 40, 8192), (100, 40));
    assert_eq!(fitted_size(8192, 8192, 8192), (8192, 8192));
}

#[test]
fn an_oversized_image_fits_its_longer_side_and_keeps_the_aspect_ratio() {
    assert_eq!(fitted_size(9000, 400, 8192), (8192, 364));
    assert_eq!(fitted_size(400, 9000, 8192), (364, 8192));
    assert_eq!(fitted_size(10_000, 10_000, 2048), (2048, 2048));
    assert_eq!(fitted_size(3000, 1500, 2048), (2048, 1024));
}

#[test]
fn a_sliver_never_shrinks_below_one_pixel() {
    assert_eq!(fitted_size(100_000, 1, 8192), (8192, 1));
    assert_eq!(fitted_size(u32::MAX, u32::MAX, 8192), (8192, 8192));
}

#[test]
fn downscaling_averages_the_block_each_pixel_covers() {
    // Four by two pixels to two by one: each target pixel averages a two by
    // two block, rounded to the nearest value.
    let pixels = [
        [10, 0, 0, 255],
        [20, 0, 0, 255],
        [0, 0, 0, 0],
        [0, 0, 100, 100],
        [30, 0, 0, 255],
        [41, 0, 0, 255],
        [0, 0, 0, 0],
        [0, 0, 101, 101],
    ]
    .concat();
    assert_eq!(
        downscale(&pixels, 4, 2, 2, 1),
        [25, 0, 0, 255, 0, 0, 50, 50]
    );
}

#[test]
fn an_oversized_image_is_cached_at_its_fitted_size() {
    let wide = Image::from_premultiplied_rgba(40, 10, vec![255; 40 * 10 * 4])
        .expect("the pixels fill the buffer");
    let mut encoder = Encoder::new(16);
    encode(&mut encoder, &list_of(&[&wide]));
    let (brush, _) = &encoder.images.brushes[&(wide.id(), 16)];
    assert_eq!((brush.image.width, brush.image.height), (16, 4));
    assert_eq!(brush.image.data.data(), &[255; 16 * 4 * 4][..]);
}

#[test]
fn an_image_outside_its_clip_is_left_out_of_the_atlas() {
    let image = image();
    let mut encoder = Encoder::new(8192);
    let clip = Some(Rect::new(0.0, 0.0, 16.0, 16.0));
    encode(
        &mut encoder,
        &list_drawing(&image, Rect::new(20.0, 20.0, 28.0, 28.0), clip),
    );
    assert!(
        !cached(&encoder, &image),
        "a clipped-out image is not handed over"
    );

    encode(
        &mut encoder,
        &list_drawing(&image, Rect::new(12.0, 12.0, 20.0, 20.0), clip),
    );
    assert!(
        cached(&encoder, &image),
        "a partly visible image is handed over"
    );
}

#[test]
fn an_image_past_the_surface_is_left_out_of_the_atlas() {
    let image = image();
    let mut encoder = Encoder::new(8192);
    encode(
        &mut encoder,
        &list_drawing(&image, Rect::new(70.0, 0.0, 90.0, 8.0), None),
    );
    assert!(!cached(&encoder, &image));
}

#[test]
fn an_image_with_a_non_finite_rect_is_left_out() {
    let image = image();
    let mut encoder = Encoder::new(8192);
    for rect in [
        Rect::new(0.0, 0.0, f32::INFINITY, 8.0),
        Rect::new(f32::NEG_INFINITY, 0.0, 8.0, 8.0),
        Rect::new(0.0, 0.0, 8.0, f32::NAN),
    ] {
        encode(&mut encoder, &list_drawing(&image, rect, None));
        assert!(!cached(&encoder, &image), "{rect:?} draws nothing");
    }
}

#[test]
fn a_large_image_is_cached_at_the_power_of_two_above_its_drawn_size() {
    let photo = Image::from_premultiplied_rgba(400, 300, vec![200; 400 * 300 * 4])
        .expect("the pixels fill the buffer");
    let mut encoder = Encoder::new(8192);
    encode(
        &mut encoder,
        &list_drawing(&photo, Rect::new(4.0, 4.0, 40.0, 31.0), None),
    );
    let (brush, _) = &encoder.images.brushes[&(photo.id(), 64)];
    assert_eq!((brush.image.width, brush.image.height), (64, 48));
}

#[test]
fn the_fitted_side_keeps_a_texel_per_drawn_pixel_on_both_axes() {
    assert_eq!(fitted_side(400, 300, 36.0, 27.0, 8192), 64);
    assert_eq!(fitted_side(400, 300, 64.0, 10.0, 8192), 64);
    assert_eq!(fitted_side(400, 300, 65.0, 10.0, 8192), 128);
    // Stretched across: the short side of the image needs 200 texels, so
    // the long side keeps its full 4000.
    assert_eq!(fitted_side(4000, 100, 100.0, 200.0, 8192), 4000);
}

#[test]
fn the_fitted_side_never_exceeds_the_image_or_the_limit() {
    assert_eq!(fitted_side(40, 30, 500.0, 375.0, 8192), 40);
    assert_eq!(fitted_side(9000, 400, 9000.0, 400.0, 8192), 8192);
    assert_eq!(fitted_side(9000, 400, 1.0e30, 1.0, 2048), 2048);
    assert_eq!(fitted_side(9000, 400, f64::NAN, f64::NAN, 2048), 2048);
}
