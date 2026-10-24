//! Tests for copying the renderer's surface into the window buffer.

use super::copy_pixels;

#[test]
fn copy_pixels_converts_each_pixel_to_rgb() {
    let source = [10, 20, 30, 255, 255, 128, 1, 255, 0, 0, 0, 255];
    let mut destination = [u32::MAX; 3];

    copy_pixels(&source, &mut destination);

    assert_eq!(destination, [0x000a_141e, 0x00ff_8001, 0x0000_0000]);
}

#[test]
fn copy_pixels_leaves_pixels_past_the_source_untouched() {
    let source = [1, 2, 3, 255];
    let mut destination = [0x00aa_bbcc; 2];

    copy_pixels(&source, &mut destination);

    assert_eq!(destination, [0x0001_0203, 0x00aa_bbcc]);
}
