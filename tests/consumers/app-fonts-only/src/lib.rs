//! The font check for a facade whose only font source is the app: once the
//! app registers a font, text in the default style and monospace text must
//! produce real glyphs, though neither names the registered family, and a
//! bold span must draw bold.

#[test]
fn text_draws_in_the_registered_font() {
    use std::rc::Rc;

    use smudge::core::app::{App, AppConfig, RepaintMode};
    use smudge::core::geometry::{Color, Size};
    use smudge::core::paint::{DisplayItem, PaintOp};
    use smudge::prelude::*;
    use smudge::text::{FontWeight, Span, SpanStyle, TextSystem, label, rich_text};

    let families = TextSystem::with(|system| {
        system.register_font(include_bytes!(
            "../../../../crates/smudge-text/assets/fonts/NotoSansMono-Regular.ttf"
        ))
    });
    assert_eq!(families, Ok(vec!["Noto Sans Mono".to_owned()]));

    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 320.0,
                height: 120.0,
            },
            scale: 1.0,
            clear_color: Color::rgb(16, 17, 21),
            repaint: RepaintMode::Damage,
        },
        || {
            column().child(label("Hello, Smudge")).child(rich_text(|| {
                Rc::from([
                    Span::new("plain ", SpanStyle::default()),
                    Span::new(
                        "bold ",
                        SpanStyle {
                            weight: Some(FontWeight::BOLD),
                            ..SpanStyle::default()
                        },
                    ),
                    Span::new(
                        "let x = 1;",
                        SpanStyle {
                            monospace: true,
                            ..SpanStyle::default()
                        },
                    ),
                ])
            }))
        },
    );
    let output = app.frame();

    let mut runs = Vec::new();
    for item in output.list.items() {
        if let DisplayItem::Op(PaintOp::Glyphs(run)) = item {
            assert!(
                run.glyphs.iter().all(|glyph| glyph.id != 0),
                "every glyph comes from the registered font, none is missing"
            );
            runs.push(run.embolden > 0.0);
        }
    }
    assert_eq!(
        runs,
        [false, false, true, false],
        "the label and the plain, bold and monospace spans draw, the bold one \
         emboldened since the registered font has no bold face"
    );
}
