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
use smudge_core::style::Edges;
use smudge_cpu::CpuRenderer;
use smudge_text::{FontFamily, FontWeight, TextLayout, TextStyle, TextSystem};

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

/// A gradient image stretched over a small rounded rect, and a translucent
/// one over another, at twice the scale, so bilinear sampling, premultiplied
/// blending and rounded-rect coverage are all in the snapshot.
fn image_scene() -> DisplayList {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::Image {
        rect: Rect::new(4.0, 4.0, 26.0, 22.0),
        radius: Corners::all(6.0),
        image: gradient_image(16, 12),
    });
    list.push_op(PaintOp::Image {
        rect: Rect::new(14.0, 12.0, 24.0, 26.0),
        radius: Corners::all(3.0),
        image: gradient_image(4, 4),
    });
    list.finish();
    list
}

/// A diagonal gradient with a translucent band, so sampling and
/// premultiplied blending both have something to interpolate.
fn gradient_image(width: u32, height: u32) -> smudge_core::image::Image {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let alpha: u32 = if y % 3 == 0 { 128 } else { 255 };
            let red = (x * 255) / (width - 1).max(1);
            let green = (y * 255) / (height - 1).max(1);
            let premultiply = |channel: u32| ((channel * alpha + 127) / 255) as u8;
            pixels.push(premultiply(red));
            pixels.push(premultiply(green));
            pixels.push(premultiply(90));
            pixels.push(alpha as u8);
        }
    }
    smudge_core::image::Image::from_premultiplied_rgba(width, height, pixels)
        .expect("the gradient fills the buffer")
}

/// Renders `list` into a fresh renderer with full-surface damage.
fn render_list(list: &DisplayList, width: u32, height: u32, scale: f32) -> Vec<u8> {
    let mut renderer = CpuRenderer::new(width, height);
    let damage = [PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    }];
    renderer.render(&FrameOutput {
        list,
        damage: &damage,
        full: true,
        clear_color: Color::WHITE,
        scale,
    });
    renderer.encode_png()
}

#[test]
fn rounded_image_at_scale_one() {
    let list = image_scene();
    assert_binary_snapshot!(".png", render_list(&list, 32, 32, 1.0));
}

#[test]
fn rounded_image_at_scale_two() {
    let list = image_scene();
    assert_binary_snapshot!(".png", render_list(&list, 64, 64, 2.0));
}

/// An image whose rect is smaller than its pixel size, so sampling skips
/// source pixels instead of stretching across them.
#[test]
fn downsampled_image_at_scale_one() {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::Image {
        rect: Rect::new(4.0, 4.0, 12.0, 10.0),
        radius: Corners::all(3.0),
        image: gradient_image(16, 12),
    });
    list.finish();
    assert_binary_snapshot!(".png", render_list(&list, 16, 14, 1.0));
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

/// A colored backdrop with two overlapping squares drawn as one group at
/// half opacity, so the overlap shows only the upper square.
fn group_scene() -> DisplayList {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: SCENE,
        radius: Corners::all(0.0),
        color: Color::rgb(40, 120, 200),
    });
    list.push_group(0.5);
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(6.0, 6.0, 28.0, 26.0),
        radius: Corners::all(4.0),
        color: Color::rgb(230, 60, 60),
    });
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(18.0, 10.0, 42.0, 28.0),
        radius: Corners::all(4.0),
        color: Color::rgb(250, 220, 60),
    });
    list.pop_group();
    list.finish();
    list
}

#[test]
fn a_group_at_half_opacity_over_a_colored_backdrop() {
    assert_binary_snapshot!(".png", render_list(&group_scene(), 48, 32, 1.0));
}

#[test]
fn nested_groups_multiply_their_opacities() {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: SCENE,
        radius: Corners::all(0.0),
        color: Color::rgb(245, 245, 245),
    });
    list.push_group(0.8);
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(4.0, 4.0, 44.0, 28.0),
        radius: Corners::all(6.0),
        color: Color::rgb(40, 40, 40),
    });
    list.push_group(0.5);
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(12.0, 8.0, 36.0, 24.0),
        radius: Corners::all(4.0),
        color: Color::rgb(60, 200, 120),
    });
    list.pop_group();
    list.pop_group();
    list.finish();
    assert_binary_snapshot!(".png", render_list(&list, 48, 32, 1.0));
}

#[test]
fn a_group_repainted_through_part_of_it_matches_a_full_repaint() {
    let list = group_scene();
    let full = {
        let mut renderer = CpuRenderer::new(48, 32);
        let damage = [PixelRect {
            x: 0,
            y: 0,
            width: 48,
            height: 32,
        }];
        renderer.render(&FrameOutput {
            list: &list,
            damage: &damage,
            full: true,
            clear_color: Color::WHITE,
            scale: 1.0,
        });
        renderer.pixels().to_vec()
    };
    let mut renderer = CpuRenderer::new(48, 32);
    let mut blank = DisplayList::new();
    blank.finish();
    let everything = [PixelRect {
        x: 0,
        y: 0,
        width: 48,
        height: 32,
    }];
    renderer.render(&FrameOutput {
        list: &blank,
        damage: &everything,
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    // The damage cuts through both squares of the group, so the group draws
    // into a layer that covers only part of it.
    let part = PixelRect {
        x: 15,
        y: 12,
        width: 20,
        height: 9,
    };
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[part],
        full: false,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    let damaged = renderer.pixels();
    for y in part.y..part.y + part.height {
        let start = ((y * 48 + part.x) * 4) as usize;
        let end = start + part.width as usize * 4;
        assert_eq!(&damaged[start..end], &full[start..end], "row {y}");
    }
}

/// The weights the variable font scene draws its three lines at.
const VARIABLE_WEIGHTS: [f32; 3] = [300.0, 500.0, 800.0];

/// The height of each line of the variable font scene, in pixels at scale
/// one.
const VARIABLE_LINE: f32 = 24.0;

/// Renders one line of the test variable font at each of
/// [`VARIABLE_WEIGHTS`], black on white, and returns the variation
/// coordinates of each line's runs with the PNG encoding of the surface and
/// how many dark pixels each line left.
fn render_variable_weights() -> (Vec<Vec<i16>>, Vec<u8>, Vec<u32>) {
    TextSystem::bundled_only().install();
    TextSystem::with(|system| system.register_font(smudge_test::VARIABLE_FONT))
        .expect("the test font is a font");
    let mut list = DisplayList::new();
    list.push_op(PaintOp::FillRect {
        rect: Rect::new(0.0, 0.0, 64.0, 72.0),
        radius: Corners::ZERO,
        color: Color::WHITE,
    });
    let mut coords = Vec::new();
    for (line, weight) in VARIABLE_WEIGHTS.into_iter().enumerate() {
        let style = TextStyle {
            family: FontFamily::named(smudge_test::VARIABLE_FAMILY),
            weight: FontWeight::new(weight),
            ..TextStyle::default()
        };
        let layout = TextLayout::new("Wgx", &style, None);
        let origin = Point {
            x: 4.0,
            y: 2.0 + line as f32 * VARIABLE_LINE,
        };
        for run in layout.glyph_runs_at(origin, Color::BLACK) {
            assert_eq!(run.embolden, 0.0, "the weight axis draws the weight");
            coords.push(run.coords.clone());
            list.push_op(PaintOp::Glyphs(Rc::new(run)));
        }
    }
    list.finish();

    let (width, height) = (64, 72);
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
        scale: 1.0,
    });
    let (pixels, _) = renderer.pixels().as_chunks::<4>();
    let mut dark = vec![0u32; VARIABLE_WEIGHTS.len()];
    for (index, &[red, green, blue, _]) in pixels.iter().enumerate() {
        let luminance = (u32::from(red) + u32::from(green) + u32::from(blue)) / 3;
        let line = (index as u32 / width) as usize / VARIABLE_LINE as usize;
        if luminance <= 63
            && let Some(count) = dark.get_mut(line)
        {
            *count += 1;
        }
    }
    (coords, renderer.encode_png(), dark)
}

#[test]
fn a_variable_font_draws_at_the_weight_the_text_asks_for() {
    let (coords, png, dark) = render_variable_weights();
    assert_eq!(coords.len(), VARIABLE_WEIGHTS.len());
    assert!(
        coords.iter().all(|coords| !coords.is_empty()),
        "every line names an instance off the default: {coords:?}"
    );
    assert!(
        coords[0] != coords[1] && coords[1] != coords[2],
        "each weight is its own instance: {coords:?}"
    );
    assert!(
        dark[0] < dark[1] && dark[1] < dark[2],
        "a heavier weight leaves more dark ink, found {dark:?}"
    );
    assert_binary_snapshot!(".png", png);
}

/// The color the border scenes draw their borders in.
const BORDER: Color = Color::rgb(30, 60, 140);

/// Renders a white surface of 48 by 40 pixels with one border of `widths`
/// on the rectangle from (4, 4) to (44, 36), rounded by `radius`, and
/// returns the renderer that drew it.
fn render_border(widths: Edges<f32>, radius: f32) -> CpuRenderer {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::StrokeSides {
        rect: Rect::new(4.0, 4.0, 44.0, 36.0),
        radius: Corners::all(radius),
        widths,
        color: BORDER,
    });
    list.finish();
    let (width, height) = (48, 40);
    let mut renderer = CpuRenderer::new(width, height);
    renderer.render(&FrameOutput {
        list: &list,
        damage: &[PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        }],
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    renderer
}

/// The pixel at `(x, y)` of a 48 pixel wide surface, without its alpha.
fn rgb_at(renderer: &CpuRenderer, x: usize, y: usize) -> [u8; 3] {
    let start = (y * 48 + x) * 4;
    let pixel = &renderer.pixels()[start..start + 3];
    [pixel[0], pixel[1], pixel[2]]
}

/// The border color without its alpha.
const BORDER_RGB: [u8; 3] = [30, 60, 140];

/// White without its alpha.
const WHITE_RGB: [u8; 3] = [255, 255, 255];

#[test]
fn a_bottom_border_alone_draws_one_band_along_the_bottom() {
    let renderer = render_border(
        Edges {
            top: 0.0,
            right: 0.0,
            bottom: 3.0,
            left: 0.0,
        },
        0.0,
    );
    for x in [4, 24, 43] {
        for y in 33..36 {
            assert_eq!(
                rgb_at(&renderer, x, y),
                BORDER_RGB,
                "the band at ({x}, {y})"
            );
        }
        assert_eq!(rgb_at(&renderer, x, 32), WHITE_RGB, "above the band at {x}");
        assert_eq!(rgb_at(&renderer, x, 4), WHITE_RGB, "no top border at {x}");
    }
    assert_eq!(rgb_at(&renderer, 4, 20), WHITE_RGB, "no left border");
    assert_eq!(rgb_at(&renderer, 43, 20), WHITE_RGB, "no right border");
    assert_binary_snapshot!(".png", renderer.encode_png());
}

#[test]
fn four_border_widths_each_draw_their_side_inside_rounded_corners() {
    let renderer = render_border(
        Edges {
            top: 1.0,
            right: 2.0,
            bottom: 4.0,
            left: 6.0,
        },
        8.0,
    );
    let (mid_x, mid_y) = (24, 20);
    assert_eq!(rgb_at(&renderer, mid_x, 4), BORDER_RGB, "the top band");
    assert_eq!(rgb_at(&renderer, mid_x, 5), WHITE_RGB, "one pixel of top");
    assert_eq!(rgb_at(&renderer, 42, mid_y), BORDER_RGB, "the right band");
    assert_eq!(
        rgb_at(&renderer, 41, mid_y),
        WHITE_RGB,
        "two pixels of right"
    );
    assert_eq!(rgb_at(&renderer, mid_x, 32), BORDER_RGB, "the bottom band");
    assert_eq!(
        rgb_at(&renderer, mid_x, 31),
        WHITE_RGB,
        "four pixels of bottom"
    );
    assert_eq!(rgb_at(&renderer, 9, mid_y), BORDER_RGB, "the left band");
    assert_eq!(
        rgb_at(&renderer, 10, mid_y),
        WHITE_RGB,
        "six pixels of left"
    );
    assert_eq!(
        rgb_at(&renderer, 4, 4),
        WHITE_RGB,
        "the rounded corner leaves its outer pixel unpainted"
    );
    assert_binary_snapshot!(".png", renderer.encode_png());
}

/// Renders a list holding one border into a 64 by 40 surface, whole, and
/// returns its pixels.
fn border_pixels(list: &DisplayList, damage: Option<PixelRect>) -> Vec<u8> {
    let (width, height) = (64, 40);
    let everything = PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let mut renderer = CpuRenderer::new(width, height);
    let mut blank = DisplayList::new();
    blank.finish();
    renderer.render(&FrameOutput {
        list: if damage.is_some() { &blank } else { list },
        damage: &[everything],
        full: true,
        clear_color: Color::WHITE,
        scale: 1.0,
    });
    if let Some(part) = damage {
        renderer.render(&FrameOutput {
            list,
            damage: &[part],
            full: false,
            clear_color: Color::WHITE,
            scale: 1.0,
        });
    }
    renderer.pixels().to_vec()
}

#[test]
fn a_border_of_unequal_sides_repainted_from_any_column_matches_a_full_repaint() {
    let borders = [
        (
            Rect::new(2.0, 0.5, 20.25, 21.25),
            5.0,
            Edges {
                top: 0.0,
                right: 2.75,
                bottom: 5.0,
                left: 0.0,
            },
        ),
        (
            Rect::new(5.0, 1.5, 47.5, 28.25),
            25.5,
            Edges {
                top: 0.25,
                right: 0.0,
                bottom: 2.5,
                left: 0.0,
            },
        ),
        (
            Rect::new(4.0, 4.0, 44.0, 24.0),
            10.0,
            Edges {
                top: 0.0,
                right: 0.0,
                bottom: 4.0,
                left: 0.0,
            },
        ),
        (
            Rect::new(2.0, 2.0, 42.0, 22.0),
            10.0,
            Edges {
                top: 0.0,
                right: 0.0,
                bottom: 15.0,
                left: 0.0,
            },
        ),
    ];
    for (rect, radius, widths) in borders {
        let mut list = DisplayList::new();
        list.push_op(PaintOp::StrokeSides {
            rect,
            radius: Corners::all(radius),
            widths,
            color: BORDER,
        });
        list.finish();
        let full = border_pixels(&list, None);
        for x in 0..48 {
            let part = PixelRect {
                x,
                y: 0,
                width: 64 - x,
                height: 40,
            };
            let damaged = border_pixels(&list, Some(part));
            for y in 0..40 {
                let row =
                    |pixels: &[u8]| pixels[(y * 64 + x as usize) * 4..(y + 1) * 64 * 4].to_vec();
                assert_eq!(
                    row(&damaged),
                    row(&full),
                    "{rect:?} rounded {radius} with {widths:?}, row {y} repainted from column {x}"
                );
            }
        }
    }
}

#[test]
fn a_pill_with_a_bottom_border_leaves_its_top_corners_blank() {
    let mut list = DisplayList::new();
    list.push_op(PaintOp::StrokeSides {
        rect: Rect::new(4.0, 4.0, 44.0, 24.0),
        radius: Corners::all(10.0),
        widths: Edges {
            top: 0.0,
            right: 0.0,
            bottom: 4.0,
            left: 0.0,
        },
        color: BORDER,
    });
    list.finish();
    let pixels = border_pixels(&list, None);
    let (pixels, _) = pixels.as_chunks::<4>();
    for y in 4..12 {
        for x in 4..44 {
            let [red, ..] = pixels[y * 64 + x];
            assert_eq!(red, 255, "ink at ({x}, {y}), above the bottom border");
        }
    }
}
