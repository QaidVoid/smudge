//! Copying the renderer's surface into the window buffer.

/// Copies premultiplied RGBA8 pixels into a softbuffer pixel buffer.
///
/// Each destination entry becomes `0x00RRGGBB`. The alpha channel is dropped,
/// so the surface must be opaque for the copy to be lossless. Pixels past the
/// end of the shorter of the two slices are left alone.
pub(crate) fn copy_pixels(source: &[u8], destination: &mut [u32]) {
    let (source, _) = source.as_chunks::<4>();
    for (pixel, rgba) in destination.iter_mut().zip(source) {
        *pixel = u32::from(rgba[0]) << 16 | u32::from(rgba[1]) << 8 | u32::from(rgba[2]);
    }
}

#[cfg(test)]
#[path = "present_tests.rs"]
mod tests;
