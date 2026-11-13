//! Tests for the encoder's image cache.

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::image::Image;
use smudge_core::paint::{DisplayList, PaintOp};

use super::{Encoder, downscale, fitted_size};

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

/// Encodes one frame of `list`.
fn encode(encoder: &mut Encoder, list: &DisplayList) {
    encoder.encode(&FrameOutput {
        list,
        damage: &[],
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
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
    assert!(encoder.images.brushes.contains_key(&dropped.id()));

    let without = list_of(&[&kept]);
    for frame in 1..=2 {
        encode(&mut encoder, &without);
        assert!(
            encoder.images.brushes.contains_key(&dropped.id()),
            "unused for {frame} frames, the image is still cached"
        );
    }
    encode(&mut encoder, &without);
    assert!(
        !encoder.images.brushes.contains_key(&dropped.id()),
        "unused for three frames, the image leaves the cache"
    );
    assert!(
        encoder.images.brushes.contains_key(&kept.id()),
        "an image drawn every frame stays"
    );
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
    let (brush, _) = &encoder.images.brushes[&image.id()];
    assert_eq!(brush.image.data.data(), image.pixels());
}

#[test]
fn the_brush_shares_the_image_pixels_instead_of_copying_them() {
    let image = image();
    let mut encoder = Encoder::new(8192);
    encode(&mut encoder, &list_of(&[&image]));
    let (brush, _) = &encoder.images.brushes[&image.id()];
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
    let (brush, _) = &encoder.images.brushes[&wide.id()];
    assert_eq!((brush.image.width, brush.image.height), (16, 4));
    assert_eq!(brush.image.data.data(), &[255; 16 * 4 * 4][..]);
}
