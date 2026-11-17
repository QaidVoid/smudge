//! Checks that a glyph run's claimed ink bounds hold every pixel its glyphs
//! cover.
//!
//! The renderer clips each glyph to the ink bounds of its run, so bounds that
//! are too small would shave glyph edges the same way in every repaint, and no
//! comparison of damaged and full repaints could see it.

use std::rc::Rc;

use smudge_core::app::{App, AppConfig, FrameOutput, RepaintMode};
use smudge_core::geometry::{Color, PixelRect, Point, Rect, Size};
use smudge_core::paint::{DisplayItem, DisplayList, GlyphRun, PaintOp};
use smudge_cpu::CpuRenderer;
use smudge_text::{
    FontFamily, FontWeight, Span, SpanStyle, TextLayout, TextStyle, TextSystem, rich_text,
};

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

/// The styles the ink test draws: the bundled sans as its faces have it,
/// and the bundled mono, which has no bold face, asked for in bold, so its
/// runs are emboldened.
fn styles() -> [TextStyle; 2] {
    [
        TextStyle::default(),
        TextStyle {
            family: FontFamily::named("Noto Sans Mono"),
            weight: FontWeight::BOLD,
            ..TextStyle::default()
        },
    ]
}

/// Asserts that clipping the runs `runs_at` places at an origin to their
/// claimed ink bounds changes no pixel, at several scales and at origins
/// that fall between pixels.
fn assert_ink_is_claimed(runs_at: impl Fn(Point) -> Vec<GlyphRun>) {
    let surface = Rect::new(0.0, 0.0, 900.0, 40.0);
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let (width, height) = (
            (surface.width() * scale).ceil() as u32,
            (surface.height() * scale).ceil() as u32,
        );
        for offset in [0.0, 0.3, 0.55, 0.8] {
            let runs = runs_at(Point {
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
fn glyphs_draw_nothing_outside_their_claimed_ink_bounds() {
    TextSystem::bundled_only().install();
    let mut synthesized = 0;
    for style in styles() {
        let layout = TextLayout::new(TEXT, &style, None);
        assert_ink_is_claimed(|origin| layout.glyph_runs_at(origin, Color::BLACK));
        let runs = layout.glyph_runs_at(Point { x: 4.0, y: 4.0 }, Color::BLACK);
        if runs.iter().any(|run| run.embolden > 0.0) {
            synthesized += 1;
        }
    }
    assert_eq!(synthesized, 1, "the bold mono draws emboldened");
}

/// The glyph runs a rich text paragraph of `TEXT` paints in the bundled
/// mono, which has a regular face only, slanted, and emboldened too when
/// `weight` is bold.
fn slanted_mono_runs(weight: Option<FontWeight>) -> Vec<GlyphRun> {
    TextSystem::bundled_only().install();
    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 900.0,
                height: 40.0,
            },
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Full,
        },
        move || {
            rich_text(move || {
                Rc::from([Span::new(
                    TEXT,
                    SpanStyle {
                        monospace: true,
                        italic: true,
                        weight,
                        ..SpanStyle::default()
                    },
                )])
            })
        },
    );
    let frame = app.frame();
    frame
        .list
        .items()
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Op(PaintOp::Glyphs(run)) => Some(GlyphRun::clone(run)),
            _ => None,
        })
        .collect()
}

#[test]
fn slanted_glyphs_draw_nothing_outside_their_claimed_ink_bounds() {
    for weight in [None, Some(FontWeight::BOLD)] {
        let runs = slanted_mono_runs(weight);
        assert!(
            runs.iter().all(|run| run.skew != 0.0),
            "the italic mono draws slanted"
        );
        assert_ink_is_claimed(|origin| {
            runs.iter()
                .map(|run| {
                    let mut run = run.clone();
                    for glyph in &mut run.glyphs {
                        glyph.x += origin.x;
                        glyph.y += origin.y;
                    }
                    run.ink_bounds = run.ink_bounds.translated(origin);
                    run
                })
                .collect()
        });
    }
}

#[test]
fn a_quarter_pixel_shift_moves_the_glyph() {
    TextSystem::bundled_only().install();
    let layout = TextLayout::new("H", &TextStyle::default(), None);
    let at = |x: f32| {
        let runs = layout.glyph_runs_at(Point { x, y: 4.0 }, Color::BLACK);
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
        .glyph_runs_at(Point { x: 4.0, y: 4.0 }, Color::BLACK)
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
