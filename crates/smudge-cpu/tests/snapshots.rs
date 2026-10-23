//! Pixel snapshots of a scene the CPU renderer draws.
//!
//! The scenes cover the parts of the renderer that a single pixel probe cannot
//! describe: rounded corners, a border that follows them, and the hard edge a
//! clip leaves. One scene is rendered at a whole scale and one at a fractional
//! scale, where every logical coordinate lands between pixels.

use insta::assert_binary_snapshot;
use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, PixelRect, Rect};
use smudge_core::paint::{DisplayList, PaintOp};
use smudge_cpu::CpuRenderer;

/// The scene size in logical pixels.
const SCENE: Rect = Rect {
    x0: 0.0,
    y0: 0.0,
    x1: 48.0,
    y1: 32.0,
};

/// A rounded background, a bordered panel on top of it, and a rounded fill that
/// overflows the clip around it.
fn scene() -> DisplayList {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: SCENE,
        radius: Corners::all(8.0),
        color: Color::rgb(245, 245, 245),
    });
    let panel = Rect::new(4.0, 4.0, 44.0, 28.0);
    list.push_op(PaintOp::FillRect {
        rect: panel,
        radius: Corners::all(5.0),
        color: Color::rgb(60, 120, 210),
    });
    list.push_op(PaintOp::StrokeRect {
        rect: panel,
        radius: Corners::all(5.0),
        width: 2.0,
        color: Color::rgb(20, 40, 80),
    });
    list.push_clip(Rect::new(10.0, 10.0, 34.0, 22.0));
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(12.0, 12.0, 50.0, 28.0),
        radius: Corners::all(4.0),
        color: Color::rgb(250, 190, 60),
    });
    list.pop_clip();
    list.finish();
    list
}

/// Renders the scene into a fresh renderer and returns its PNG encoding.
fn render(width: u32, height: u32, scale: f32) -> Vec<u8> {
    let mut renderer = CpuRenderer::new(width, height);
    let list = scene();
    let damage = [PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    }];
    renderer.render(&FrameOutput {
        list: &list,
        damage: &damage,
        full: true,
        clear_color: Color::WHITE,
        scale,
    });
    renderer.encode_png()
}

#[test]
fn rounded_scene_at_scale_one() {
    assert_binary_snapshot!(".png", render(48, 32, 1.0));
}

#[test]
fn rounded_scene_at_scale_one_and_a_half() {
    assert_binary_snapshot!(".png", render(72, 48, 1.5));
}
