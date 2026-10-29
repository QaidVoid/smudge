//! Tests for the image widget.

use smudge_core::geometry::{Color, PixelRect};
use smudge_core::image::Image;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_widgets::image;

/// A `seed`ed four by four gradient with a translucent band, so the painted
/// pixels exercise sampling and premultiplied blending.
fn gradient(seed: u8) -> Image {
    let mut pixels = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4u32 {
        for x in 0..4u32 {
            let alpha: u32 = if y % 3 == 0 { 128 } else { 255 };
            let premultiply = |channel: u32| ((channel * alpha + 127) / 255) as u8;
            pixels.push(premultiply(u32::from(seed).wrapping_add(x * 40) % 256));
            pixels.push(premultiply(((y * 255) / 3).min(255)));
            pixels.push(90);
            pixels.push(alpha as u8);
        }
    }
    Image::from_premultiplied_rgba(4, 4, pixels).expect("the gradient fills the buffer")
}

#[test]
fn swapping_the_source_repaints_only_the_image() {
    let source = RwSignal::new(None);
    let set = source.clone();
    let mut harness = Harness::new(40.0, 40.0, 1.0, move || {
        panel().child(image(move || set.get()).width(px(4.0)).height(px(4.0)))
    });
    harness.frame();
    harness.settle();

    source.set(Some(gradient(0)));
    let second = harness.frame();
    assert!(!second.full, "a source swap repaints the image's rect only");
    assert_eq!(
        second.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        }],
        "the image measures as its pixel size divided by the scale"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn an_empty_source_paints_nothing() {
    let source = RwSignal::new(None);
    let set = source.clone();
    let mut harness = Harness::new(40.0, 40.0, 1.0, move || {
        panel().child(image(move || set.get()).width(px(4.0)).height(px(4.0)))
    });
    let first = harness.frame();
    assert!(first.full, "the first frame repaints the whole surface");
    let mut blank = Harness::new(40.0, 40.0, 1.0, || {
        panel().child(panel().width(px(4.0)).height(px(4.0)))
    });
    blank.frame();
    harness.assert_matches(&blank);

    let after = harness.frame();
    assert!(
        after.damage.is_empty() && after.nodes_painted == 0,
        "an empty source paints nothing, so an idle frame costs no node"
    );
}

#[test]
fn the_image_fills_the_content_box_inside_a_painted_background_and_border() {
    // An image with padding, a border and a background must paint like a
    // panel with the same box around an image that fills the content box,
    // rounded concentrically: 10 less the 5 pixels of border and padding.
    let picture = gradient(30);
    let mut harness = Harness::new(40.0, 40.0, 1.0, {
        let picture = picture.clone();
        move || {
            panel().align_items(Align::Start).child(
                image(Some(picture))
                    .width(px(26.0))
                    .height(px(22.0))
                    .padding(px(3.0))
                    .border(2.0)
                    .border_color(Color::rgb(20, 40, 200))
                    .background(Color::rgb(200, 180, 20))
                    .corner_radius(10.0),
            )
        }
    });
    harness.frame();
    let mut expected = Harness::new(40.0, 40.0, 1.0, move || {
        panel().align_items(Align::Start).child(
            panel()
                .align_items(Align::Start)
                .padding(px(3.0))
                .border(2.0)
                .border_color(Color::rgb(20, 40, 200))
                .background(Color::rgb(200, 180, 20))
                .corner_radius(10.0)
                .child(
                    image(Some(picture))
                        .width(px(16.0))
                        .height(px(12.0))
                        .corner_radius(5.0),
                ),
        )
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn one_fixed_axis_keeps_the_aspect_ratio() {
    let wide = || {
        Image::from_premultiplied_rgba(40, 20, vec![255; 40 * 20 * 4])
            .expect("the pixels fill the buffer")
    };
    let harness_with = |element: smudge_core::element::Element<smudge_widgets::ImageView>| {
        let mut harness = Harness::new(200.0, 200.0, 1.0, move || {
            panel().align_items(Align::Start).child(element)
        });
        harness.frame();
        harness.layout_dump()
    };
    let dump = harness_with(image(Some(wide())).width(px(100.0)));
    assert!(
        dump.contains("ImageView [0, 0, 100, 50]"),
        "a fixed width derives the height:\n{dump}"
    );
    let dump = harness_with(image(Some(wide())).height(px(30.0)));
    assert!(
        dump.contains("ImageView [0, 0, 60, 30]"),
        "a fixed height derives the width:\n{dump}"
    );
    let dump = harness_with(image(Some(wide())).width(px(100.0)).height(px(10.0)));
    assert!(
        dump.contains("ImageView [0, 0, 100, 10]"),
        "two fixed axes stretch:\n{dump}"
    );
}

#[test]
fn the_image_measures_against_the_window_scale() {
    // A four by four image with no size of its own measures as two logical
    // pixels at scale 2, so one image pixel still lands on one device pixel:
    // the node's rectangle says so, and a swap between two of these images
    // damages exactly those four device pixels.
    let source = RwSignal::new(Some(gradient(0)));
    let set = source.clone();
    let mut harness = Harness::new(40.0, 40.0, 2.0, move || {
        panel()
            .align_items(Align::Start)
            .child(image(move || set.get()))
    });
    harness.frame();
    let dump = harness.layout_dump();
    assert!(
        dump.contains("ImageView [0, 0, 2, 2]"),
        "the image measures two logical pixels at scale 2:\n{dump}"
    );
    source.set(Some(gradient(200)));
    let second = harness.frame();
    assert!(
        !second.full,
        "a swap between same-size images repaints only"
    );
    assert_eq!(
        second.damage,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        }],
        "four image pixels cover four device pixels at scale 2"
    );
    harness.assert_matches_full_repaint();
}
