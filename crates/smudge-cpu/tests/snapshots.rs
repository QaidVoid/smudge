//! Pixel snapshots of scenes the CPU renderer draws.
//!
//! The scenes cover the parts of the renderer that a single pixel probe cannot
//! describe: rounded corners, a border that follows them, the hard edge a clip
//! leaves, and the anti-aliased ink of glyphs. One scene is rendered at a whole
//! scale and one at a fractional scale, where every logical coordinate lands
//! between pixels.

use std::rc::Rc;

use insta::assert_binary_snapshot;
use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, PixelRect, Point, Rect};
use smudge_core::paint::{DisplayItem, DisplayList, PaintOp};
use smudge_cpu::CpuRenderer;
use smudge_text::{TextLayout, TextStyle, TextSystem};

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

/// A white background with a line of black text on it.
///
/// The glyphs come from the bundled font, so the ink is the same on every
/// machine.
fn text_scene() -> DisplayList {
    TextSystem::bundled_only().install();
    let style = TextStyle::default();
    let layout = TextLayout::new("Wgx", &style, None);

    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: SCENE,
        radius: Corners::ZERO,
        color: Color::WHITE,
    });
    for run in layout.glyph_runs_at(Point { x: 4.0, y: 4.0 }, Color::BLACK) {
        list.push_op(PaintOp::Glyphs(Rc::new(run)));
    }
    list.finish();
    list
}

/// Renders the text scene into a fresh renderer, checks the ink it left, and
/// returns the PNG encoding of the surface.
///
/// The checks describe what the text must look like rather than which exact
/// bytes the snapshot holds: dark ink where the glyphs are, pixels of every
/// shade between ink and background where their edges are anti-aliased, and no
/// ink outside the bounds the glyph run claims.
fn render_text(width: u32, height: u32, scale: f32) -> Vec<u8> {
    let list = text_scene();
    let ink = list
        .items()
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Op(PaintOp::Glyphs(run)) => Some(run.ink_bounds),
            _ => None,
        })
        .fold(Rect::default(), Rect::union);
    assert!(!ink.is_empty(), "the text claims ink");
    let (logical_width, logical_height) = (width as f32 / scale, height as f32 / scale);
    assert!(
        ink.x0 >= 0.0 && ink.y0 >= 0.0 && ink.x1 <= logical_width && ink.y1 <= logical_height,
        "the text must fit the surface, so no glyph is cut off: ink {ink:?} on a \
         {logical_width} by {logical_height} surface"
    );

    let mut renderer = CpuRenderer::new(width, height);
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

    let (pixels, _) = renderer.pixels().as_chunks::<4>();
    let claimed = PixelRect::from_logical(ink, scale, width, height)
        .expect("the ink sits inside the surface");
    let (mut dark, mut middle) = (0u32, 0u32);
    for (index, pixel) in pixels.iter().enumerate() {
        let &[red, green, blue, _] = pixel;
        let luminance = (u32::from(red) + u32::from(green) + u32::from(blue)) / 3;
        match luminance {
            0..=63 => dark += 1,
            64..=191 => middle += 1,
            _ => {}
        }
        if luminance <= 191 {
            let x = (index as u32 % width) as i64;
            let y = (index as u32 / width) as i64;
            let inside = x >= i64::from(claimed.x)
                && y >= i64::from(claimed.y)
                && x < i64::from(claimed.x + claimed.width)
                && y < i64::from(claimed.y + claimed.height);
            assert!(
                inside,
                "ink at ({x}, {y}) lies outside the bounds the run claims"
            );
        }
    }
    assert!(
        dark >= 20,
        "a line of glyphs must leave dark ink, found {dark} dark pixels"
    );
    assert!(
        middle >= 10,
        "anti-aliased glyph edges must leave pixels between ink and \
         background, found {middle}"
    );

    renderer.encode_png()
}

#[test]
fn text_at_scale_one() {
    assert_binary_snapshot!(".png", render_text(48, 32, 1.0));
}

#[test]
fn text_at_scale_two() {
    assert_binary_snapshot!(".png", render_text(96, 64, 2.0));
}
