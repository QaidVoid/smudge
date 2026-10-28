//! Renders frames on a real GPU adapter and checks the pixels they produce.
//!
//! Every test skips with a printed reason when the machine has no GPU
//! adapter, so a missing adapter can never fail a run.

use std::rc::Rc;
use std::sync::Arc;

use smudge_core::app::{App, AppConfig, FrameOutput, RepaintMode};
use smudge_core::geometry::{Color, Corners, Point, Rect, Size};
use smudge_core::paint::{DisplayList, FontBlob, GlyphRun, PaintOp, PositionedGlyph};
use smudge_core::renderer::Renderer;
use smudge_cpu::CpuRenderer;
use smudge_gpu::GpuRenderer;
use smudge_text::{TextSystem, label};

/// The size of the test surface, in physical pixels.
const SIZE: u32 = 64;

/// Renders `ops` onto a white surface and returns the pixels.
fn rendered(ops: &[PaintOp], scale: f32) -> Vec<u8> {
    let mut renderer = GpuRenderer::headless(SIZE, SIZE)
        .unwrap_or_else(|| panic!("the GPU adapter disappeared between tests"));
    let mut list = DisplayList::new();
    list.extend_ops(ops);
    list.finish();
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[],
        full: true,
        clear_color: Color::WHITE,
        scale,
    });
    renderer.read_pixels()
}

/// Returns the pixel at `x`, `y` in physical pixels.
fn pixel_at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let start = (y as usize * SIZE as usize + x as usize) * 4;
    pixels[start..start + 4].try_into().expect("a whole pixel")
}

#[test]
fn a_filled_rect_covers_its_center_and_leaves_the_clear_color_outside() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let pixels = rendered(
        &[PaintOp::FillRect {
            rect: Rect::new(16.0, 16.0, 48.0, 48.0),
            radius: Corners::ZERO,
            color: Color::rgb(200, 30, 40),
        }],
        1.0,
    );
    assert_eq!(pixel_at(&pixels, 0, 0), [255, 255, 255, 255]);
    assert_eq!(pixel_at(&pixels, 32, 32), [200, 30, 40, 255]);
    // The anti-aliased edge is between the rect and the background, so a
    // pixel just outside the rect keeps the clear color.
    assert_eq!(pixel_at(&pixels, 14, 32), [255, 255, 255, 255]);
}

#[test]
fn the_scale_factor_maps_logical_rects_to_physical_pixels() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    // At scale 2 the logical rect covers the same physical pixels as the
    // scale 1 rect of the other test.
    let pixels = rendered(
        &[PaintOp::FillRect {
            rect: Rect::new(8.0, 8.0, 24.0, 24.0),
            radius: Corners::ZERO,
            color: Color::rgb(200, 30, 40),
        }],
        2.0,
    );
    assert_eq!(pixel_at(&pixels, 0, 0), [255, 255, 255, 255]);
    assert_eq!(pixel_at(&pixels, 32, 32), [200, 30, 40, 255]);
    assert_eq!(pixel_at(&pixels, 14, 32), [255, 255, 255, 255]);
}

#[test]
fn a_stroke_draws_inside_its_rectangle() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let pixels = rendered(
        &[PaintOp::StrokeRect {
            rect: Rect::new(8.0, 8.0, 56.0, 56.0),
            radius: Corners::ZERO,
            width: 8.0,
            color: Color::rgb(200, 30, 40),
        }],
        1.0,
    );
    // The stroke of width 8 covers the 8 pixel band inside the edge, so the
    // pixel at 12 in from the edge is border and the middle stays background.
    assert_eq!(pixel_at(&pixels, 12, 12), [200, 30, 40, 255]);
    assert_eq!(pixel_at(&pixels, 32, 32), [255, 255, 255, 255]);
    // Outside the rectangle nothing is drawn.
    assert_eq!(pixel_at(&pixels, 4, 4), [255, 255, 255, 255]);
}

#[test]
fn a_pushed_clip_limits_what_after_it_draws() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(0.0, 0.0, 16.0, SIZE as f32));
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(0.0, 0.0, 64.0, 64.0),
        radius: Corners::ZERO,
        color: Color::rgb(200, 30, 40),
    });
    list.pop_clip();
    list.finish();
    let mut renderer = GpuRenderer::headless(SIZE, SIZE)
        .unwrap_or_else(|| panic!("the GPU adapter disappeared between tests"));
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[],
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    let pixels = renderer.read_pixels();
    assert_eq!(pixel_at(&pixels, 8, 32), [200, 30, 40, 255]);
    assert_eq!(pixel_at(&pixels, 32, 32), [255, 255, 255, 255]);
}

#[test]
fn an_offset_moves_what_after_it_draws() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 16.0, y: 8.0 });
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(8.0, 8.0, 32.0, 32.0),
        radius: Corners::ZERO,
        color: Color::rgb(200, 30, 40),
    });
    list.pop_offset();
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(48.0, 48.0, 64.0, 64.0),
        radius: Corners::ZERO,
        color: Color::rgb(30, 40, 200),
    });
    list.finish();
    let mut renderer = GpuRenderer::headless(SIZE, SIZE)
        .unwrap_or_else(|| panic!("the GPU adapter disappeared between tests"));
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[],
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    let pixels = renderer.read_pixels();
    assert_eq!(pixel_at(&pixels, 24, 24), [200, 30, 40, 255]);
    assert_eq!(pixel_at(&pixels, 55, 55), [30, 40, 200, 255]);
    assert_eq!(
        pixel_at(&pixels, 8, 24),
        [255, 255, 255, 255],
        "left of the moved fill"
    );
    assert_eq!(
        pixel_at(&pixels, 47, 55),
        [255, 255, 255, 255],
        "left of the unmoved fill"
    );
}

#[test]
fn an_offset_is_in_logical_pixels_at_any_scale() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 10.0, y: 6.0 });
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(2.0, 2.0, 8.0, 8.0),
        radius: Corners::ZERO,
        color: Color::rgb(200, 30, 40),
    });
    list.pop_offset();
    list.finish();
    let mut renderer = GpuRenderer::headless(SIZE, SIZE)
        .unwrap_or_else(|| panic!("the GPU adapter disappeared between tests"));
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[],
        full: true,
        clear_color: Color::WHITE,
        scale: 2.0,
    });
    let pixels = renderer.read_pixels();
    // The fill lands at (12, 8) to (18, 14) logical, 24 to 36 by 16 to 28
    // physical. An offset applied after the scale would put it at 14 to 26
    // by 10 to 22.
    assert_eq!(pixel_at(&pixels, 34, 26), [200, 30, 40, 255]);
    assert_eq!(
        pixel_at(&pixels, 16, 12),
        [255, 255, 255, 255],
        "where an offset in physical pixels would have put the fill"
    );
}

#[test]
fn a_clip_under_an_offset_clips_where_the_offset_puts_it() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let mut list = DisplayList::new();
    list.push_offset(Point { x: 8.0, y: 8.0 });
    list.push_clip(Rect::new(0.0, 0.0, 16.0, 56.0));
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(0.0, 0.0, 64.0, 64.0),
        radius: Corners::ZERO,
        color: Color::rgb(200, 30, 40),
    });
    list.pop_clip();
    list.pop_offset();
    list.finish();
    let mut renderer = GpuRenderer::headless(SIZE, SIZE)
        .unwrap_or_else(|| panic!("the GPU adapter disappeared between tests"));
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[],
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    let pixels = renderer.read_pixels();
    assert_eq!(pixel_at(&pixels, 12, 32), [200, 30, 40, 255]);
    assert_eq!(
        pixel_at(&pixels, 24, 32),
        [255, 255, 255, 255],
        "right of the moved clip"
    );
    assert_eq!(
        pixel_at(&pixels, 8, 8),
        [200, 30, 40, 255],
        "the moved clip's first pixel"
    );
    assert_eq!(
        pixel_at(&pixels, 8, 7),
        [255, 255, 255, 255],
        "above the moved clip"
    );
    assert_eq!(
        pixel_at(&pixels, 7, 32),
        [255, 255, 255, 255],
        "left of the moved clip"
    );
}

/// The smallest box, in physical pixels, holding every pixel of a white
/// `width` wide surface that text darkened, or `None` when nothing did.
fn ink_bounds(pixels: &[u8], width: usize) -> Option<(usize, usize, usize, usize)> {
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        if pixel[..3].iter().all(|&channel| channel > 128) {
            continue;
        }
        let (x, y) = (index % width, index / width);
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    bounds
}

#[test]
fn text_lands_where_the_cpu_renderer_draws_it() {
    let Some(mut gpu) = GpuRenderer::headless(320, 80) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    TextSystem::bundled_only().install();
    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 160.0,
                height: 40.0,
            },
            scale: 2.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Full,
        },
        || label("Smudge"),
    );
    let frame = app.frame();
    let mut cpu = CpuRenderer::new(320, 80);
    cpu.render(&frame);
    gpu.render(&frame);

    let expected = ink_bounds(cpu.pixels(), 320).expect("the CPU renderer drew the text");
    let actual = ink_bounds(&gpu.read_pixels(), 320).expect("the GPU renderer drew the text");
    let close = |a: usize, b: usize| a.abs_diff(b) <= 2;
    assert!(
        close(actual.0, expected.0)
            && close(actual.1, expected.1)
            && close(actual.2, expected.2)
            && close(actual.3, expected.3),
        "the glyphs landed at {actual:?} on the GPU and at {expected:?} on the CPU"
    );
}

#[test]
fn a_run_in_a_font_that_does_not_parse_draws_nothing() {
    let Some(_) = GpuRenderer::headless(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let run = GlyphRun {
        font: FontBlob {
            id: u64::MAX,
            data: Arc::new(vec![0u8; 64]),
            index: 0,
        },
        size: 16.0,
        color: Color::BLACK,
        glyphs: vec![PositionedGlyph {
            id: 1,
            x: 8.0,
            y: 32.0,
        }],
        ink_bounds: Rect::new(8.0, 16.0, 24.0, 36.0),
    };
    let pixels = rendered(&[PaintOp::Glyphs(Rc::new(run))], 1.0);
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255; 4]),
        "a face that does not parse leaves the surface clear"
    );
}
