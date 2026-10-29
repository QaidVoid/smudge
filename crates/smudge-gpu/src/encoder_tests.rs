//! Tests for the encoder's image cache.

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::image::Image;
use smudge_core::paint::{DisplayList, PaintOp};

use super::Encoder;

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
    let mut encoder = Encoder::new();
    encode(&mut encoder, &list_of(&[&kept, &dropped]));
    assert!(encoder.images.contains_key(&dropped.id()));

    let without = list_of(&[&kept]);
    for frame in 1..=2 {
        encode(&mut encoder, &without);
        assert!(
            encoder.images.contains_key(&dropped.id()),
            "unused for {frame} frames, the image is still cached"
        );
    }
    encode(&mut encoder, &without);
    assert!(
        !encoder.images.contains_key(&dropped.id()),
        "unused for three frames, the image leaves the cache"
    );
    assert!(
        encoder.images.contains_key(&kept.id()),
        "an image drawn every frame stays"
    );
    assert_eq!(encoder.images.len(), 1);
}

#[test]
fn an_evicted_image_is_cached_again_under_its_own_id() {
    let image = image();
    let mut encoder = Encoder::new();
    encode(&mut encoder, &list_of(&[&image]));
    for _ in 0..3 {
        encode(&mut encoder, &list_of(&[]));
    }
    assert!(encoder.images.is_empty());
    encode(&mut encoder, &list_of(&[&image]));
    let (brush, _) = &encoder.images[&image.id()];
    assert_eq!(brush.image.data.data(), image.pixels());
}

#[test]
fn the_brush_shares_the_image_pixels_instead_of_copying_them() {
    let image = image();
    let mut encoder = Encoder::new();
    encode(&mut encoder, &list_of(&[&image]));
    let (brush, _) = &encoder.images[&image.id()];
    assert_eq!(
        brush.image.data.data().as_ptr(),
        image.pixels().as_ptr(),
        "the blob points at the image's own buffer"
    );
}
