//! Behavior tests for the glyph mask cache.

use std::sync::Arc;

use smudge_core::geometry::{Color, Point};
use smudge_core::paint::FontBlob;
use smudge_text::{FontFamily, FontWeight, TextLayout, TextStyle, TextSystem};

use crate::glyph_cache::{GlyphCache, MAX_INSTANCES, MaskKey};

/// Installs a bundled-only system, so the masks come from the bundled fonts
/// and nothing depends on the fonts installed on the machine.
fn setup() {
    TextSystem::bundled_only().install();
}

/// A font blob and the mask keys of one glyph of the letter at `index` in the
/// text, rasterized at 64 physical pixels.
///
/// The keys differ between calls by their size, in steps of 1/64 of a pixel,
/// so each call produces a distinct cache entry whose mask is as wide and tall
/// as the same ink.
fn keys(text: &str, index: usize, count: usize) -> (FontBlob, Vec<MaskKey>) {
    let style = TextStyle::default();
    let layout = TextLayout::new(text, &style, None);
    let runs = layout.glyph_runs_at(Point { x: 0.0, y: 64.0 }, Color::BLACK);
    let run = runs.first().expect("the text has at least one run");
    let glyph = run.glyphs[index];
    let keys = (0..count)
        .map(|step| MaskKey {
            font_id: run.font.id,
            font_index: run.font.index,
            glyph_id: glyph.id,
            size: 64 * 64 + step as i32,
            embolden: 0,
            skew: 0,
            subpixel: 0,
            instance: 0,
        })
        .collect();
    (clone_font_blob(&run.font), keys)
}

/// Copies a run's font into a blob that outlives the layout.
fn clone_font_blob(font: &FontBlob) -> FontBlob {
    FontBlob {
        id: font.id,
        data: Arc::clone(&font.data),
        index: font.index,
    }
}

#[test]
fn repeated_draw_hits_the_cache() {
    setup();

    let (font, keys) = keys("M", 0, 1);
    let mut cache = GlyphCache::new();

    assert!(cache.entry(keys[0], &font, &[]).is_some());
    assert_eq!(cache.misses(), 1, "the first draw rasterizes the mask");
    assert_eq!(cache.hits(), 0);

    assert!(cache.entry(keys[0], &font, &[]).is_some());
    assert_eq!(cache.misses(), 1, "the second draw reuses the mask");
    assert_eq!(cache.hits(), 1);

    assert!(
        cache.entry(keys[0], &font, &[]).is_some() && cache.entry(keys[0], &font, &[]).is_some(),
        "the mask stays cached"
    );
    assert_eq!(cache.hits(), 3);
    assert_eq!(cache.misses(), 1);
}

#[test]
fn a_glyph_without_outline_is_cached_as_empty() {
    setup();

    let style = TextStyle::default();
    let layout = TextLayout::new("a b", &style, None);
    let runs = layout.glyph_runs_at(Point { x: 0.0, y: 20.0 }, Color::BLACK);
    let run = runs.first().expect("the text shapes to a run");
    let font = clone_font_blob(&run.font);
    let keys: Vec<_> = run
        .glyphs
        .iter()
        .map(|glyph| MaskKey {
            font_id: run.font.id,
            font_index: run.font.index,
            glyph_id: glyph.id,
            size: 14 * 64,
            embolden: 0,
            skew: 0,
            subpixel: 0,
            instance: 0,
        })
        .collect();

    let mut cache = GlyphCache::new();
    let mut empty = false;
    for key in &keys {
        let mask = cache
            .entry(*key, &font, &[])
            .expect("every glyph gets a mask");
        empty |= mask.width() == 0;
    }
    assert!(empty, "the space has no outline, so one mask is empty");
    assert_eq!(
        cache.misses(),
        keys.len() as u64,
        "each glyph rasterized once"
    );

    for key in &keys {
        assert!(cache.entry(*key, &font, &[]).is_some());
    }
    assert_eq!(
        cache.misses(),
        keys.len() as u64,
        "the empty mask is cached like any other"
    );
}

#[test]
fn filling_the_budget_drops_the_older_generation() {
    setup();

    // One mask of the letter at 64 px, to size the budget in masks rather
    // than bytes.
    let (font, probe) = keys("M", 0, 1);
    let mut probe_cache = GlyphCache::new();
    probe_cache
        .entry(probe[0], &font, &[])
        .expect("the probe rasterizes");
    let mask_bytes = probe_cache.bytes();
    let mut cache = GlyphCache::with_budget(mask_bytes * 10);

    let (font, keys) = keys("M", 0, 40);
    for key in &keys {
        assert!(cache.entry(*key, &font, &[]).is_some());
    }

    // Forty masks of about ten masks' worth of budget rotated the
    // generations at least three times, so the first mask has been dropped
    // from both.
    assert_eq!(cache.misses(), 40, "every distinct key rasterized once");
    let misses = cache.misses();
    assert!(
        cache.entry(keys[0], &font, &[]).is_some(),
        "a dropped mask rasterizes again"
    );
    assert_eq!(cache.misses(), misses + 1, "the first mask was evicted");

    assert!(
        cache.entry(keys[39], &font, &[]).is_some(),
        "the last mask rasterized"
    );
    assert_eq!(
        cache.misses(),
        misses + 1,
        "a recently rasterized mask is still cached"
    );
    assert!(
        cache.bytes() <= mask_bytes * 10,
        "the current generation stays within the budget"
    );
}

#[test]
fn a_hit_in_the_previous_generation_moves_back_to_the_current_one() {
    setup();

    let (font, probe) = keys("M", 0, 1);
    let mut probe_cache = GlyphCache::new();
    probe_cache
        .entry(probe[0], &font, &[])
        .expect("the probe rasterizes");
    let mask_bytes = probe_cache.bytes();
    let mut cache = GlyphCache::with_budget(mask_bytes * 6);

    let (font, keys) = keys("M", 0, 12);
    for key in &keys[..6] {
        cache.entry(*key, &font, &[]).expect("each mask rasterizes");
    }
    // The cache is full, so this rotation moves the first six masks into the
    // previous generation.
    cache
        .entry(keys[6], &font, &[])
        .expect("the mask rasterizes");

    // Reading the first mask brings it back to the current generation, so the
    // rotation that follows drops the second generation's masks, not this one.
    cache
        .entry(keys[0], &font, &[])
        .expect("the mask is still cached");
    for key in &keys[7..] {
        cache.entry(*key, &font, &[]).expect("each mask rasterizes");
    }

    let misses = cache.misses();
    cache
        .entry(keys[0], &font, &[])
        .expect("the mask is still cached");
    assert_eq!(cache.misses(), misses, "the mask survived the rotation");
}

/// The font, the glyph of the letter W and the variation coordinates of the
/// test variable font at `weight`.
fn variable_w(weight: f32) -> (FontBlob, u32, Vec<i16>) {
    TextSystem::bundled_only().install();
    TextSystem::with(|system| system.register_font(smudge_test::VARIABLE_FONT))
        .expect("the test font is a font");
    let style = TextStyle {
        family: FontFamily::named(smudge_test::VARIABLE_FAMILY),
        weight: FontWeight::new(weight),
        ..TextStyle::default()
    };
    let layout = TextLayout::new("W", &style, None);
    let runs = layout.glyph_runs_at(Point { x: 0.0, y: 64.0 }, Color::BLACK);
    let run = runs.first().expect("the text has a run");
    (
        clone_font_blob(&run.font),
        run.glyphs[0].id,
        run.coords.clone(),
    )
}

/// The key of `glyph` in `font` at 64 physical pixels and the instance
/// numbered `instance`.
fn variable_key(font: &FontBlob, glyph: u32, instance: u32) -> MaskKey {
    MaskKey {
        font_id: font.id,
        font_index: font.index,
        glyph_id: glyph,
        instance,
        size: 64 * 64,
        embolden: 0,
        skew: 0,
        subpixel: 0,
    }
}

/// The mask bytes of `glyph` at `coords`, rasterized by a cache of its own.
fn fresh_mask(font: &FontBlob, glyph: u32, coords: &[i16]) -> Vec<u8> {
    let mut cache = GlyphCache::new();
    let instance = cache.instance(coords);
    cache
        .entry(variable_key(font, glyph, instance), font, coords)
        .expect("the glyph has a mask")
        .data()
        .to_vec()
}

#[test]
fn each_instance_of_a_variable_font_gets_its_own_mask() {
    let (font, glyph, light) = variable_w(300.0);
    let (_, _, heavy) = variable_w(800.0);
    let mut cache = GlyphCache::new();

    let first = cache.instance(&light);
    let second = cache.instance(&heavy);

    let light_mask = cache
        .entry(variable_key(&font, glyph, first), &font, &light)
        .expect("the glyph has a mask")
        .data()
        .to_vec();
    let heavy_mask = cache
        .entry(variable_key(&font, glyph, second), &font, &heavy)
        .expect("the glyph has a mask")
        .data()
        .to_vec();
    assert_ne!(light_mask, heavy_mask);
    assert_eq!(light_mask, fresh_mask(&font, glyph, &light));
    assert_eq!(heavy_mask, fresh_mask(&font, glyph, &heavy));
    assert_eq!(
        fresh_mask(&font, glyph, &vec![0; light.len()]),
        fresh_mask(&font, glyph, &[]),
        "all zero coordinates draw the default instance"
    );
}

#[test]
fn starting_over_past_the_instance_limit_never_serves_a_stale_mask() {
    let (font, glyph, light) = variable_w(300.0);
    let (_, _, heavy) = variable_w(800.0);
    let mut cache = GlyphCache::new();

    let number = cache.instance(&light);
    cache.entry(variable_key(&font, glyph, number), &font, &light);
    let default = variable_key(&font, glyph, cache.instance(&[]));
    cache.entry(default, &font, &[]);
    for step in 1..=MAX_INSTANCES as i16 {
        let coords = [step * 16];
        let number = cache.instance(&coords);
        cache.entry(variable_key(&font, glyph, number), &font, &coords);
    }

    let number = cache.instance(&heavy);
    let mask = cache
        .entry(variable_key(&font, glyph, number), &font, &heavy)
        .expect("the glyph has a mask")
        .data()
        .to_vec();
    assert_eq!(mask, fresh_mask(&font, glyph, &heavy));

    let misses = cache.misses();
    cache.entry(default, &font, &[]);
    assert_eq!(
        cache.misses(),
        misses,
        "the default instance's masks outlive a start-over"
    );
}
