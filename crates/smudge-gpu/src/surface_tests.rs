//! Tests for the surface presenter's format choice.

use vello::wgpu::TextureFormat;

use super::pick_format;

#[test]
fn the_first_supported_format_in_capability_order_wins() {
    let formats = [
        TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Bgra8Unorm,
        TextureFormat::Rgba8Unorm,
    ];
    assert_eq!(pick_format(&formats), Some(TextureFormat::Bgra8Unorm));
}

#[test]
fn a_surface_with_only_unsupported_formats_has_no_pick() {
    let formats = [
        TextureFormat::Rgba16Float,
        TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Rgba8UnormSrgb,
    ];
    assert_eq!(pick_format(&formats), None);
}
