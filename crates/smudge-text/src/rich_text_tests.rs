use std::rc::Rc;

use parley::FontWeight;
use smudge_core::geometry::Color;
use smudge_core::reactive::Owner;

use super::{Span, SpanChange, SpanStyle, span_change, span_font};
use crate::TextSystem;
use crate::layout::RichLayout;
use crate::style::TextStyle;

/// Two plain spans, the second in `color` when one is given.
fn spans(first: &str, second: &str, color: Option<Color>) -> Vec<Span> {
    vec![
        Span::new(first, SpanStyle::default()),
        Span::new(
            second,
            SpanStyle {
                color: color.map(Into::into),
                ..SpanStyle::default()
            },
        ),
    ]
}

#[test]
fn a_color_only_change_asks_for_paint() {
    let old = spans("Hello ", "world", None);
    let new = spans("Hello ", "world", Some(Color::rgb(200, 40, 40)));
    assert_eq!(span_change(&old, &new), SpanChange::Colors);
}

#[test]
fn a_text_or_font_change_asks_for_layout() {
    let old = spans("Hello ", "world", None);
    assert_eq!(
        span_change(&old, &spans("Hello ", "there", None)),
        SpanChange::Layout
    );
    let mut bold = old.clone();
    bold[1].style.weight = Some(FontWeight::BOLD);
    assert_eq!(span_change(&old, &bold), SpanChange::Layout);
    let mut link = old.clone();
    link[1].style.link = Some(Rc::new(|| {}));
    assert_eq!(span_change(&old, &link), SpanChange::Layout);
    assert_eq!(span_change(&old, &old[..1]), SpanChange::Layout);
}

#[test]
fn a_new_handler_on_a_link_changes_nothing_that_shows() {
    let mut old = spans("Hello ", "world", None);
    old[1].style.link = Some(Rc::new(|| {}));
    let mut new = old.clone();
    new[1].style.link = Some(Rc::new(|| {}));
    assert_eq!(span_change(&old, &new), SpanChange::Nothing);
}

#[test]
fn a_point_over_a_span_maps_to_that_span() {
    let _owner = Owner::new();
    TextSystem::bundled_only().install();
    let spans = spans("Hello ", "world", Some(Color::rgb(200, 40, 40)));
    let fonts: Vec<_> = [0..6, 6..11]
        .into_iter()
        .zip(&spans)
        .map(|(range, span)| (range, span_font(&span.style)))
        .collect();
    let layout = RichLayout::new("Hello world", &TextStyle::default(), &fonts, None);
    let size = layout.size();
    let middle = size.height / 2.0;
    assert_eq!(layout.span_at(2.0, middle), Some(0), "the first letters");
    assert_eq!(
        layout.span_at(size.width - 2.0, middle),
        Some(1),
        "the last letters, which only a color sets apart"
    );
    assert_eq!(
        layout.span_at(size.width + 20.0, middle),
        None,
        "a point beside the text"
    );
}

/// The bundled faces the glyph runs of `layout` paint with, in run order.
#[cfg(feature = "bundled-fonts")]
fn faces(layout: &RichLayout) -> Vec<&'static str> {
    use parley::PositionedLayoutItem;

    let named = [
        (crate::fonts::SANS_REGULAR, "regular"),
        (crate::fonts::SANS_BOLD, "bold"),
        (crate::fonts::SANS_ITALIC, "italic"),
        (crate::fonts::SANS_BOLD_ITALIC, "bold italic"),
        (crate::fonts::MONO_REGULAR, "mono"),
    ];
    layout
        .layout
        .lines()
        .flat_map(|line| line.items().collect::<Vec<_>>())
        .filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => {
                let data: &[u8] = run.run().font().data.as_ref();
                named
                    .iter()
                    .find(|(face, _)| *face == data)
                    .map(|(_, name)| *name)
            }
            PositionedLayoutItem::InlineBox(_) => None,
        })
        .collect()
}

#[cfg(feature = "bundled-fonts")]
#[test]
fn emphasis_draws_with_the_bundled_italic_faces() {
    let _owner = Owner::new();
    TextSystem::bundled_only().install();
    let styles = [
        SpanStyle::default(),
        SpanStyle {
            italic: true,
            ..SpanStyle::default()
        },
        SpanStyle {
            italic: true,
            weight: Some(FontWeight::BOLD),
            ..SpanStyle::default()
        },
        SpanStyle {
            weight: Some(FontWeight::BOLD),
            ..SpanStyle::default()
        },
        SpanStyle {
            monospace: true,
            ..SpanStyle::default()
        },
    ];
    let texts = ["plain ", "emphasis ", "strong ", "bold ", "code"];
    let mut ranges = Vec::new();
    let mut text = String::new();
    for (part, style) in texts.iter().zip(&styles) {
        let start = text.len();
        text.push_str(part);
        ranges.push((start..text.len(), span_font(style)));
    }
    let layout = RichLayout::new(&text, &TextStyle::default(), &ranges, None);
    assert_eq!(
        faces(&layout),
        ["regular", "italic", "bold italic", "bold", "mono"],
        "each span draws with the bundled face of its style"
    );
}
