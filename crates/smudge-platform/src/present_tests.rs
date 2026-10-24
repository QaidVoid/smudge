//! Tests for copying the renderer's surface into the window buffer and for
//! choosing the region a present has to copy.

use smudge_core::geometry::PixelRect;

use super::{DamageHistory, copy_rect, damage_region, present_skipped, tint_rects};

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
    let history = DamageHistory::default();
    let mut out = Vec::new();

    assert!(damage_region(1, &[rect(3, 4)], false, &history, &mut out));
    assert_eq!(out, [rect(3, 4)]);
}

#[test]
fn older_ages_add_the_damage_of_the_frames_they_cover() {
    let mut history = DamageHistory::default();
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

    assert!(damage_region(1, &[rect(0, 0)], false, &history, &mut out));
    assert!(!damage_region(2, &[rect(0, 0)], false, &history, &mut out));
    assert!(!damage_region(3, &[rect(0, 0)], false, &history, &mut out));

    history.push(&[rect(1, 0)], false);
    assert!(damage_region(2, &[rect(0, 0)], false, &history, &mut out));
    assert!(!damage_region(3, &[rect(0, 0)], false, &history, &mut out));

    history.push(&[rect(2, 0)], false);
    assert!(damage_region(3, &[rect(0, 0)], false, &history, &mut out));
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
