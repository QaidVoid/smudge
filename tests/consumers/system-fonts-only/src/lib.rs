//! The font check for a facade that relies on the machine's fonts alone: a
//! label drawn through the ordinary text system must produce real glyphs.

#[test]
fn a_label_lays_out_real_glyphs() {
    use smudge::core::app::{App, AppConfig, RepaintMode};
    use smudge::core::geometry::{Color, Size};
    use smudge::core::paint::{DisplayItem, PaintOp};
    use smudge::text::label;

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
    assert!(
        glyphs > 0,
        "no system font was found: the label drew no glyphs. The machine needs at least one installed font"
    );
}
