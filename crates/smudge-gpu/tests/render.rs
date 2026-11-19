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
use smudge_core::style::Edges;
use smudge_cpu::CpuRenderer;
use smudge_gpu::GpuRenderer;
use smudge_text::{FontWeight, Span, SpanStyle, TextSystem, label, rich_text};

/// The size of the test surface, in physical pixels.
const SIZE: u32 = 64;

/// Returns a renderer like [`GpuRenderer::headless`], on a device requested
/// without the pipeline cache feature, so the tests write no cache file.
fn headless_renderer(width: u32, height: u32) -> Option<GpuRenderer> {
    use futures_executor::block_on;
    use smudge_gpu::{device_limits, instance, wgpu};
    let adapter =
        block_on(instance().request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: device_limits(&adapter),
        ..Default::default()
    }))
    .ok()?;
    Some(GpuRenderer::new(device, queue, width, height))
}

/// Renders `ops` onto a white surface and returns the pixels.
fn rendered(ops: &[PaintOp], scale: f32) -> Vec<u8> {
    let mut renderer = headless_renderer(SIZE, SIZE)
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

/// The side of the surface [`rendered_on`] draws on when a test needs more
/// room than [`SIZE`] gives, in physical pixels.
const WIDE: u32 = 128;

/// Renders `ops` onto a white surface `size` pixels square and returns the
/// pixels.
fn rendered_on(ops: &[PaintOp], scale: f32, size: u32) -> Vec<u8> {
    let mut renderer = headless_renderer(size, size)
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
fn a_side_of_width_zero_draws_nothing_where_it_cuts_through_pixels() {
    let Some(_) = headless_renderer(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let pixels = rendered(
        &[PaintOp::StrokeSides {
            rect: Rect::new(8.0, 4.5, 56.0, 40.5),
            radius: Corners::ZERO,
            widths: Edges {
                top: 0.0,
                right: 0.0,
                bottom: 3.0,
                left: 0.0,
            },
            color: Color::rgb(20, 20, 20),
        }],
        1.0,
    );
    for x in [8, 20, 40, 55] {
        assert_eq!(
            pixel_at(&pixels, x, 4),
            [255, 255, 255, 255],
            "the empty top side at column {x}"
        );
        assert_eq!(
            pixel_at(&pixels, x, 38),
            [20, 20, 20, 255],
            "the bottom band at column {x}"
        );
    }
    for y in [10, 20, 30] {
        assert_eq!(
            pixel_at(&pixels, 8, y),
            [255, 255, 255, 255],
            "the empty left side"
        );
    }
}

#[test]
fn a_side_wider_than_the_rounded_rectangle_allows_draws_nothing_outside_it() {
    let Some(_) = headless_renderer(SIZE, SIZE) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let sides = |top: f32, right: f32, bottom: f32| Edges {
        top,
        right,
        bottom,
        left: 0.0,
    };
    // Each border has a side wider than its rectangle less a corner radius,
    // so an inner corner's curve stops at the inner side across from it, and
    // some of those curves run along the outer one.
    let cases = [
        (
            Rect::new(2.0, 2.0, 42.0, 22.0),
            10.0,
            sides(0.0, 0.0, 15.0),
            1.0,
        ),
        (
            Rect::new(2.0, 2.0, 41.5, 39.75),
            20.75,
            sides(6.75, 37.5, 0.0),
            2.0,
        ),
        (
            Rect::new(3.25, 2.75, 51.5, 8.75),
            11.75,
            sides(0.0, 5.75, 5.5),
            2.0,
        ),
    ];
    let color = Color::rgb(20, 20, 20);
    for (rect, radius, widths, scale) in cases {
        let radius = Corners::all(radius);
        let border = rendered_on(
            &[PaintOp::StrokeSides {
                rect,
                radius,
                widths,
                color,
            }],
            scale,
            WIDE,
        );
        let shape = rendered_on(
            &[PaintOp::FillRect {
                rect,
                radius,
                color,
            }],
            scale,
            WIDE,
        );
        for y in 0..WIDE {
            for x in 0..WIDE {
                let at = ((y * WIDE + x) * 4) as usize;
                let (drawn, allowed) = (border[at], shape[at]);
                // Pixels the outline's edge passes through may shade a few
                // levels apart, since the two outlines are flattened apart.
                // A pixel the outline leaves blank must stay blank.
                if allowed >= 250 {
                    assert!(
                        u16::from(drawn) + 10 >= u16::from(allowed),
                        "{rect:?} at scale {scale}: ink at ({x}, {y}) outside the \
                         rounded outline, {drawn} where the shape itself draws {allowed}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_pushed_clip_limits_what_after_it_draws() {
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
    let mut renderer = headless_renderer(SIZE, SIZE)
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
    let mut renderer = headless_renderer(SIZE, SIZE)
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
    let mut renderer = headless_renderer(SIZE, SIZE)
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
    let mut renderer = headless_renderer(SIZE, SIZE)
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
    let Some(mut gpu) = headless_renderer(320, 80) else {
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
    let Some(_) = headless_renderer(SIZE, SIZE) else {
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
        embolden: 0.0,
        skew: 0.0,
        coords: Vec::new(),
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

/// How many pixels of a white surface text darkened.
fn dark_pixels(pixels: &[u8]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[..3].iter().any(|&channel| channel <= 128))
        .count()
}

/// Renders `text` in the bundled mono, which has a regular face only, at
/// `weight` and slanted when `italic`, on both renderers, and returns the
/// CPU's pixels and the GPU's.
fn mono_on_both(
    gpu: &mut GpuRenderer,
    text: &'static str,
    weight: Option<FontWeight>,
    italic: bool,
) -> (Vec<u8>, Vec<u8>) {
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
        move || {
            rich_text(move || {
                Rc::from([Span::new(
                    text,
                    SpanStyle {
                        monospace: true,
                        weight,
                        italic,
                        ..SpanStyle::default()
                    },
                )])
            })
        },
    );
    let frame = app.frame();
    let mut cpu = CpuRenderer::new(320, 80);
    cpu.render(&frame);
    gpu.render(&frame);
    (cpu.pixels().to_vec(), gpu.read_pixels())
}

#[test]
fn synthetic_bold_and_slant_match_the_cpu_renderer() {
    let Some(mut gpu) = headless_renderer(320, 80) else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let (cpu_regular, gpu_regular) = mono_on_both(&mut gpu, "Smudge", None, false);
    let (cpu_bold, gpu_bold) = mono_on_both(&mut gpu, "Smudge", Some(FontWeight::BOLD), true);

    let expected = ink_bounds(&cpu_bold, 320).expect("the CPU renderer drew the text");
    let actual = ink_bounds(&gpu_bold, 320).expect("the GPU renderer drew the text");
    let close = |a: usize, b: usize| a.abs_diff(b) <= 2;
    assert!(
        close(actual.0, expected.0)
            && close(actual.1, expected.1)
            && close(actual.2, expected.2)
            && close(actual.3, expected.3),
        "the slanted glyphs landed at {actual:?} on the GPU and at {expected:?} on the CPU"
    );
    for (renderer, regular, bold) in [
        ("CPU", &cpu_regular, &cpu_bold),
        ("GPU", &gpu_regular, &gpu_bold),
    ] {
        let (regular, bold) = (dark_pixels(regular), dark_pixels(bold));
        assert!(
            bold * 100 > regular * 115,
            "the {renderer} renderer drew the bold {bold} dark pixels against {regular} regular"
        );
    }
    let (cpu, gpu) = (dark_pixels(&cpu_bold), dark_pixels(&gpu_bold));
    assert!(
        cpu.abs_diff(gpu) * 10 <= cpu,
        "the renderers disagree on the bold's weight: {cpu} dark pixels on the CPU, {gpu} on the GPU"
    );
}
