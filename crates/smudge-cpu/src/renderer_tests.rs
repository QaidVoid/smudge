//! Behavior tests for the renderer.
//!
//! Every test drives the public API, so what it checks is what a caller sees:
//! the pixels of the surface after a frame.

use std::rc::Rc;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, PixelRect, Point, Rect};
use smudge_core::paint::{DisplayList, GlyphRun, PaintOp};
use smudge_text::{TextLayout, TextStyle, TextSystem};

use crate::CpuRenderer;

/// Opaque white in premultiplied RGBA.
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Opaque black in premultiplied RGBA.
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// Opaque red in premultiplied RGBA.
const RED: [u8; 4] = [255, 0, 0, 255];

/// The color every test surface starts from.
const CLEAR: Color = Color::WHITE;

/// Renders `list` for the physical rectangles in `damage`.
fn render(renderer: &mut CpuRenderer, list: &DisplayList, damage: &[PixelRect], scale: f32) {
    renderer.render(&FrameOutput {
        list,
        damage,
        full: false,
        clear_color: CLEAR,
        scale,
    });
}

/// Returns the premultiplied RGBA value of one pixel.
fn pixel(renderer: &CpuRenderer, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * renderer.width() + x) * 4) as usize;
    let pixels = renderer.pixels();
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

/// Returns the first pixel where two surfaces differ.
fn first_difference(left: &CpuRenderer, right: &CpuRenderer) -> Option<(u32, u32)> {
    let width = left.width();
    for y in 0..left.height() {
        for x in 0..width {
            if pixel(left, x, y) != pixel(right, x, y) {
                return Some((x, y));
            }
        }
    }
    None
}

/// A display list holding one operation.
fn list_of(op: PaintOp) -> DisplayList {
    let mut list = DisplayList::new();
    list.push_op(op);
    list.finish();
    list
}

/// A filled rectangle with square corners.
fn fill(rect: Rect, color: Color) -> PaintOp {
    PaintOp::FillRect {
        rect,
        radius: Corners::ZERO,
        color,
    }
}

/// The damage rectangle covering a whole square surface.
fn surface(size: u32) -> PixelRect {
    PixelRect {
        x: 0,
        y: 0,
        width: size,
        height: size,
    }
}

/// Splits a square surface into tiles of `tile` pixels, cut short at the far
/// edges.
fn tiles(size: u32, tile: u32) -> Vec<PixelRect> {
    let mut tiles = Vec::new();
    for y in (0..size).step_by(tile as usize) {
        for x in (0..size).step_by(tile as usize) {
            tiles.push(PixelRect {
                x,
                y,
                width: tile.min(size - x),
                height: tile.min(size - y),
            });
        }
    }
    tiles
}

/// A rounded panel, a rounded inner rectangle with a border, and a clipped
/// fill that overflows its clip, so one frame exercises every drawing path.
fn scene(inner: Color) -> DisplayList {
    let mut list = DisplayList::new();
    list.push_op(fill(
        Rect::new(0.0, 0.0, 40.0, 40.0),
        Color::rgb(200, 200, 200),
    ));
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(4.0, 4.0, 20.0, 20.0),
        radius: Corners::all(3.0),
        color: inner,
    });
    list.push_op(PaintOp::StrokeRect {
        rect: Rect::new(4.0, 4.0, 20.0, 20.0),
        radius: Corners::all(3.0),
        width: 1.5,
        color: Color::rgba(20, 40, 80, 180),
    });
    list.push_clip(Rect::new(6.0, 6.0, 30.0, 30.0));
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(2.0, 2.0, 34.0, 34.0),
        radius: Corners::all(5.0),
        color: Color::rgb(220, 40, 90),
    });
    list.pop_clip();
    list.finish();
    list
}

#[test]
fn damage_leaves_pixels_outside_it_unchanged() {
    let mut renderer = CpuRenderer::new(16, 16);
    let black = list_of(fill(Rect::new(0.0, 0.0, 16.0, 16.0), Color::BLACK));
    render(&mut renderer, &black, &[surface(16)], 1.0);
    assert_eq!(pixel(&renderer, 0, 0), BLACK);

    let white = list_of(fill(Rect::new(0.0, 0.0, 16.0, 16.0), Color::WHITE));
    let damage = [PixelRect {
        x: 4,
        y: 4,
        width: 4,
        height: 4,
    }];
    render(&mut renderer, &white, &damage, 1.0);

    for y in 0..16 {
        for x in 0..16 {
            let inside = (4..8).contains(&x) && (4..8).contains(&y);
            let expected = if inside { WHITE } else { BLACK };
            assert_eq!(pixel(&renderer, x, y), expected, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn fill_rect_at_scale_two_colors_its_physical_rect() {
    let mut renderer = CpuRenderer::new(80, 80);
    let red = list_of(fill(
        Rect::new(10.0, 10.0, 30.0, 30.0),
        Color::rgb(255, 0, 0),
    ));
    render(&mut renderer, &red, &[surface(80)], 2.0);

    assert_eq!(pixel(&renderer, 20, 20), RED);
    assert_eq!(pixel(&renderer, 59, 59), RED);
    assert_eq!(pixel(&renderer, 19, 20), WHITE, "left of the rectangle");
    assert_eq!(pixel(&renderer, 20, 19), WHITE, "above the rectangle");
    assert_eq!(pixel(&renderer, 60, 20), WHITE, "right of the rectangle");
    assert_eq!(pixel(&renderer, 20, 60), WHITE, "below the rectangle");
}

#[test]
fn partly_covered_pixels_blend_by_the_covered_area() {
    let cases = [
        (Color::BLACK, 127..=128, 191..=192),
        (Color::rgba(0, 0, 0, 128), 191..=192, 223..=224),
    ];
    for (color, half, quarter) in cases {
        let mut renderer = CpuRenderer::new(16, 16);
        let list = list_of(fill(Rect::new(0.5, 0.5, 10.0, 10.0), color));
        render(&mut renderer, &list, &[surface(16)], 1.0);

        for (x, y, expected, place) in [
            (5, 0, &half, "half covered on the top edge"),
            (0, 5, &half, "half covered on the left edge"),
            (0, 0, &quarter, "quarter covered in the corner"),
        ] {
            let [r, g, b, a] = pixel(&renderer, x, y);
            assert!(
                expected.contains(&r) && r == g && g == b && a == 255,
                "{place} with alpha {}: {:?}",
                color.a,
                [r, g, b, a]
            );
        }
    }
}

#[test]
fn op_inside_a_clip_draws_nothing_outside_it() {
    let mut renderer = CpuRenderer::new(40, 40);
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(10.0, 10.0, 20.0, 20.0));
    list.push_op(fill(Rect::new(0.0, 0.0, 40.0, 40.0), Color::BLACK));
    list.pop_clip();
    list.finish();

    render(&mut renderer, &list, &[surface(40)], 1.0);

    assert_eq!(pixel(&renderer, 10, 10), BLACK);
    assert_eq!(pixel(&renderer, 19, 19), BLACK);
    assert_eq!(pixel(&renderer, 9, 15), WHITE, "left of the clip");
    assert_eq!(pixel(&renderer, 15, 9), WHITE, "above the clip");
    assert_eq!(pixel(&renderer, 20, 15), WHITE, "right of the clip");
    assert_eq!(pixel(&renderer, 15, 20), WHITE, "below the clip");
}

#[test]
fn op_inside_a_narrow_clip_keeps_its_own_edges() {
    let mut renderer = CpuRenderer::new(40, 40);
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(10.0, 10.0, 30.0, 30.0));
    list.push_op(fill(Rect::new(12.0, 12.0, 18.0, 18.0), Color::BLACK));
    list.pop_clip();
    list.finish();

    render(&mut renderer, &list, &[surface(40)], 1.0);

    assert_eq!(pixel(&renderer, 12, 12), BLACK);
    assert_eq!(pixel(&renderer, 17, 17), BLACK);
    assert_eq!(pixel(&renderer, 11, 12), WHITE);
    assert_eq!(pixel(&renderer, 18, 17), WHITE);
}

#[test]
fn nested_clips_intersect() {
    let mut renderer = CpuRenderer::new(40, 40);
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(8.0, 8.0, 30.0, 30.0));
    list.push_clip(Rect::new(20.0, 4.0, 36.0, 24.0));
    list.push_op(fill(Rect::new(0.0, 0.0, 40.0, 40.0), Color::BLACK));
    list.pop_clip();
    list.pop_clip();
    list.finish();

    render(&mut renderer, &list, &[surface(40)], 1.0);

    assert_eq!(pixel(&renderer, 20, 8), BLACK);
    assert_eq!(pixel(&renderer, 29, 23), BLACK);
    assert_eq!(pixel(&renderer, 19, 15), WHITE, "left of the inner clip");
    assert_eq!(pixel(&renderer, 25, 7), WHITE, "above the inner clip");
    assert_eq!(pixel(&renderer, 30, 15), WHITE, "right of the outer clip");
    assert_eq!(pixel(&renderer, 25, 24), WHITE, "below the inner clip");
}

#[test]
fn damage_render_equals_a_full_repaint() {
    let mut damaged = CpuRenderer::new(40, 40);
    render(&mut damaged, &scene(Color::BLACK), &[surface(40)], 1.0);
    let changed = PixelRect {
        x: 4,
        y: 4,
        width: 16,
        height: 16,
    };
    render(
        &mut damaged,
        &scene(Color::rgb(10, 20, 30)),
        &[changed],
        1.0,
    );

    let mut full = CpuRenderer::new(40, 40);
    render(
        &mut full,
        &scene(Color::rgb(10, 20, 30)),
        &[surface(40)],
        1.0,
    );

    assert_eq!(first_difference(&full, &damaged), None);
}

#[test]
fn damage_render_equals_a_full_repaint_at_a_fractional_scale() {
    let mut damaged = CpuRenderer::new(40, 40);
    render(&mut damaged, &scene(Color::BLACK), &[surface(40)], 1.5);
    let changed = PixelRect {
        x: 6,
        y: 6,
        width: 24,
        height: 24,
    };
    render(
        &mut damaged,
        &scene(Color::rgb(10, 20, 30)),
        &[changed],
        1.5,
    );

    let mut full = CpuRenderer::new(40, 40);
    render(
        &mut full,
        &scene(Color::rgb(10, 20, 30)),
        &[surface(40)],
        1.5,
    );

    assert_eq!(first_difference(&full, &damaged), None);
}

#[test]
fn damage_edges_through_rounded_corners_match_a_full_repaint() {
    for scale in [1.0, 1.5] {
        let size = (40.0 * scale) as u32;
        let list = scene(Color::BLACK);
        let mut full = CpuRenderer::new(size, size);
        render(&mut full, &list, &[surface(size)], scale);

        for tile in 1..=19 {
            let mut tiled = CpuRenderer::new(size, size);
            render(&mut tiled, &list, &tiles(size, tile), scale);
            assert_eq!(
                first_difference(&full, &tiled),
                None,
                "tiles of {tile} pixels at scale {scale}"
            );
        }
    }
}

#[test]
fn corner_radius_is_in_logical_pixels() {
    let mut renderer = CpuRenderer::new(80, 80);
    let list = list_of(PaintOp::FillRect {
        rect: Rect::new(0.0, 0.0, 40.0, 40.0),
        radius: Corners::all(10.0),
        color: Color::BLACK,
    });
    render(&mut renderer, &list, &[surface(80)], 2.0);

    assert_eq!(pixel(&renderer, 3, 3), WHITE, "outside a 20 pixel corner");
    assert_eq!(pixel(&renderer, 8, 8), BLACK, "inside a 20 pixel corner");
}

#[test]
fn border_outer_edge_follows_a_background_with_the_same_radius() {
    let rect = Rect::new(0.0, 0.0, 40.0, 40.0);
    let radius = Corners::all(8.0);
    let background = list_of(PaintOp::FillRect {
        rect,
        radius,
        color: Color::BLACK,
    });
    let border = list_of(PaintOp::StrokeRect {
        rect,
        radius,
        width: 4.0,
        color: Color::BLACK,
    });
    let mut filled = CpuRenderer::new(40, 40);
    render(&mut filled, &background, &[surface(40)], 1.0);
    let mut stroked = CpuRenderer::new(40, 40);
    render(&mut stroked, &border, &[surface(40)], 1.0);

    for y in 0..12 {
        for x in 0..12 {
            if x < 4 || y < 4 {
                assert_eq!(
                    pixel(&stroked, x, y),
                    pixel(&filled, x, y),
                    "pixel ({x}, {y}) between the outer edge and the border's inner edge"
                );
            }
        }
    }
}

#[test]
fn op_under_an_offset_draws_at_the_offset() {
    let mut renderer = CpuRenderer::new(40, 40);
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 10.0, y: 6.5 });
    list.push_op(fill(Rect::new(2.0, 2.0, 20.0, 20.0), Color::BLACK));
    list.pop_offset();
    list.push_op(fill(Rect::new(30.0, 30.0, 40.0, 40.0), Color::BLACK));
    list.finish();

    render(&mut renderer, &list, &[surface(40)], 1.0);

    assert_eq!(pixel(&renderer, 12, 8), BLACK, "inside the moved fill");
    assert_eq!(pixel(&renderer, 30, 8), BLACK, "at the moved fill's edge");
    assert_eq!(pixel(&renderer, 11, 8), WHITE, "left of the moved fill");
    assert_eq!(pixel(&renderer, 31, 37), BLACK, "outside any offset");
    assert_eq!(pixel(&renderer, 30, 36), WHITE, "above the unmoved fill");
}

#[test]
fn a_clip_under_an_offset_clips_where_the_offset_puts_it() {
    let mut renderer = CpuRenderer::new(40, 40);
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 8.0, y: 8.0 });
    list.push_clip(Rect::new(4.0, 4.0, 20.0, 20.0));
    list.push_op(fill(Rect::new(0.0, 0.0, 40.0, 40.0), Color::BLACK));
    list.pop_clip();
    list.pop_offset();
    list.finish();

    render(&mut renderer, &list, &[surface(40)], 1.0);

    assert_eq!(pixel(&renderer, 12, 12), BLACK);
    assert_eq!(pixel(&renderer, 27, 27), BLACK);
    assert_eq!(pixel(&renderer, 11, 12), WHITE, "left of the moved clip");
    assert_eq!(pixel(&renderer, 12, 11), WHITE, "above the moved clip");
    assert_eq!(pixel(&renderer, 28, 15), WHITE, "right of the moved clip");
    assert_eq!(pixel(&renderer, 15, 28), WHITE, "below the moved clip");
}

#[test]
fn nested_offsets_compose() {
    let mut renderer = CpuRenderer::new(48, 48);
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 10.0, y: 4.0 });
    list.push_offset(Point { x: 2.0, y: 30.0 });
    list.push_op(fill(Rect::new(0.0, 0.0, 8.0, 8.0), Color::BLACK));
    list.pop_offset();
    list.push_op(fill(Rect::new(0.0, 0.0, 8.0, 8.0), Color::BLACK));
    list.pop_offset();
    list.finish();

    render(&mut renderer, &list, &[surface(48)], 1.0);

    assert_eq!(pixel(&renderer, 12, 34), BLACK, "inside both offsets");
    assert_eq!(pixel(&renderer, 17, 39), BLACK);
    assert_eq!(pixel(&renderer, 11, 34), WHITE, "left of the inner fill");
    assert_eq!(
        pixel(&renderer, 10, 4),
        BLACK,
        "inside the outer offset only"
    );
    assert_eq!(pixel(&renderer, 17, 11), BLACK);
    assert_eq!(pixel(&renderer, 9, 4), WHITE, "left of the outer fill");
}

#[test]
fn an_offset_at_a_fractional_scale_lands_on_physical_pixels() {
    let mut renderer = CpuRenderer::new(48, 48);
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 5.0, y: 7.0 });
    list.push_op(fill(Rect::new(2.0, 2.0, 14.0, 14.0), Color::BLACK));
    list.pop_offset();
    list.finish();

    render(&mut renderer, &list, &[surface(48)], 2.0);

    assert_eq!(pixel(&renderer, 14, 18), BLACK);
    assert_eq!(pixel(&renderer, 37, 41), BLACK);
    assert_eq!(pixel(&renderer, 13, 18), WHITE, "left of the moved fill");
    assert_eq!(pixel(&renderer, 14, 17), WHITE, "above the moved fill");
    assert_eq!(pixel(&renderer, 38, 18), WHITE, "right of the moved fill");
    assert_eq!(pixel(&renderer, 14, 42), WHITE, "below the moved fill");
}

#[test]
fn text_under_an_offset_draws_the_same_bytes_as_translated_text() {
    TextSystem::bundled_only().install();
    let run = TextSystem::with(|_| {
        TextLayout::new("Smudge", &TextStyle::default(), None)
            .glyph_runs_at(Point { x: 4.0, y: 4.0 }, Color::BLACK)
    });
    let run = run.into_iter().next().expect("the label shapes one run");
    let moved = {
        let mut glyphs = run.glyphs.clone();
        for glyph in &mut glyphs {
            glyph.x += 10.0;
            glyph.y += 12.0;
        }
        GlyphRun {
            font: run.font.clone(),
            size: run.size,
            color: run.color,
            glyphs,
            ink_bounds: run.ink_bounds.translated(Point { x: 10.0, y: 12.0 }),
        }
    };

    let mut list = DisplayList::new();
    list.push_offset(Point { x: 10.0, y: 12.0 });
    list.push_op(PaintOp::Glyphs(Rc::new(run)));
    list.pop_offset();
    list.finish();
    let translated = list_of(PaintOp::Glyphs(Rc::new(moved)));

    let mut shifted = CpuRenderer::new(40, 40);
    render(&mut shifted, &list, &[surface(40)], 1.0);
    let mut direct = CpuRenderer::new(40, 40);
    render(&mut direct, &translated, &[surface(40)], 1.0);

    assert_eq!(first_difference(&shifted, &direct), None);
}

#[test]
fn damage_render_equals_a_full_repaint_under_an_offset() {
    let shifted = |inner: Color| {
        let mut list = DisplayList::new();
        list.push_offset(Point { x: 6.0, y: 2.0 });
        list.push_clip(Rect::new(2.0, 2.0, 30.0, 30.0));
        list.push_op(fill(
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Color::rgb(200, 200, 200),
        ));
        list.push_op(PaintOp::FillRect {
            rect: Rect::new(4.0, 4.0, 20.0, 20.0),
            radius: Corners::all(3.0),
            color: inner,
        });
        list.pop_clip();
        list.pop_offset();
        list.finish();
        list
    };

    let mut damaged = CpuRenderer::new(40, 40);
    render(&mut damaged, &shifted(Color::BLACK), &[surface(40)], 1.0);
    render(
        &mut damaged,
        &shifted(Color::rgb(10, 20, 30)),
        &[PixelRect {
            x: 10,
            y: 6,
            width: 16,
            height: 16,
        }],
        1.0,
    );

    let mut full = CpuRenderer::new(40, 40);
    render(
        &mut full,
        &shifted(Color::rgb(10, 20, 30)),
        &[surface(40)],
        1.0,
    );

    assert_eq!(first_difference(&full, &damaged), None);
}

#[test]
fn empty_surface_holds_no_pixels() {
    let mut renderer = CpuRenderer::new(0, 0);
    render(&mut renderer, &DisplayList::new(), &[], 1.0);

    assert_eq!(renderer.width(), 0);
    assert_eq!(renderer.height(), 0);
    assert!(renderer.pixels().is_empty());
    assert!(renderer.encode_png().is_empty());
}

#[test]
fn resize_replaces_the_surface_size() {
    let mut renderer = CpuRenderer::new(8, 8);
    renderer.resize(16, 4);

    assert_eq!(renderer.width(), 16);
    assert_eq!(renderer.height(), 4);
    assert_eq!(renderer.pixels().len(), 16 * 4 * 4);
}

#[test]
#[should_panic(expected = "different surface size")]
fn render_panics_when_damage_reaches_outside_the_surface() {
    let mut renderer = CpuRenderer::new(8, 8);
    let damage = [PixelRect {
        x: 4,
        y: 4,
        width: 8,
        height: 8,
    }];
    render(&mut renderer, &DisplayList::new(), &damage, 1.0);
}
