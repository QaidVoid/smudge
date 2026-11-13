//! The font check for a direct `smudge-widgets` consumer: a label in a frame
//! must produce real glyphs, which proves a font reached the text crate.

#[test]
fn a_label_lays_out_real_glyphs() {
    use smudge_core::app::{App, AppConfig, RepaintMode};
    use smudge_core::geometry::{Color, Size};
    use smudge_core::paint::{DisplayItem, PaintOp};
    use smudge_text::label;

    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 240.0,
                height: 80.0,
            },
            scale: 1.0,
            clear_color: Color::rgb(16, 17, 21),
            repaint: RepaintMode::Damage,
        },
        || label("Hello, Smudge"),
    );
    let output = app.frame();

    let mut glyphs = 0;
    for item in output.list.items() {
        if let DisplayItem::Op(PaintOp::Glyphs(run)) = item {
            glyphs += run.glyphs.iter().filter(|glyph| glyph.id != 0).count();
        }
    }
    assert!(glyphs > 0, "no font was found: the label drew no glyphs");
}
