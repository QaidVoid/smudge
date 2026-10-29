//! Unit tests for images.

use crate::image::{Image, ImageError};

#[test]
fn a_zero_dimension_is_rejected_whatever_the_buffer() {
    for (width, height) in [(0, 0), (0, 3), (3, 0)] {
        for pixels in [Vec::new(), vec![0; 12]] {
            let found = pixels.len();
            assert_eq!(
                Image::from_premultiplied_rgba(width, height, pixels.clone()),
                Err(ImageError::ZeroSize { width, height }),
                "{width}x{height} with {found} bytes"
            );
            assert_eq!(
                Image::from_straight_rgba(width, height, pixels),
                Err(ImageError::ZeroSize { width, height }),
                "{width}x{height} with {found} bytes, straight"
            );
        }
    }
}

#[test]
fn a_buffer_of_the_wrong_length_is_rejected() {
    for found in [0, 15, 17] {
        assert_eq!(
            Image::from_premultiplied_rgba(2, 2, vec![0; found]),
            Err(ImageError::PixelCount {
                expected: 16,
                found
            })
        );
        assert_eq!(
            Image::from_straight_rgba(2, 2, vec![0; found]),
            Err(ImageError::PixelCount {
                expected: 16,
                found
            })
        );
    }
}

#[test]
fn dimensions_whose_byte_count_overflows_are_rejected_without_a_panic() {
    for (width, height) in [(1 << 31, 1 << 31), (u32::MAX, u32::MAX), (1 << 31, 3)] {
        let result = Image::from_premultiplied_rgba(width, height, Vec::new());
        assert!(
            matches!(result, Err(ImageError::PixelCount { found: 0, .. })),
            "{width}x{height} gave {result:?}"
        );
    }
}

#[test]
fn color_channels_above_alpha_are_clamped_to_alpha() {
    let image = Image::from_premultiplied_rgba(2, 1, vec![255, 90, 200, 128, 10, 20, 30, 40])
        .expect("two pixels fill the buffer");
    assert_eq!(image.pixels(), &[128, 90, 128, 128, 10, 20, 30, 40]);
}

#[test]
fn straight_alpha_pixels_are_premultiplied_with_rounding() {
    let image = Image::from_straight_rgba(
        3,
        1,
        vec![255, 255, 255, 128, 200, 100, 1, 0, 10, 20, 30, 255],
    )
    .expect("three pixels fill the buffer");
    assert_eq!(
        image.pixels(),
        &[128, 128, 128, 128, 0, 0, 0, 0, 10, 20, 30, 255]
    );
}

#[test]
fn errors_describe_themselves() {
    let error: Box<dyn std::error::Error> = Box::new(ImageError::ZeroSize {
        width: 0,
        height: 4,
    });
    assert_eq!(error.to_string(), "an image cannot be 0x4 pixels");
    let error = ImageError::PixelCount {
        expected: 16,
        found: 7,
    };
    assert_eq!(
        error.to_string(),
        "the pixel buffer holds 7 bytes, but the size calls for 16"
    );
}
