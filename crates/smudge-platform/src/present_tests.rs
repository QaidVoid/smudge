//! Tests for drawing or copying a frame into the window buffer and for
//! choosing the region a present has to carry.

use smudge_core::app::FrameOutput;
use smudge_core::geometry::{Color, Corners, PixelRect, Rect};
use smudge_core::paint::{DisplayList, PaintOp};
use smudge_cpu::{CpuRenderer, XrgbRenderer};

use super::{
    CpuFrames, DamageHistory, copy_rect, damage_region, draw_direct, present_skipped, tint_rects,
};

/// Returns a one-pixel rectangle at the given position, so tests can tell
/// rectangles apart by their position.
fn rect(x: u32, y: u32) -> PixelRect {
    PixelRect {
        x,
        y,
        width: 1,
        height: 1,
    }
}

#[test]
fn copy_rect_writes_only_the_rect() {
    let mut source = Vec::new();
    for y in 0..3u32 {
        for x in 0..4u32 {
            source.extend_from_slice(&[x as u8, y as u8, 32, 255]);
        }
    }
    let mut destination = [0x00aa_bbcc_u32; 12];

    copy_rect(
        &source,
        4,
        &mut destination,
        PixelRect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        },
    );

    assert_eq!(&destination[..4], &[0x00aa_bbcc; 4]);
    assert_eq!(
        &destination[4..8],
        &[0x00aa_bbcc, 0x0001_0120, 0x0002_0120, 0x00aa_bbcc]
    );
    assert_eq!(
        &destination[8..12],
        &[0x00aa_bbcc, 0x0001_0220, 0x0002_0220, 0x00aa_bbcc]
    );
}

#[test]
fn copy_rect_drops_the_alpha_channel() {
    let source = [200, 150, 100, 77];
    let mut destination = [0_u32; 1];

    copy_rect(
        &source,
        1,
        &mut destination,
        PixelRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    );

    assert_eq!(destination, [0x00c8_9664]);
}

#[test]
fn unknown_ages_copy_everything() {
    let history = DamageHistory::default();
    let mut out = Vec::new();

    for age in [0_u8, 4, 200] {
        assert!(!damage_region(
            age,
            &[rect(0, 0)],
            false,
            &history,
            &mut out
        ));
        assert!(out.is_empty());
    }
}

#[test]
fn a_full_current_frame_copies_everything() {
    let history = DamageHistory::default();
    let mut out = Vec::new();

    for age in [1_u8, 2, 3] {
        assert!(!damage_region(age, &[rect(0, 0)], true, &history, &mut out));
        assert!(out.is_empty());
    }
}

#[test]
fn age_one_presents_only_the_current_damage() {
    let mut history = DamageHistory::default();
    history.push(&[rect(9, 9)], false);
    let mut out = Vec::new();

    assert!(damage_region(1, &[rect(3, 4)], false, &history, &mut out));
    assert_eq!(out, [rect(3, 4)]);
}

#[test]
fn older_ages_add_the_damage_of_the_frames_they_cover() {
    let mut history = DamageHistory::default();
    history.push(&[rect(9, 9)], false);
    history.push(&[rect(1, 0)], false);
    history.push(&[rect(2, 0)], false);
    let mut out = Vec::new();

    assert!(damage_region(2, &[rect(0, 0)], false, &history, &mut out));
    assert_eq!(out, [rect(0, 0), rect(2, 0)]);

    assert!(damage_region(3, &[rect(0, 0)], false, &history, &mut out));
    assert_eq!(out, [rect(0, 0), rect(2, 0), rect(1, 0)]);
}

#[test]
fn age_one_ignores_a_full_previous_frame() {
    let mut history = DamageHistory::default();
    history.push(&[rect(0, 0)], true);
    let mut out = Vec::new();

    assert!(damage_region(1, &[rect(1, 1)], false, &history, &mut out));
    assert_eq!(out, [rect(1, 1)]);
}

#[test]
fn a_full_previous_frame_copies_everything_for_the_ages_it_covers() {
    let mut history = DamageHistory::default();
    history.push(&[rect(1, 0)], false);
    history.push(&[rect(2, 0)], true);
    let mut out = Vec::new();

    assert!(!damage_region(2, &[rect(0, 0)], false, &history, &mut out));
    assert!(out.is_empty());
    assert!(!damage_region(3, &[rect(0, 0)], false, &history, &mut out));
    assert!(out.is_empty());
}

#[test]
fn a_full_second_previous_frame_only_blocks_age_three() {
    let mut history = DamageHistory::default();
    history.push(&[rect(1, 0)], true);
    history.push(&[rect(2, 0)], false);
    let mut out = Vec::new();

    assert!(damage_region(2, &[rect(0, 0)], false, &history, &mut out));
    assert_eq!(out, [rect(0, 0), rect(2, 0)]);
    assert!(!damage_region(3, &[rect(0, 0)], false, &history, &mut out));
    assert!(out.is_empty());
}

#[test]
fn a_full_third_previous_frame_blocks_no_age() {
    let mut history = DamageHistory::default();
    history.push(&[rect(3, 0)], true);
    history.push(&[rect(2, 0)], false);
    history.push(&[rect(1, 0)], false);
    let mut out = Vec::new();

    for age in [1_u8, 2, 3] {
        assert!(damage_region(age, &[rect(0, 0)], false, &history, &mut out));
        assert_eq!(out[0], rect(0, 0));
    }
}

#[test]
fn missing_history_copies_everything() {
    let mut history = DamageHistory::default();
    let mut out = Vec::new();

    assert!(!damage_region(1, &[rect(0, 0)], false, &history, &mut out));

    history.push(&[rect(1, 0)], false);
    assert!(damage_region(1, &[rect(0, 0)], false, &history, &mut out));
    assert!(!damage_region(2, &[rect(0, 0)], false, &history, &mut out));

    history.push(&[rect(2, 0)], false);
    assert!(damage_region(2, &[rect(0, 0)], false, &history, &mut out));
    assert!(!damage_region(3, &[rect(0, 0)], false, &history, &mut out));

    history.push(&[rect(3, 0)], false);
    assert!(damage_region(3, &[rect(0, 0)], false, &history, &mut out));

    history.clear();
    assert!(!damage_region(1, &[rect(0, 0)], false, &history, &mut out));
}

#[test]
fn the_ring_keeps_only_the_last_three_frames() {
    let mut history = DamageHistory::default();
    history.push(&[rect(9, 9)], false);
    history.push(&[rect(1, 0)], false);
    history.push(&[rect(2, 0)], false);
    history.push(&[rect(3, 0)], false);
    let mut out = Vec::new();

    assert!(damage_region(3, &[rect(0, 0)], false, &history, &mut out));
    assert_eq!(out, [rect(0, 0), rect(3, 0), rect(2, 0)]);
}

#[test]
fn a_frame_without_damage_copies_everything() {
    let mut history = DamageHistory::default();
    history.push(&[rect(1, 0)], false);
    history.push(&[rect(2, 0)], false);
    let mut out = Vec::new();

    for age in [1_u8, 2, 3] {
        assert!(!damage_region(age, &[], false, &history, &mut out));
        assert!(out.is_empty());
    }
}

#[test]
fn an_unchanged_frame_with_a_tick_skips_its_present() {
    assert!(present_skipped(&[], true, false));
}

#[test]
fn an_unchanged_frame_without_a_tick_presents_to_restore_the_window() {
    assert!(!present_skipped(&[], false, false));
}

#[test]
fn an_unchanged_frame_with_a_tick_presents_while_the_damage_overlay_is_on() {
    assert!(!present_skipped(&[], true, true));
}

#[test]
fn a_changed_frame_with_a_tick_presents() {
    assert!(!present_skipped(&[rect(0, 0)], true, false));
}

#[test]
fn a_region_that_changes_every_frame_is_copied_once() {
    let wide = PixelRect {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    let mut history = DamageHistory::default();
    history.push(&[rect(9, 9)], false);
    history.push(&[rect(5, 5), wide], false);
    history.push(&[wide], false);
    let mut out = Vec::new();

    assert!(damage_region(3, &[wide], false, &history, &mut out));
    assert_eq!(
        out,
        [wide, rect(5, 5)],
        "the rectangles the current damage contains add nothing to copy"
    );
}

/// Returns a rectangle from its position and size.
fn area(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
    PixelRect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn a_past_strip_that_overlaps_the_current_one_merges_into_it() {
    let mut history = DamageHistory::default();
    history.push(&[area(90, 90, 1, 1)], false);
    history.push(&[area(10, 40, 20, 100)], false);
    let mut out = Vec::new();

    assert!(damage_region(
        2,
        &[area(10, 0, 20, 100)],
        false,
        &history,
        &mut out
    ));
    assert_eq!(
        out,
        [area(10, 0, 20, 140)],
        "the union holds fewer pixels than the two strips drawn apart"
    );
}

#[test]
fn past_rectangles_whose_union_adds_pixels_stay_apart() {
    let across = area(0, 10, 100, 4);
    let down = area(10, 0, 4, 100);
    let mut history = DamageHistory::default();
    history.push(&[area(90, 90, 1, 1)], false);
    history.push(&[down], false);
    let mut out = Vec::new();

    assert!(damage_region(2, &[across], false, &history, &mut out));
    assert_eq!(out, [across, down], "two crossing bars stay two bars");
}

#[test]
fn the_tint_changes_only_the_pixels_inside_the_damage_rect() {
    let outside = 0x0010_2030_u32;
    let mut buffer = [outside; 9];

    tint_rects(
        &mut buffer,
        3,
        &[PixelRect {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        }],
    );

    assert_eq!(buffer[4], 0x006a_2c36);
    let mut expected = [outside; 9];
    expected[4] = buffer[4];
    assert_eq!(buffer, expected);
}

#[test]
fn the_tint_blends_toward_its_color_instead_of_replacing() {
    let mut buffer = [0x00ff_ffff_u32];

    tint_rects(
        &mut buffer,
        1,
        &[PixelRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }],
    );

    assert_eq!(buffer, [0x00ff_b7b7]);
}

/// The side of the surface the swap chain tests draw, in physical pixels.
const CHAIN_SIZE: u32 = 64;

/// One translucent, rounded box per entry, in logical pixels.
fn boxes_list(boxes: &[(Rect, Color)]) -> DisplayList {
    let mut list = DisplayList::new();
    for &(rect, color) in boxes {
        list.push_op(PaintOp::FillRect {
            rect,
            radius: Corners::all(3.0),
            color,
        });
    }
    list.finish();
    list
}

/// Returns a full repaint of `list` as the `0x00RRGGBB` words a window
/// shows.
fn full_xrgb(list: &DisplayList, scale: f32) -> Vec<u32> {
    let mut renderer = CpuRenderer::new(CHAIN_SIZE, CHAIN_SIZE);
    renderer.render(&FrameOutput {
        list,
        damage: &[PixelRect {
            x: 0,
            y: 0,
            width: CHAIN_SIZE,
            height: CHAIN_SIZE,
        }],
        full: true,
        clear_color: Color::WHITE,
        scale,
    });
    let mut buffer = vec![0; (CHAIN_SIZE * CHAIN_SIZE) as usize];
    copy_rect(
        renderer.pixels(),
        CHAIN_SIZE,
        &mut buffer,
        PixelRect {
            x: 0,
            y: 0,
            width: CHAIN_SIZE,
            height: CHAIN_SIZE,
        },
    );
    buffer
}

#[test]
fn direct_presents_match_a_full_repaint_at_every_buffer_age() {
    for buffers in 1..=3_usize {
        for scale in [1.0, 1.5] {
            let side = CHAIN_SIZE as f32 / scale;
            let mut seed = 0x9e37_79b9_u32 ^ buffers as u32;
            let mut next = move || {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed
            };
            let mut boxes: Vec<(Rect, Color)> = (0..6)
                .map(|index| {
                    let at = index as f32 * 6.0;
                    (
                        Rect::new(at, at, at + 12.0, at + 9.0),
                        Color::rgba(40 * index as u8, 90, 200, 150 + 15 * index as u8),
                    )
                })
                .collect();
            let mut renderer = XrgbRenderer::new(CHAIN_SIZE, CHAIN_SIZE);
            let mut chain =
                vec![vec![0xffff_ffff_u32; (CHAIN_SIZE * CHAIN_SIZE) as usize]; buffers];
            let mut presented_at = vec![None; buffers];
            let mut history = DamageHistory::default();
            let mut region = Vec::new();

            for frame_index in 0..40_usize {
                // The first frame and every thirteenth repaint everything,
                // every eleventh changes nothing as a window system redraw
                // does, and the rest move one or two boxes.
                let full = frame_index % 13 == 0;
                let mut damage = Vec::new();
                if full {
                    damage.push(PixelRect {
                        x: 0,
                        y: 0,
                        width: CHAIN_SIZE,
                        height: CHAIN_SIZE,
                    });
                } else if frame_index % 11 != 0 {
                    for _ in 0..1 + next() % 2 {
                        let index = next() as usize % boxes.len();
                        let old = boxes[index].0;
                        let x = (next() % 400) as f32 / 400.0 * (side - 14.0);
                        let y = (next() % 400) as f32 / 400.0 * (side - 11.0);
                        let moved = Rect::new(x, y, x + 13.5, y + 10.25);
                        boxes[index].0 = moved;
                        for rect in [old, moved] {
                            damage.extend(PixelRect::from_logical(
                                rect, scale, CHAIN_SIZE, CHAIN_SIZE,
                            ));
                        }
                    }
                }
                let list = boxes_list(&boxes);
                let slot = frame_index % buffers;
                let age = presented_at[slot].map_or(0, |at: usize| (frame_index - at) as u8);
                let output = FrameOutput {
                    list: &list,
                    damage: &damage,
                    full,
                    clear_color: Color::WHITE,
                    scale,
                };

                draw_direct(
                    &mut renderer,
                    &output,
                    age,
                    &history,
                    &mut region,
                    &mut chain[slot],
                );
                history.push(&damage, full);
                presented_at[slot] = Some(frame_index);

                let expected = full_xrgb(&list, scale);
                let wrong = chain[slot]
                    .iter()
                    .zip(&expected)
                    .position(|(shown, expected)| shown != expected);
                assert_eq!(
                    wrong, None,
                    "{buffers} buffers at scale {scale}, frame {frame_index} at age {age}"
                );
            }
        }
    }
}

#[test]
fn a_frame_without_damage_draws_only_what_the_buffer_misses() {
    let list = boxes_list(&[(Rect::new(4.0, 4.0, 20.0, 20.0), Color::BLACK)]);
    let mut renderer = XrgbRenderer::new(CHAIN_SIZE, CHAIN_SIZE);
    let mut buffer = vec![0x0012_3456_u32; (CHAIN_SIZE * CHAIN_SIZE) as usize];
    let mut history = DamageHistory::default();
    history.push(&[rect(0, 0)], true);
    let mut region = Vec::new();
    let frame = FrameOutput {
        list: &list,
        damage: &[],
        full: false,
        clear_color: Color::WHITE,
        scale: 1.0,
    };

    let with_damage = draw_direct(&mut renderer, &frame, 1, &history, &mut region, &mut buffer);

    assert!(!with_damage, "a frame without damage presents everything");
    assert!(
        buffer.iter().all(|&pixel| pixel == 0x0012_3456),
        "a buffer that holds the previous frame needs no pixel drawn"
    );
}

/// A window's buffers as a platform hands them out in turn, each filled with
/// garbage until a frame is drawn into it.
struct SwapChain {
    /// The buffers, one `0x00RRGGBB` word per pixel.
    buffers: Vec<Vec<u32>>,
    /// The present each buffer last showed, by its index.
    presented_at: Vec<Option<usize>>,
    /// How many presents happened.
    presents: usize,
    /// Whether the platform reports buffer ages, or only ever age 0.
    reports_ages: bool,
}

impl SwapChain {
    fn new(count: usize, side: u32, reports_ages: bool) -> SwapChain {
        SwapChain {
            buffers: vec![vec![0x00ab_cdef; (side * side) as usize]; count],
            presented_at: vec![None; count],
            presents: 0,
            reports_ages,
        }
    }

    /// Replaces every buffer with a fresh one of garbage, as a resize does.
    fn resize(&mut self, side: u32) {
        let presents = self.presents;
        *self = SwapChain::new(self.buffers.len(), side, self.reports_ages);
        self.presents = presents;
    }

    /// Draws `frame` into the next buffer, presents it, and returns what the
    /// draw did and the buffer's contents.
    fn present(
        &mut self,
        frames: &mut CpuFrames,
        frame: &FrameOutput<'_>,
    ) -> (super::Drawn, &[u32]) {
        let slot = self.presents % self.buffers.len();
        let age = match self.presented_at[slot] {
            Some(at) if self.reports_ages => (self.presents - at) as u8,
            _ => 0,
        };
        let drawn = frames.draw(frame, age, &mut self.buffers[slot]);
        frames.presented(frame);
        self.presented_at[slot] = Some(self.presents);
        self.presents += 1;
        (drawn, &self.buffers[slot])
    }
}

/// The damage of moving a box from `from` to `to`, in physical pixels at
/// scale 1.
fn moved(from: Rect, to: Rect) -> Vec<PixelRect> {
    [from, to]
        .into_iter()
        .filter_map(|rect| PixelRect::from_logical(rect, 1.0, CHAIN_SIZE, CHAIN_SIZE))
        .collect()
}

/// Runs `frames` through a swap chain for `count` frames, the first a full
/// repaint and the rest moving one box, and calls `check` with each frame's
/// index, what the draw did and whether the buffer matched a full repaint.
fn run_frames(
    frames: &mut CpuFrames,
    chain: &mut SwapChain,
    count: usize,
    mut check: impl FnMut(usize, &super::Drawn),
) {
    let whole = PixelRect {
        x: 0,
        y: 0,
        width: CHAIN_SIZE,
        height: CHAIN_SIZE,
    };
    let mut at = Rect::new(2.0, 2.0, 14.0, 12.0);
    for index in 0..count {
        let next = Rect::new(at.x0 + 3.0, at.y0 + 2.0, at.x1 + 3.0, at.y1 + 2.0);
        let damage = if index == 0 {
            vec![whole]
        } else {
            moved(at, next)
        };
        if index > 0 {
            at = next;
        }
        let list = boxes_list(&[(at, Color::rgba(30, 90, 200, 180))]);
        let frame = FrameOutput {
            list: &list,
            damage: &damage,
            full: index == 0,
            clear_color: Color::WHITE,
            scale: 1.0,
        };
        let (drawn, shown) = chain.present(frames, &frame);
        assert!(
            shown == full_xrgb(&list, 1.0).as_slice(),
            "frame {index} does not match a full repaint"
        );
        check(index, &drawn);
    }
}

#[test]
fn a_triple_buffered_surface_keeps_drawing_into_its_buffers() {
    let mut frames = CpuFrames::new(CHAIN_SIZE, CHAIN_SIZE, false);
    let mut chain = SwapChain::new(3, CHAIN_SIZE, true);

    run_frames(&mut frames, &mut chain, 12, |index, drawn| {
        assert_eq!(drawn.bytes_copied, 0, "frame {index} copied from a pixmap");
        assert_eq!(drawn.with_damage, index >= 3, "frame {index}");
    });
}

#[test]
fn a_surface_without_buffer_ages_falls_back_to_copying_the_whole_frame() {
    let mut frames = CpuFrames::new(CHAIN_SIZE, CHAIN_SIZE, false);
    let mut chain = SwapChain::new(2, CHAIN_SIZE, false);
    let whole_bytes = u64::from(CHAIN_SIZE * CHAIN_SIZE * 4);

    run_frames(&mut frames, &mut chain, 10, |index, drawn| {
        let expected = if index < 3 { 0 } else { whole_bytes };
        assert_eq!(drawn.bytes_copied, expected, "frame {index}");
        assert!(
            !drawn.with_damage,
            "frame {index} presented only part of the surface"
        );
    });
}

#[test]
fn a_resize_forgets_the_frames_presented_before_it() {
    for reports_ages in [true, false] {
        let mut frames = CpuFrames::new(CHAIN_SIZE, CHAIN_SIZE, false);
        let mut chain = SwapChain::new(1, CHAIN_SIZE, reports_ages);
        run_frames(&mut frames, &mut chain, 6, |_, _| {});

        // The resized buffers hold garbage while the platform still claims
        // the one buffer holds the last frame, and the frame after the
        // resize damages only the box it moved.
        frames.resize(CHAIN_SIZE, CHAIN_SIZE);
        chain.resize(CHAIN_SIZE);
        chain.presented_at[0] = Some(chain.presents - 1);
        let from = Rect::new(20.0, 20.0, 30.0, 30.0);
        let to = Rect::new(24.0, 22.0, 34.0, 32.0);
        let damage = moved(from, to);
        let list = boxes_list(&[(to, Color::rgba(30, 90, 200, 180))]);
        let frame = FrameOutput {
            list: &list,
            damage: &damage,
            full: false,
            clear_color: Color::WHITE,
            scale: 1.0,
        };
        let (drawn, shown) = chain.present(&mut frames, &frame);

        assert!(!drawn.with_damage, "ages {reports_ages}");
        assert!(
            shown == full_xrgb(&list, 1.0).as_slice(),
            "ages {reports_ages}: the frame after a resize does not match a full repaint"
        );
    }
}
