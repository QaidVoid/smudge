//! Behavior tests for drawing straight into a window buffer.
//!
//! Every test compares the buffer with what a full repaint by
//! [`CpuRenderer`] shows once its alpha is dropped, which is what a window
//! presented before this renderer existed.

use std::rc::Rc;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, PixelRect, Point, Rect};
use smudge_core::image::Image;
use smudge_core::paint::{DisplayList, PaintOp};
use smudge_text::{TextLayout, TextStyle, TextSystem};

use crate::{CpuRenderer, XrgbRenderer};

/// The surface side in physical pixels.
const SIZE: u32 = 48;

/// A value no pixel of a scene takes, so a pixel left alone stands out.
const UNTOUCHED: u32 = 0xff12_3456;

/// Builds a frame of `list` that repaints `damage` at `scale`.
fn frame<'a>(list: &'a DisplayList, damage: &'a [PixelRect], scale: f32) -> FrameOutput<'a> {
    FrameOutput {
        list,
        damage,
        full: false,
        clear_color: Color::WHITE,
        scale,
    }
}

/// The damage rectangle covering the whole surface.
fn whole() -> PixelRect {
    PixelRect {
        x: 0,
        y: 0,
        width: SIZE,
        height: SIZE,
    }
}

/// A scene that exercises every drawing path: translucent and rounded fills,
/// a border, text under an offset, an image and a clip. `step` moves and
/// recolors parts of it, so consecutive steps differ in a few places.
fn scene(step: u32) -> DisplayList {
    TextSystem::bundled_only().install();
    let shift = (step % 5) as f32 * 1.5;
    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(0.0, 0.0, 32.0, 32.0),
        radius: Corners::ZERO,
        color: Color::rgb(200, 210, 220),
    });
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(2.0 + shift, 3.0, 18.0 + shift, 15.0),
        radius: Corners::all(4.0),
        color: Color::rgba(40, (step * 37 % 255) as u8, 90, 170),
    });
    list.push_op(PaintOp::StrokeRect {
        rect: Rect::new(1.0, 1.0, 30.0, 30.0),
        radius: Corners::all(3.0),
        width: 1.25,
        color: Color::rgba(20, 40, 80, 200),
    });
    let run = TextLayout::new("Ag", &TextStyle::default(), None)
        .glyph_runs_at(Point { x: 0.0, y: 0.0 }, Color::BLACK)
        .into_iter()
        .next()
        .expect("the label shapes one run");
    list.push_offset(Point {
        x: 4.0 + shift,
        y: 14.0,
    });
    list.push_op(PaintOp::Glyphs(Rc::new(run)));
    list.pop_offset();
    list.push_clip(Rect::new(18.0, 16.0, 31.0, 31.0));
    list.push_op(PaintOp::Image {
        rect: Rect::new(14.0 - shift, 12.0, 34.0, 32.0),
        radius: Corners::all(5.0),
        image: checker(),
    });
    list.pop_clip();
    list.finish();
    list
}

/// A small translucent checkerboard, so sampling and blending both show.
fn checker() -> Image {
    let mut pixels = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            let pixel = if (x + y) % 2 == 0 {
                [120, 20, 60, 160]
            } else {
                [10, 200, 30, 255]
            };
            pixels.extend_from_slice(&pixel);
        }
    }
    Image::from_premultiplied_rgba(4, 4, pixels).expect("the checker fills its buffer")
}

/// Returns what a window showed for a full repaint of `list`: the
/// premultiplied pixels of [`CpuRenderer`] as `0x00RRGGBB` words.
fn full_repaint(list: &DisplayList, scale: f32) -> Vec<u32> {
    let mut renderer = CpuRenderer::new(SIZE, SIZE);
    renderer.render(&frame(list, &[whole()], scale));
    renderer
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[red, green, blue, _]| {
            u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue)
        })
        .collect()
}

/// Returns the first pixel where two buffers differ, with both values.
fn first_difference(left: &[u32], right: &[u32]) -> Option<(u32, u32, u32, u32)> {
    let index = left.iter().zip(right).position(|(a, b)| a != b)?;
    Some((
        index as u32 % SIZE,
        index as u32 / SIZE,
        left[index],
        right[index],
    ))
}

#[test]
fn a_full_frame_fills_the_buffer_like_a_full_repaint() {
    for scale in [1.0, 1.5] {
        let list = scene(0);
        let mut renderer = XrgbRenderer::new(SIZE, SIZE);
        let mut buffer = vec![UNTOUCHED; (SIZE * SIZE) as usize];

        renderer.render_into(&frame(&list, &[whole()], scale), &mut buffer);

        assert_eq!(
            first_difference(&buffer, &full_repaint(&list, scale)),
            None,
            "scale {scale}"
        );
    }
}

#[test]
fn damaged_frames_keep_the_buffer_equal_to_a_full_repaint() {
    // The rectangles cover everything a step moves and overlap, so some
    // pixels are painted twice within one frame.
    let damage = [
        PixelRect {
            x: 0,
            y: 0,
            width: 26,
            height: 18,
        },
        PixelRect {
            x: 2,
            y: 10,
            width: 24,
            height: 24,
        },
        PixelRect {
            x: 12,
            y: 10,
            width: 24,
            height: 24,
        },
    ];
    let mut renderer = XrgbRenderer::new(SIZE, SIZE);
    let mut buffer = vec![UNTOUCHED; (SIZE * SIZE) as usize];
    renderer.render_into(&frame(&scene(0), &[whole()], 1.0), &mut buffer);

    for step in 1..8 {
        let list = scene(step);
        renderer.render_into(&frame(&list, &damage, 1.0), &mut buffer);

        assert_eq!(
            first_difference(&buffer, &full_repaint(&list, 1.0)),
            None,
            "step {step}"
        );
    }
}

#[test]
fn pixels_outside_the_damage_keep_the_buffer_contents() {
    let list = scene(3);
    let mut renderer = XrgbRenderer::new(SIZE, SIZE);
    let mut buffer = vec![UNTOUCHED; (SIZE * SIZE) as usize];
    let rect = PixelRect {
        x: 5,
        y: 7,
        width: 11,
        height: 9,
    };

    renderer.render_into(&frame(&list, &[rect], 1.0), &mut buffer);

    for (index, &pixel) in buffer.iter().enumerate() {
        let (x, y) = (index as u32 % SIZE, index as u32 / SIZE);
        let inside = (rect.x..rect.x + rect.width).contains(&x)
            && (rect.y..rect.y + rect.height).contains(&y);
        assert_eq!(pixel == UNTOUCHED, !inside, "pixel ({x}, {y})");
    }
}

#[test]
#[should_panic(expected = "one entry per pixel")]
fn render_panics_when_the_buffer_has_another_size() {
    let list = DisplayList::new();
    let mut renderer = XrgbRenderer::new(SIZE, SIZE);
    let mut buffer = vec![0; 10];

    renderer.render_into(&frame(&list, &[whole()], 1.0), &mut buffer);
}
