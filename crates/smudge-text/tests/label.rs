//! Whole-frame behavior of the label widget, driven through the headless
//! harness.
//!
//! Every test installs a bundled-only text system first, so the layout never
//! depends on the fonts installed on the machine running the tests.

use smudge_core::geometry::{Color, Rect};
use smudge_core::reactive::{prelude::*, signal_local};
use smudge_core::style::{Align, px};
use smudge_core::theme::{ColorToken, Theme};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{LabelExt, TextStyle, TextSystem, label, provide_text_style};

/// The absolute rectangles of every visible node, in paint order.
fn node_rects(harness: &mut Harness) -> Vec<Rect> {
    let mut rects = Vec::new();
    harness
        .app()
        .visit_nodes(|_depth, _name, rect| rects.push(rect));
    rects
}

#[test]
fn text_color_change_repaints_without_layout() {
    TextSystem::bundled_only().install();
    let (color, set_color) = signal_local(Color::BLACK);
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || {
        label("Hello").text_color(move || color.get().into())
    });
    harness.frame();

    set_color.set(Color::rgb(180, 30, 30));
    let report = harness.frame();

    assert!(
        !report.layout_ran,
        "a color change must not run the layout pass"
    );
    assert_eq!(report.nodes_painted, 1, "only the label repaints");
    harness.assert_matches_full_repaint();
}

#[test]
fn longer_text_relays_out_and_moves_following_sibling() {
    TextSystem::bundled_only().install();
    let (text, set_text) = signal_local("Hi".to_owned());
    let mut harness = Harness::new(320.0, 100.0, 1.0, move || {
        panel().child(label(move || text.get())).child(
            panel()
                .width(px(40.0))
                .height(px(20.0))
                .background(Color::BLACK),
        )
    });
    harness.frame();
    let before = node_rects(&mut harness);

    set_text.set("Hello, longer world".to_owned());
    let report = harness.frame();
    let after = node_rects(&mut harness);

    assert!(report.layout_ran, "a longer text must run the layout pass");
    assert!(
        after[1].width() > before[1].width(),
        "the label grows wider, found {} then {}",
        before[1].width(),
        after[1].width()
    );
    assert_eq!(
        after[2].x0, after[1].x1,
        "the sibling starts where the label ends"
    );
    assert!(
        after[2].x0 > before[2].x0,
        "the sibling moves right, found {} then {}",
        before[2].x0,
        after[2].x0
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn equal_text_write_changes_nothing() {
    TextSystem::bundled_only().install();
    let (text, set_text) = signal_local("Same".to_owned());
    let mut harness = Harness::new(200.0, 100.0, 1.0, move || label(move || text.get()));
    harness.frame();

    set_text.set("Same".to_owned());
    let report = harness.frame();

    assert!(
        !report.layout_ran,
        "an equal text must not run the layout pass"
    );
    assert_eq!(report.nodes_painted, 0, "an equal text repaints nothing");
    assert!(report.damage.is_empty(), "an equal text damages nothing");
}

#[test]
fn label_wraps_in_narrow_column_and_grows_taller() {
    TextSystem::bundled_only().install();
    let long = "the quick brown fox jumps over the lazy dog again and again";
    // The root is sized to the viewport, so the narrow column is a child of it.
    let (width, set_width) = signal_local(100.0);
    let mut narrow = Harness::new(320.0, 240.0, 1.0, move || {
        panel().child(
            panel()
                .column()
                .width(move || px(width.get()))
                .child(label(long)),
        )
    });
    let mut wide = Harness::new(320.0, 240.0, 1.0, || {
        panel().child(panel().column().child(label("Hi")))
    });
    narrow.frame();
    wide.frame();

    let narrow_rects = node_rects(&mut narrow);
    let wide_rects = node_rects(&mut wide);
    let one_line = wide_rects[2].height();
    let wrapped = narrow_rects[2].height();

    assert_eq!(
        narrow_rects[2].width(),
        100.0,
        "the column stretches the label to its own width"
    );
    assert!(
        wrapped >= 2.0 * one_line,
        "the text wraps onto several lines, found height {wrapped} against \
         one line of height {one_line}"
    );

    set_width.set(60.0);
    narrow.frame();
    let tighter = node_rects(&mut narrow)[2].height();

    assert!(
        tighter > wrapped,
        "a narrower column wraps onto more lines, found height {tighter} \
         against {wrapped}"
    );
    narrow.assert_matches_full_repaint();
}

#[test]
fn label_repaints_at_its_new_position_when_a_sibling_grows() {
    TextSystem::bundled_only().install();
    let (width, set_width) = signal_local(20.0);
    let mut harness = Harness::new(320.0, 100.0, 1.0, move || {
        panel()
            .child(
                panel()
                    .width(move || px(width.get()))
                    .height(px(20.0))
                    .background(Color::BLACK),
            )
            .child(label("moved"))
    });
    harness.frame();

    set_width.set(80.0);
    harness.frame();

    let rects = node_rects(&mut harness);
    assert_eq!(rects[2].x0, 80.0, "the label sits after the grown sibling");
    harness.assert_matches_full_repaint();
}

#[test]
fn label_uses_the_ambient_style_and_explicit_fields_win() {
    TextSystem::bundled_only().install();
    let ambient = TextStyle {
        size: 30.0,
        ..TextStyle::default()
    };
    let mut big = Harness::new(320.0, 240.0, 1.0, {
        let ambient = ambient.clone();
        move || {
            provide_text_style(ambient);
            // A column stretches the label across, but its height stays the
            // height of its text.
            panel().column().child(label("Hi"))
        }
    });
    let mut small = Harness::new(320.0, 240.0, 1.0, move || {
        provide_text_style(ambient);
        panel().column().child(label("Hi").text_size(10.0))
    });
    let mut plain = Harness::new(320.0, 240.0, 1.0, || panel().column().child(label("Hi")));
    big.frame();
    small.frame();
    plain.frame();

    let big_height = node_rects(&mut big)[1].height();
    let small_height = node_rects(&mut small)[1].height();
    let plain_height = node_rects(&mut plain)[1].height();

    assert!(
        big_height > plain_height,
        "the ambient size applies when no builder sets one, found {big_height} \
         against {plain_height}"
    );
    assert!(
        small_height < plain_height,
        "an explicit size wins over the ambient one, found {small_height} \
         against {plain_height}"
    );
}

#[test]
fn a_label_in_a_row_too_narrow_for_it_wraps_and_leaves_its_sibling_room() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 200.0, 1.0, || {
        panel().align_items(Align::Start).child(
            panel()
                .row()
                .width(px(120.0))
                .child(panel().width(px(30.0)).height(px(10.0)))
                .child(label("several words of text here")),
        )
    });
    harness.frame();

    let rects = node_rects(&mut harness);
    let (row, sibling, text) = (rects[1], rects[2], rects[3]);
    assert!(
        text.x1 <= row.x1,
        "the label stays inside its row, not {text:?} in {row:?}"
    );
    assert!(
        text.height() > 20.0,
        "the label wraps onto more than one line, not {text:?}"
    );
    assert!(
        sibling.width() > 0.0,
        "the sibling keeps part of its width, not {sibling:?}"
    );
}

#[test]
fn a_theme_switch_repaints_a_label_whose_text_color_is_a_token() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, || {
        label("Hello").text_color(ColorToken::TEXT)
    });
    harness.frame();

    harness.app().set_theme(Theme::dark());
    let report = harness.frame();

    assert_eq!(
        report.nodes_painted, 1,
        "the token-using label repaints on a theme switch"
    );
    assert!(!report.layout_ran, "a color change never lays out again");

    let mut expected = Harness::new(200.0, 100.0, 1.0, || {
        label("Hello").text_color(Theme::dark().color(ColorToken::TEXT))
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn a_default_label_paints_with_the_theme_s_text_color() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, || label("Hello"));
    harness.app().set_theme(Theme::dark());
    harness.frame();

    let mut expected = Harness::new(200.0, 100.0, 1.0, || {
        label("Hello").text_color(Theme::dark().color(ColorToken::TEXT))
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn a_theme_switch_repaints_a_default_label_and_damages_it() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, || label("Hello"));
    harness.frame();

    harness.app().set_theme(Theme::dark());
    let report = harness.frame();

    assert_eq!(
        report.nodes_painted, 1,
        "the default label follows the theme's text token, so it repaints"
    );
    assert!(!report.layout_ran, "a color change never lays out again");
    assert!(
        !report.damage.is_empty(),
        "the switch damages the label's area"
    );

    let mut expected = Harness::new(200.0, 100.0, 1.0, || {
        label("Hello").text_color(Theme::dark().color(ColorToken::TEXT))
    });
    expected.frame();
    harness.assert_matches(&expected);
}

#[test]
fn a_label_with_a_literal_text_color_ignores_a_theme_switch() {
    TextSystem::bundled_only().install();
    const LITERAL: Color = Color::rgb(10, 20, 30);
    let mut harness = Harness::new(200.0, 100.0, 1.0, || label("Hello").text_color(LITERAL));
    harness.frame();

    harness.app().set_theme(Theme::dark());
    let report = harness.frame();

    assert_eq!(
        report.nodes_painted, 0,
        "a literal color consults no token, so nothing repaints"
    );
    assert!(
        report.damage.is_empty(),
        "a theme switch that the label ignores damages nothing"
    );

    let mut expected = Harness::new(200.0, 100.0, 1.0, || label("Hello").text_color(LITERAL));
    expected.frame();
    harness.assert_matches(&expected);
}
