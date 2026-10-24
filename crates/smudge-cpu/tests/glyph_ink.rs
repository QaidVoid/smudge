//! Checks that a glyph run's claimed ink bounds hold every pixel its glyphs
//! cover.
//!
//! The renderer clips each glyph to the ink bounds of its run, so bounds that
//! are too small would shave glyph edges the same way in every repaint, and no
//! comparison of damaged and full repaints could see it.

use std::rc::Rc;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, PixelRect, Point, Rect};
use smudge_core::paint::{DisplayList, GlyphRun, PaintOp};
use smudge_cpu::CpuRenderer;
use smudge_text::{TextLayout, TextStyle, TextSystem};

/// Every printable ASCII character, so the runs hold ascenders, descenders and
/// glyphs that reach past their advance.
const TEXT: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// Renders `runs` over the whole surface and returns its pixels.
fn render(runs: &[GlyphRun], width: u32, height: u32, scale: f32) -> Vec<u8> {
    let mut list = DisplayList::new();
    for run in runs {
        list.push_op(PaintOp::Glyphs(Rc::new(run.clone())));
    }
    list.finish();
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
        scale,
    });
    renderer.pixels().to_vec()
}

#[test]
fn glyphs_draw_nothing_outside_their_claimed_ink_bounds() {
    TextSystem::bundled_only().install();
    let style = TextStyle::default();
    let layout = TextLayout::new(TEXT, &style, None);
    let surface = Rect::new(0.0, 0.0, 900.0, 40.0);

    for scale in [1.0, 1.25, 1.5, 2.0] {
        let (width, height) = (
            (surface.width() * scale).ceil() as u32,
            (surface.height() * scale).ceil() as u32,
        );
        for offset in [0.0, 0.3, 0.55, 0.8] {
            let runs = layout.glyph_runs_at(Point {
                x: 4.0 + offset,
                y: 4.0 + offset,
            });
            let unclipped: Vec<GlyphRun> = runs
                .iter()
                .map(|run| GlyphRun {
                    ink_bounds: surface,
                    ..run.clone()
                })
                .collect();

            assert!(
                render(&runs, width, height, scale) == render(&unclipped, width, height, scale),
                "at scale {scale} and offset {offset}, clipping to the claimed ink \
                 bounds changed the image, so a glyph covers pixels its run does \
                 not claim"
            );
        }
    }
}

#[test]
fn a_quarter_pixel_shift_moves_the_glyph() {
    TextSystem::bundled_only().install();
    let layout = TextLayout::new("H", &TextStyle::default(), None);
    let at = |x: f32| {
        let runs = layout.glyph_runs_at(Point { x, y: 4.0 });
        render(&runs, 32, 32, 1.0)
    };

    assert!(
        at(4.0) != at(4.5),
        "a glyph half a pixel to the right must shade its stems differently"
    );
    assert!(
        at(4.0) == at(4.1),
        "positions within one quarter pixel share a mask"
    );
}

#[test]
fn a_glyph_far_outside_the_surface_draws_nothing() {
    TextSystem::bundled_only().install();
    let layout = TextLayout::new("H", &TextStyle::default(), None);
    let runs: Vec<GlyphRun> = layout
        .glyph_runs_at(Point { x: 4.0, y: 4.0 })
        .into_iter()
        .map(|mut run| {
            for glyph in &mut run.glyphs {
                glyph.x = 1.0e30;
            }
            run.ink_bounds = Rect::new(0.0, 0.0, 32.0, 32.0);
            run
        })
        .collect();

    assert_eq!(
        render(&runs, 32, 32, 1.0),
        render(&[], 32, 32, 1.0),
        "a glyph a long way off the surface leaves it blank"
    );
}
